//! Integration: concurrency and fault-path behavior.

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
use tonic::Code;

async fn start_server() -> String {
    let dir = Box::leak(Box::new(tempfile::tempdir().expect("tempdir")));
    let db = dir.path().join("concurrency.db");
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
    format!("http://{addr}")
}

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
async fn concurrent_appenders_produce_consistent_snapshot() {
    let endpoint = start_server().await;
    let mut bootstrap = QuoteLedgerServiceClient::connect(endpoint.clone())
        .await
        .expect("connect");
    let quote_id = "q-concurrent-1";

    append_one(
        &mut bootstrap,
        quote_id,
        "create-1",
        QuoteCommand {
            kind: Some(CmdKind::CreateQuote(CreateQuote {
                currency_code: "USD".into(),
                jurisdiction_id: "US-CA".into(),
            })),
        },
    )
    .await;

    const WRITERS: usize = 24;
    let mut join_set = tokio::task::JoinSet::new();
    for i in 0..WRITERS {
        let endpoint_i = endpoint.clone();
        let quote_id_i = quote_id.to_string();
        join_set.spawn(async move {
            let mut c = QuoteLedgerServiceClient::connect(endpoint_i)
                .await
                .expect("connect writer");
            append_one(
                &mut c,
                &quote_id_i,
                &format!("cc-{i}"),
                QuoteCommand {
                    kind: Some(CmdKind::AddLineItem(AddLineItem {
                        line_id: format!("L{i}"),
                        sku: "SKU".into(),
                        description: "Widget".into(),
                        quantity: 1,
                        unit_minor: 100,
                    })),
                },
            )
            .await;
        });
    }

    while let Some(res) = join_set.join_next().await {
        res.expect("writer task");
    }

    let mut reader = QuoteLedgerServiceClient::connect(endpoint)
        .await
        .expect("connect reader");
    let mut sub = reader
        .subscribe_quote(SubscribeQuoteRequest {
            quote_id: quote_id.into(),
            after_seq: 0,
        })
        .await
        .expect("subscribe")
        .into_inner();

    let mut saw_snapshot = false;
    while let Some(msg) = sub.message().await.expect("stream") {
        if let UpdateKind::Snapshot(snapshot) = msg.kind.expect("kind") {
            assert_eq!(snapshot.last_seq, (WRITERS + 1) as u64);
            let view = snapshot.view.expect("view");
            assert_eq!(view.line_items.len(), WRITERS);
            assert_eq!(view.subtotal_minor, (WRITERS as i64) * 100);
            assert_eq!(view.tax_minor, (WRITERS as i64) * 8);
            assert_eq!(view.total_minor, (WRITERS as i64) * 108);
            saw_snapshot = true;
            break;
        }
    }
    assert!(saw_snapshot, "expected a snapshot frame");
}

#[tokio::test]
async fn subscribe_rejects_after_seq_past_head() {
    let endpoint = start_server().await;
    let mut client = QuoteLedgerServiceClient::connect(endpoint)
        .await
        .expect("connect");
    let quote_id = "q-after-head-1";

    append_one(
        &mut client,
        quote_id,
        "create-1",
        QuoteCommand {
            kind: Some(CmdKind::CreateQuote(CreateQuote {
                currency_code: "USD".into(),
                jurisdiction_id: "US-CA".into(),
            })),
        },
    )
    .await;

    let err = client
        .subscribe_quote(SubscribeQuoteRequest {
            quote_id: quote_id.into(),
            after_seq: 999,
        })
        .await
        .expect_err("after_seq beyond head should fail");
    assert_eq!(err.code(), Code::InvalidArgument);
}
