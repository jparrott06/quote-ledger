//! Integration: append CreateQuote then subscribe sees snapshot at head.

use std::time::Duration;

use quote_ledger::v1::quote_command::Kind as CmdKind;
use quote_ledger::v1::quote_ledger_service_client::QuoteLedgerServiceClient;
use quote_ledger::v1::quote_update::Kind as UpdateKind;
use quote_ledger::v1::{AppendCommandRequest, CreateQuote, QuoteCommand, SubscribeQuoteRequest};
use quote_ledger::{grpc_server, sqlite, LedgerService};
use tokio::net::TcpListener;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::transport::Server;

#[tokio::test]
async fn append_then_subscribe_snapshot() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("ledger.db");
    let conn = sqlite::open_and_migrate(db.to_str().expect("utf8 path")).expect("migrate");

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let incoming = TcpListenerStream::new(listener);

    let svc = grpc_server(LedgerService::new(conn));

    tokio::spawn(async move {
        Server::builder()
            .add_service(svc)
            .serve_with_incoming(incoming)
            .await
            .expect("server");
    });

    tokio::time::sleep(Duration::from_millis(150)).await;

    let mut client = QuoteLedgerServiceClient::connect(format!("http://{addr}"))
        .await
        .expect("connect");

    let quote_id = "q-append-1".to_string();

    let cmd = QuoteCommand {
        kind: Some(CmdKind::CreateQuote(CreateQuote {
            currency_code: "USD".into(),
            jurisdiction_id: "US-CA".into(),
        })),
    };

    let (tx, rx) = tokio::sync::mpsc::channel::<AppendCommandRequest>(4);
    tx.send(AppendCommandRequest {
        client_command_id: "cc-1".into(),
        quote_id: quote_id.clone(),
        command: Some(cmd),
    })
    .await
    .expect("send");
    drop(tx);

    let append_resp = client
        .append_commands(ReceiverStream::new(rx))
        .await
        .expect("append ok")
        .into_inner();

    assert_eq!(append_resp.last_committed_seq, 1);
    assert_eq!(append_resp.committed.len(), 1);

    let (tx2, rx2) = tokio::sync::mpsc::channel::<AppendCommandRequest>(4);
    tx2.send(AppendCommandRequest {
        client_command_id: "cc-1".into(),
        quote_id: quote_id.clone(),
        command: Some(QuoteCommand {
            kind: Some(CmdKind::CreateQuote(CreateQuote {
                currency_code: "USD".into(),
                jurisdiction_id: "US-CA".into(),
            })),
        }),
    })
    .await
    .expect("send");
    drop(tx2);

    let idempotent = client
        .append_commands(ReceiverStream::new(rx2))
        .await
        .expect("idempotent append ok")
        .into_inner();

    assert_eq!(idempotent.last_committed_seq, 1);
    assert_eq!(idempotent.committed.len(), 1);

    let mut sub = client
        .subscribe_quote(SubscribeQuoteRequest {
            quote_id: quote_id.clone(),
            after_seq: 0,
        })
        .await
        .expect("subscribe ok")
        .into_inner();

    let mut saw_snapshot = false;
    while let Some(msg) = sub.message().await.expect("stream") {
        match msg.kind.expect("kind") {
            UpdateKind::Tail(t) => {
                assert_eq!(t.from_seq_exclusive, 0);
                assert_eq!(t.to_seq_inclusive, 1);
                assert_eq!(t.events.len(), 1);
            }
            UpdateKind::Snapshot(s) => {
                assert_eq!(s.last_seq, 1);
                let v = s.view.expect("view");
                assert_eq!(v.quote_id, quote_id);
                assert_eq!(v.currency_code, "USD");
                assert_eq!(v.jurisdiction_id, "US-CA");
                saw_snapshot = true;
                break;
            }
        }
    }

    assert!(saw_snapshot, "expected a snapshot frame");
}
