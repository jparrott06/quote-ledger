use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::mpsc;
use tokio::sync::watch;
use tokio::sync::Mutex as TokioMutex;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::Stream;
use tonic::codec::Streaming;
use tonic::{Request, Response, Status};
use tracing::instrument;

use crate::domain;
use crate::mapping::{
    domain_error_to_status, domain_event_to_proto, proto_command_to_domain, quote_state_to_view,
    replay_stored_to_state, store_error_to_status,
};
use crate::store;
use crate::v1::quote_ledger_service_server::{QuoteLedgerService, QuoteLedgerServiceServer};
use crate::v1::{
    quote_update, AppendCommandRequest, AppendCommandsResponse, QuoteCommand, QuoteSnapshot,
    QuoteTail, QuoteUpdate, StoredEvent, SubscribeQuoteRequest,
};

pub struct LedgerInner {
    conn: Arc<std::sync::Mutex<rusqlite::Connection>>,
    bus: Arc<TokioMutex<HashMap<String, watch::Sender<u64>>>>,
}

impl LedgerInner {
    pub fn new(conn: rusqlite::Connection) -> Self {
        Self {
            conn: Arc::new(std::sync::Mutex::new(conn)),
            bus: Arc::new(TokioMutex::new(HashMap::new())),
        }
    }

    pub async fn db<T, F>(&self, f: F) -> Result<T, Status>
    where
        F: FnOnce(&mut rusqlite::Connection) -> Result<T, crate::error::StoreError>
            + Send
            + 'static,
        T: Send + 'static,
    {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let mut g = conn.lock().expect("db mutex poisoned");
            f(&mut g)
        })
        .await
        .map_err(|e| Status::internal(format!("blocking join: {e}")))?
        .map_err(store_error_to_status)
    }

    pub async fn publish_head(&self, quote_id: &str, head: u64) {
        let mut m = self.bus.lock().await;
        let tx = m
            .entry(quote_id.to_string())
            .or_insert_with(|| watch::channel(head).0);
        let _ = tx.send_replace(head);
    }

    #[instrument(skip(self, cmd), fields(quote_id = %quote_id, client_command_id = %client_command_id))]
    pub async fn append_one(
        &self,
        quote_id: &str,
        client_command_id: &str,
        cmd: &QuoteCommand,
    ) -> Result<Vec<StoredEvent>, Status> {
        let quote_id_owned = quote_id.to_string();
        let client_owned = client_command_id.to_string();

        if let Some(committed) = self
            .db({
                let q = quote_id_owned.clone();
                let c = client_owned.clone();
                move |conn| store::try_load_idempotent_events(conn, &q, &c)
            })
            .await?
        {
            return Ok(committed);
        }

        let domain_cmd = proto_command_to_domain(cmd)?;

        let all = self
            .db({
                let q = quote_id_owned.clone();
                move |c| store::load_stored_events(c, &q)
            })
            .await?;

        let state = replay_stored_to_state(&all).map_err(store_error_to_status)?;
        let events =
            domain::command_to_events(&state, &domain_cmd).map_err(domain_error_to_status)?;
        let proto_events: Vec<crate::v1::QuoteEvent> =
            events.iter().map(domain_event_to_proto).collect();

        let committed = self
            .db(move |c| {
                store::append_command_events(c, &quote_id_owned, &client_owned, &proto_events)
            })
            .await?;

        if let Some(last) = committed.last() {
            self.publish_head(quote_id, last.seq).await;
        }

        Ok(committed)
    }
}

pub struct LedgerService {
    pub(crate) inner: Arc<LedgerInner>,
}

impl LedgerService {
    pub fn new(conn: rusqlite::Connection) -> Self {
        Self {
            inner: Arc::new(LedgerInner::new(conn)),
        }
    }
}

#[tonic::async_trait]
impl QuoteLedgerService for LedgerService {
    async fn append_commands(
        &self,
        request: Request<Streaming<AppendCommandRequest>>,
    ) -> Result<Response<AppendCommandsResponse>, Status> {
        let mut stream = request.into_inner();
        let mut quote_id: Option<String> = None;
        let mut last_committed_seq: u64 = 0;
        let mut all_committed: Vec<StoredEvent> = Vec::new();

        loop {
            let msg = match stream.message().await {
                Ok(Some(m)) => m,
                Ok(None) => break,
                Err(e) => return Err(e),
            };

            if msg.client_command_id.is_empty() || msg.quote_id.is_empty() {
                return Err(Status::invalid_argument(
                    "client_command_id and quote_id are required",
                ));
            }

            match &quote_id {
                None => quote_id = Some(msg.quote_id.clone()),
                Some(q) if q != &msg.quote_id => {
                    return Err(Status::invalid_argument(
                        "quote_id must remain consistent within AppendCommands",
                    ));
                }
                _ => {}
            }

            let qid = quote_id.as_ref().unwrap();
            let cmd = msg
                .command
                .as_ref()
                .ok_or_else(|| Status::invalid_argument("command required"))?;

            let stored = self
                .inner
                .append_one(qid, &msg.client_command_id, cmd)
                .await?;

            if let Some(tail) = stored.last() {
                last_committed_seq = tail.seq;
            }
            all_committed.extend(stored);
        }

        if quote_id.is_none() {
            return Err(Status::invalid_argument("append_commands stream was empty"));
        }

        Ok(Response::new(AppendCommandsResponse {
            last_committed_seq,
            committed: all_committed,
        }))
    }

    type SubscribeQuoteStream =
        Pin<Box<dyn Stream<Item = Result<QuoteUpdate, Status>> + Send + 'static>>;

    async fn subscribe_quote(
        &self,
        request: Request<SubscribeQuoteRequest>,
    ) -> Result<Response<Self::SubscribeQuoteStream>, Status> {
        let req = request.into_inner();
        if req.quote_id.is_empty() {
            return Err(Status::invalid_argument("quote_id is required"));
        }

        let inner = self.inner.clone();
        let qid = req.quote_id.clone();
        let after = req.after_seq;

        struct Init {
            last_seq: u64,
            view: crate::v1::QuoteView,
            catchup: Vec<StoredEvent>,
        }

        let init = inner
            .db({
                let quote_id = qid.clone();
                move |c| {
                    let all = store::load_stored_events(c, &quote_id)?;
                    let last = all.last().map(|e| e.seq).unwrap_or(0);
                    if after > last {
                        return Err(crate::error::StoreError::InvalidAfterSeq {
                            after_seq: after,
                            last_seq: last,
                        });
                    }

                    let state = replay_stored_to_state(&all)?;
                    let view = quote_state_to_view(&quote_id, &state);
                    let catchup = if after < last {
                        store::load_stored_events_between(c, &quote_id, after, last)?
                    } else {
                        Vec::new()
                    };

                    Ok(Init {
                        last_seq: last,
                        view,
                        catchup,
                    })
                }
            })
            .await?;

        let Init {
            last_seq,
            view,
            catchup,
        } = init;

        let mut rx_watch = {
            let mut m = inner.bus.lock().await;
            let tx = m
                .entry(qid.clone())
                .or_insert_with(|| watch::channel(last_seq).0);
            let _ = tx.send_replace(last_seq);
            tx.subscribe()
        };

        let (tx, rx) = mpsc::channel::<Result<QuoteUpdate, Status>>(32);

        let inner2 = inner.clone();
        let qid2 = qid.clone();
        tokio::spawn(async move {
            async fn push_update(
                tx: &mpsc::Sender<Result<QuoteUpdate, Status>>,
                update: QuoteUpdate,
            ) -> bool {
                tx.send(Ok(update)).await.is_ok()
            }

            if !catchup.is_empty() {
                let from = after;
                let to = catchup.last().map(|e| e.seq).unwrap_or(from);
                let tail = QuoteUpdate {
                    kind: Some(quote_update::Kind::Tail(QuoteTail {
                        from_seq_exclusive: from,
                        to_seq_inclusive: to,
                        events: catchup,
                    })),
                };
                if !push_update(&tx, tail).await {
                    return;
                }
            }

            let snap = QuoteUpdate {
                kind: Some(quote_update::Kind::Snapshot(QuoteSnapshot {
                    last_seq,
                    view: Some(view),
                })),
            };
            if !push_update(&tx, snap).await {
                return;
            }

            let mut watermark = last_seq;

            let head_now = *rx_watch.borrow();
            if head_now > watermark {
                match inner2
                    .db({
                        let q = qid2.clone();
                        move |c| store::load_stored_events_between(c, &q, watermark, head_now)
                    })
                    .await
                {
                    Ok(chunk) if !chunk.is_empty() => {
                        let from = watermark;
                        let to = chunk.last().map(|e| e.seq).unwrap_or(from);
                        let tail = QuoteUpdate {
                            kind: Some(quote_update::Kind::Tail(QuoteTail {
                                from_seq_exclusive: from,
                                to_seq_inclusive: to,
                                events: chunk,
                            })),
                        };
                        if !push_update(&tx, tail).await {
                            return;
                        }
                        watermark = head_now;
                    }
                    _ => watermark = head_now,
                }
            }

            loop {
                if rx_watch.changed().await.is_err() {
                    break;
                }
                let head = *rx_watch.borrow();
                if head <= watermark {
                    continue;
                }

                let chunk = match inner2
                    .db({
                        let q = qid2.clone();
                        let wm = watermark;
                        move |c| store::load_stored_events_between(c, &q, wm, head)
                    })
                    .await
                {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                if chunk.is_empty() {
                    watermark = head;
                    continue;
                }

                let from = watermark;
                let to = chunk.last().map(|e| e.seq).unwrap_or(from);
                let tail = QuoteUpdate {
                    kind: Some(quote_update::Kind::Tail(QuoteTail {
                        from_seq_exclusive: from,
                        to_seq_inclusive: to,
                        events: chunk,
                    })),
                };

                if !push_update(&tx, tail).await {
                    break;
                }

                watermark = head;
            }
        });

        Ok(Response::new(Box::pin(ReceiverStream::new(rx))
            as Pin<
                Box<dyn Stream<Item = Result<QuoteUpdate, Status>> + Send + 'static>,
            >))
    }
}

pub fn grpc_server(service: LedgerService) -> QuoteLedgerServiceServer<LedgerService> {
    QuoteLedgerServiceServer::new(service)
}
