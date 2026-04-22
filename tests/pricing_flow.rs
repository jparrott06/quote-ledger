//! Integration: create quote, add line, finalize; snapshot shows totals and finalized=true.

use std::time::Duration;

use quote_ledger::v1::quote_command::Kind as CmdKind;
use quote_ledger::v1::quote_ledger_service_client::QuoteLedgerServiceClient;
use quote_ledger::v1::quote_update::Kind as UpdateKind;
use quote_ledger::v1::{
    AddLineItem, AppendCommandRequest, CreateQuote, FinalizeQuote, QuoteCommand,
    SubscribeQuoteRequest,
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
async fn pricing_finalize_snapshot() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("pricing.db");
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

    let quote_id = "q-price-1";

    append_one(
        &mut client,
        quote_id,
        "c-create",
        QuoteCommand {
            kind: Some(CmdKind::CreateQuote(CreateQuote {
                currency_code: "USD".into(),
                jurisdiction_id: "US-CA".into(),
            })),
        },
    )
    .await;

    append_one(
        &mut client,
        quote_id,
        "c-line",
        QuoteCommand {
            kind: Some(CmdKind::AddLineItem(AddLineItem {
                line_id: "L1".into(),
                sku: "SKU".into(),
                description: "Widget".into(),
                quantity: 2,
                unit_minor: 5_000,
            })),
        },
    )
    .await;

    append_one(
        &mut client,
        quote_id,
        "c-final",
        QuoteCommand {
            kind: Some(CmdKind::FinalizeQuote(FinalizeQuote {})),
        },
    )
    .await;

    let mut sub = client
        .subscribe_quote(SubscribeQuoteRequest {
            quote_id: quote_id.into(),
            after_seq: 0,
        })
        .await
        .expect("subscribe ok")
        .into_inner();

    let mut saw_snapshot = false;
    while let Some(msg) = sub.message().await.expect("stream") {
        if let UpdateKind::Snapshot(s) = msg.kind.expect("kind") {
            assert_eq!(s.last_seq, 3);
            let v = s.view.expect("view");
            assert!(v.finalized);
            assert_eq!(v.subtotal_minor, 10_000);
            assert_eq!(v.tax_minor, 800);
            assert_eq!(v.total_minor, 10_800);
            assert_eq!(v.line_items.len(), 1);
            assert_eq!(v.line_items[0].line_total_minor, 10_000);
            saw_snapshot = true;
            break;
        }
    }

    assert!(saw_snapshot);
}
