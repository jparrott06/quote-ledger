//! Longer soak-style concurrency test (ignored in default CI).

use std::time::Duration;

use quote_ledger::v1::quote_command::Kind as CmdKind;
use quote_ledger::v1::quote_ledger_service_client::QuoteLedgerServiceClient;
use quote_ledger::v1::quote_update::Kind as UpdateKind;
use quote_ledger::v1::{
    AddLineItem, AppendCommandRequest, CreateQuote, QuoteCommand, SubscribeQuoteRequest,
};
use quote_ledger::{grpc_server, sqlite, LedgerService};
use tokio::net::TcpListener;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::transport::Channel;
use tonic::transport::Server;

async fn append_one(
    client: &mut QuoteLedgerServiceClient<Channel>,
    quote_id: &str,
    client_command_id: &str,
    cmd: QuoteCommand,
) {
    let (tx, rx) = tokio::sync::mpsc::channel::<AppendCommandRequest>(4);
    tx.send(AppendCommandRequest {
        client_command_id: client_command_id.into(),
        quote_id: quote_id.into(),
        command: Some(cmd),
    })
    .await
    .expect("send");
    drop(tx);
    client
        .append_commands(ReceiverStream::new(rx))
        .await
        .expect("append");
}

#[tokio::test]
#[ignore = "long-running soak; run manually: cargo test -p quote_ledger soak_many_parallel_line_items -- --ignored"]
async fn soak_many_parallel_line_items() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("soak.db");
    let conn = sqlite::open_and_migrate(db.to_str().expect("utf8")).expect("migrate");

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
    tokio::time::sleep(Duration::from_millis(200)).await;

    let endpoint = format!("http://{addr}");
    let mut bootstrap = QuoteLedgerServiceClient::connect(endpoint.clone())
        .await
        .expect("connect");
    let quote_id = "q-soak-1";

    append_one(
        &mut bootstrap,
        quote_id,
        "create",
        QuoteCommand {
            kind: Some(CmdKind::CreateQuote(CreateQuote {
                currency_code: "USD".into(),
                jurisdiction_id: "US-CA".into(),
            })),
        },
    )
    .await;

    const N: usize = 128;
    let mut join_set = tokio::task::JoinSet::new();
    for i in 0..N {
        let ep = endpoint.clone();
        let q = quote_id.to_string();
        join_set.spawn(async move {
            let mut c = QuoteLedgerServiceClient::connect(ep)
                .await
                .expect("connect");
            append_one(
                &mut c,
                &q,
                &format!("line-{i}"),
                QuoteCommand {
                    kind: Some(CmdKind::AddLineItem(AddLineItem {
                        line_id: format!("L{i}"),
                        sku: "SKU".into(),
                        description: "Soak".into(),
                        quantity: 1,
                        unit_minor: 1,
                    })),
                },
            )
            .await;
        });
    }
    while let Some(r) = join_set.join_next().await {
        r.expect("writer");
    }

    let mut reader = QuoteLedgerServiceClient::connect(endpoint)
        .await
        .expect("connect");
    let mut sub = reader
        .subscribe_quote(SubscribeQuoteRequest {
            quote_id: quote_id.into(),
            after_seq: 0,
        })
        .await
        .expect("subscribe")
        .into_inner();

    let mut saw = false;
    while let Some(msg) = sub.message().await.expect("stream") {
        if let UpdateKind::Snapshot(s) = msg.kind.expect("kind") {
            assert_eq!(s.last_seq, (N + 1) as u64);
            let v = s.view.expect("view");
            assert_eq!(v.line_items.len(), N);
            saw = true;
            break;
        }
    }
    assert!(saw);
}
