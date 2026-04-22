use std::collections::HashMap;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use metrics::{counter, gauge, histogram};
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
    domain_event_to_proto, proto_command_to_domain, quote_state_to_view, replay_stored_to_state,
    store_error_to_status,
};
use crate::store;
use crate::v1::quote_ledger_service_server::{QuoteLedgerService, QuoteLedgerServiceServer};
use crate::v1::{
    quote_update, AppendCommandRequest, AppendCommandsResponse, QuoteCommand, QuoteSnapshot,
    QuoteTail, QuoteUpdate, StoredEvent, SubscribeQuoteRequest,
};

const MAX_ID_LEN: usize = 128;
const DEFAULT_APPEND_IDLE_TIMEOUT_MS: u64 = 10_000;
const DEFAULT_APPEND_MAX_COMMANDS: usize = 512;

/// Bounded retries for SQLite reads on the subscribe pump. Transient failures must not skip seqs.
const SUBSCRIBE_DB_MAX_ATTEMPTS: u32 = 8;
const SUBSCRIBE_DB_BACKOFF_START_MS: u64 = 10;
const SUBSCRIBE_DB_BACKOFF_CAP_MS: u64 = 500;

async fn subscribe_load_between(
    inner: &LedgerInner,
    quote_id: &str,
    after_exclusive: u64,
    upto_inclusive: u64,
) -> Result<Vec<StoredEvent>, Status> {
    let mut backoff_ms = SUBSCRIBE_DB_BACKOFF_START_MS;
    let mut last_err: Option<Status> = None;

    for attempt in 0..SUBSCRIBE_DB_MAX_ATTEMPTS {
        if attempt > 0 {
            counter!("quote_ledger_subscribe_db_retries_total").increment(1);
            tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
            backoff_ms = (backoff_ms.saturating_mul(2)).min(SUBSCRIBE_DB_BACKOFF_CAP_MS);
        }

        match inner
            .db({
                let q = quote_id.to_string();
                move |c| store::load_stored_events_between(c, &q, after_exclusive, upto_inclusive)
            })
            .await
        {
            Ok(events) => return Ok(events),
            Err(e) => {
                counter!("quote_ledger_subscribe_db_load_failures_total").increment(1);
                last_err = Some(e);
            }
        }
    }

    counter!(
        "quote_ledger_subscribe_stream_terminal_errors_total",
        "cause" => "db_load_between"
    )
    .increment(1);
    Err(last_err.unwrap_or_else(|| Status::internal("subscribe db load failed")))
}

fn validate_identifier(field: &'static str, value: &str) -> Result<(), Status> {
    if value.trim().is_empty() {
        return Err(Status::invalid_argument(format!("{field} is required")));
    }
    if value.len() > MAX_ID_LEN {
        return Err(Status::invalid_argument(format!(
            "{field} exceeds {MAX_ID_LEN} characters"
        )));
    }
    if value != value.trim() {
        return Err(Status::invalid_argument(format!(
            "{field} must not contain leading/trailing spaces"
        )));
    }
    let valid = value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'));
    if !valid {
        return Err(Status::invalid_argument(format!(
            "{field} contains invalid characters"
        )));
    }
    Ok(())
}

struct InFlightGuard {
    counter: Arc<AtomicUsize>,
}

impl InFlightGuard {
    fn enter(counter: &Arc<AtomicUsize>) -> Self {
        let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
        gauge!("quote_ledger_in_flight_appends").set(n as f64);
        Self {
            counter: counter.clone(),
        }
    }
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        let prev = self.counter.fetch_sub(1, Ordering::SeqCst);
        let n = prev.saturating_sub(1);
        gauge!("quote_ledger_in_flight_appends").set(n as f64);
    }
}

struct AppendOneLatency(Instant);

impl AppendOneLatency {
    fn start() -> Self {
        Self(Instant::now())
    }
}

impl Drop for AppendOneLatency {
    fn drop(&mut self) {
        histogram!("quote_ledger_append_one_duration_seconds")
            .record(self.0.elapsed().as_secs_f64());
    }
}

#[derive(Clone, Debug)]
pub struct ReliabilityLimits {
    pub append_idle_timeout: Duration,
    pub append_max_commands: usize,
}

impl Default for ReliabilityLimits {
    fn default() -> Self {
        Self {
            append_idle_timeout: Duration::from_millis(DEFAULT_APPEND_IDLE_TIMEOUT_MS),
            append_max_commands: DEFAULT_APPEND_MAX_COMMANDS,
        }
    }
}

pub struct LedgerInner {
    conn: Arc<std::sync::Mutex<rusqlite::Connection>>,
    bus: Arc<TokioMutex<HashMap<String, watch::Sender<u64>>>>,
    pub in_flight_appends: Arc<AtomicUsize>,
}

impl LedgerInner {
    pub fn new(conn: rusqlite::Connection) -> Self {
        Self {
            conn: Arc::new(std::sync::Mutex::new(conn)),
            bus: Arc::new(TokioMutex::new(HashMap::new())),
            in_flight_appends: Arc::new(AtomicUsize::new(0)),
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
        let _in_flight = InFlightGuard::enter(&self.in_flight_appends);
        let _latency = AppendOneLatency::start();

        let quote_id_owned = quote_id.to_string();
        let client_owned = client_command_id.to_string();

        let domain_cmd = proto_command_to_domain(cmd)?;

        let committed = self
            .db({
                let quote_id_owned = quote_id_owned.clone();
                let client_owned = client_owned.clone();
                let domain_cmd = domain_cmd.clone();
                move |c| {
                    if let Some((first_seq, _last_seq)) =
                        store::idempotency_lookup(c, &quote_id_owned, &client_owned)?
                    {
                        let before = store::load_stored_events_between(
                            c,
                            &quote_id_owned,
                            0,
                            first_seq.saturating_sub(1),
                        )?;
                        let state_before = replay_stored_to_state(&before)?;
                        let events = domain::command_to_events(&state_before, &domain_cmd)?;
                        let proto_events: Vec<crate::v1::QuoteEvent> =
                            events.iter().map(domain_event_to_proto).collect();
                        return store::append_command_events(
                            c,
                            &quote_id_owned,
                            &client_owned,
                            &proto_events,
                        );
                    }

                    let all = store::load_stored_events(c, &quote_id_owned)?;
                    let state = replay_stored_to_state(&all)?;
                    let events = domain::command_to_events(&state, &domain_cmd)?;
                    let proto_events: Vec<crate::v1::QuoteEvent> =
                        events.iter().map(domain_event_to_proto).collect();
                    store::append_command_events(c, &quote_id_owned, &client_owned, &proto_events)
                }
            })
            .await?;

        if let Some(last) = committed.last() {
            self.publish_head(quote_id, last.seq).await;
        }

        counter!("quote_ledger_append_one_commits_total").increment(1);
        Ok(committed)
    }
}

pub struct LedgerService {
    pub(crate) inner: Arc<LedgerInner>,
    reliability: ReliabilityLimits,
}

impl LedgerService {
    pub fn new(conn: rusqlite::Connection) -> Self {
        Self::with_reliability(conn, ReliabilityLimits::default())
    }

    pub fn with_reliability(conn: rusqlite::Connection, reliability: ReliabilityLimits) -> Self {
        Self {
            inner: Arc::new(LedgerInner::new(conn)),
            reliability,
        }
    }

    pub fn in_flight_appends(&self) -> usize {
        self.inner.in_flight_appends.load(Ordering::SeqCst)
    }

    pub fn in_flight_counter(&self) -> Arc<AtomicUsize> {
        self.inner.in_flight_appends.clone()
    }
}

#[tonic::async_trait]
impl QuoteLedgerService for LedgerService {
    async fn append_commands(
        &self,
        request: Request<Streaming<AppendCommandRequest>>,
    ) -> Result<Response<AppendCommandsResponse>, Status> {
        let stream_started = Instant::now();
        let mut stream = request.into_inner();
        let mut quote_id: Option<String> = None;
        let mut last_committed_seq: u64 = 0;
        let mut all_committed: Vec<StoredEvent> = Vec::new();
        let mut command_count: usize = 0;

        loop {
            let msg = match tokio::time::timeout(
                self.reliability.append_idle_timeout,
                stream.message(),
            )
            .await
            {
                Ok(Ok(Some(m))) => m,
                Ok(Ok(None)) => break,
                Ok(Err(e)) => {
                    counter!("quote_ledger_append_commands_streams_total", "result" => "transport_error")
                        .increment(1);
                    return Err(e);
                }
                Err(_) => {
                    counter!("quote_ledger_append_commands_streams_total", "result" => "timeout")
                        .increment(1);
                    return Err(Status::deadline_exceeded(
                        "append_commands stream idle timeout",
                    ));
                }
            };

            command_count += 1;
            if command_count > self.reliability.append_max_commands {
                counter!("quote_ledger_append_commands_streams_total", "result" => "too_many")
                    .increment(1);
                return Err(Status::resource_exhausted(format!(
                    "append_commands stream exceeds max {} commands",
                    self.reliability.append_max_commands
                )));
            }

            validate_identifier("client_command_id", &msg.client_command_id)?;
            validate_identifier("quote_id", &msg.quote_id)?;

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
            counter!("quote_ledger_append_commands_streams_total", "result" => "invalid")
                .increment(1);
            return Err(Status::invalid_argument("append_commands stream was empty"));
        }

        counter!("quote_ledger_append_commands_streams_total", "result" => "ok").increment(1);
        histogram!("quote_ledger_append_commands_stream_duration_seconds")
            .record(stream_started.elapsed().as_secs_f64());
        Ok(Response::new(AppendCommandsResponse {
            last_committed_seq,
            committed: all_committed,
        }))
    }

    type SubscribeQuoteStream =
        Pin<Box<dyn Stream<Item = Result<QuoteUpdate, Status>> + Send + 'static>>;

    #[instrument(skip(self, request))]
    async fn subscribe_quote(
        &self,
        request: Request<SubscribeQuoteRequest>,
    ) -> Result<Response<Self::SubscribeQuoteStream>, Status> {
        let req = request.into_inner();
        validate_identifier("quote_id", &req.quote_id)?;

        counter!("quote_ledger_subscribe_streams_total").increment(1);
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
                    let view = quote_state_to_view(&quote_id, &state)?;
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
                match subscribe_load_between(&inner2, &qid2, watermark, head_now).await {
                    Ok(chunk) => {
                        if chunk.is_empty() {
                            watermark = head_now;
                        } else {
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
                    }
                    Err(status) => {
                        let _ = tx.send(Err(status)).await;
                        return;
                    }
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

                let chunk = match subscribe_load_between(&inner2, &qid2, watermark, head).await {
                    Ok(c) => c,
                    Err(status) => {
                        let _ = tx.send(Err(status)).await;
                        break;
                    }
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

#[cfg(test)]
mod subscribe_load_tests {
    use super::*;

    #[tokio::test]
    async fn subscribe_load_between_empty_range() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = dir.path().join("sub.db");
        let conn = crate::sqlite::open_and_migrate(db.to_str().expect("utf8")).expect("migrate");
        let inner = LedgerInner::new(conn);
        let out = subscribe_load_between(&inner, "no-such-quote", 0, 0)
            .await
            .expect("ok");
        assert!(out.is_empty());
    }
}
