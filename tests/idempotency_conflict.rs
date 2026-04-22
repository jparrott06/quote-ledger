//! Idempotency key collision: same client_command_id with different command must fail.

use std::time::Duration;

use quote_ledger::v1::quote_command::Kind as CmdKind;
use quote_ledger::v1::quote_ledger_service_client::QuoteLedgerServiceClient;
use quote_ledger::v1::{AddLineItem, AppendCommandRequest, CreateQuote, QuoteCommand};
use quote_ledger::{grpc_server, sqlite, LedgerService};
use tokio::net::TcpListener;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::transport::Channel;
use tonic::transport::Server;
use tonic::Code;

async fn append_one(
    client: &mut QuoteLedgerServiceClient<Channel>,
    quote_id: &str,
    client_command_id: &str,
    cmd: QuoteCommand,
) -> Result<(), tonic::Status> {
    let (tx, rx) = tokio::sync::mpsc::channel::<AppendCommandRequest>(2);
    tx.send(AppendCommandRequest {
        client_command_id: client_command_id.into(),
        quote_id: quote_id.into(),
        command: Some(cmd),
    })
    .await
    .expect("send");
    drop(tx);
    client.append_commands(ReceiverStream::new(rx)).await?;
    Ok(())
}

#[tokio::test]
async fn idempotency_conflict_on_same_key_different_command() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("idem.db");
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
    tokio::time::sleep(Duration::from_millis(150)).await;

    let mut client = QuoteLedgerServiceClient::connect(format!("http://{addr}"))
        .await
        .expect("connect");
    let quote_id = "q-idem-1";

    append_one(
        &mut client,
        quote_id,
        "same-key",
        QuoteCommand {
            kind: Some(CmdKind::CreateQuote(CreateQuote {
                currency_code: "USD".into(),
                jurisdiction_id: "US-CA".into(),
            })),
        },
    )
    .await
    .expect("first append");

    let err = append_one(
        &mut client,
        quote_id,
        "same-key",
        QuoteCommand {
            kind: Some(CmdKind::AddLineItem(AddLineItem {
                line_id: "L1".into(),
                sku: "SKU".into(),
                description: "Widget".into(),
                quantity: 1,
                unit_minor: 100,
            })),
        },
    )
    .await
    .expect_err("conflicting idempotency payload");

    assert_eq!(err.code(), Code::FailedPrecondition);
}
