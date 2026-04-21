//! PR smoke: SubscribeQuote returns a streaming frame (snapshot on empty quote).

use std::time::Duration;

use quote_ledger::v1::quote_ledger_service_client::QuoteLedgerServiceClient;
use quote_ledger::v1::quote_update::Kind as UpdateKind;
use quote_ledger::v1::SubscribeQuoteRequest;
use quote_ledger::{grpc_server, sqlite, LedgerService};
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::transport::Server;

#[tokio::test]
async fn subscribe_quote_empty_snapshot_smoke() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("smoke.db");
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

    let mut stream = client
        .subscribe_quote(SubscribeQuoteRequest {
            quote_id: "smoke-quote".into(),
            after_seq: 0,
        })
        .await
        .expect("subscribe ok")
        .into_inner();

    let first = stream
        .message()
        .await
        .expect("stream ok")
        .expect("first frame");

    match first.kind.expect("kind") {
        UpdateKind::Snapshot(s) => {
            assert_eq!(s.last_seq, 0);
            let v = s.view.expect("view");
            assert_eq!(v.quote_id, "smoke-quote");
            assert_eq!(v.currency_code, "");
            assert_eq!(v.jurisdiction_id, "");
            assert!(!v.finalized);
        }
        UpdateKind::Tail(_) => panic!("unexpected tail on empty quote"),
    }
}
