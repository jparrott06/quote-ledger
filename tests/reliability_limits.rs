use std::time::Duration;

use quote_ledger::v1::quote_command::Kind as CmdKind;
use quote_ledger::v1::quote_ledger_service_client::QuoteLedgerServiceClient;
use quote_ledger::v1::quote_ledger_service_server::QuoteLedgerServiceServer;
use quote_ledger::v1::{AppendCommandRequest, CreateQuote, QuoteCommand};
use quote_ledger::{sqlite, LedgerService, ReliabilityLimits};
use tokio::net::TcpListener;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::transport::Server;
use tonic::Code;

async fn start_server_with_startup_delay(
    limits: ReliabilityLimits,
    startup_delay: Duration,
) -> String {
    let dir = Box::leak(Box::new(tempfile::tempdir().expect("tempdir")));
    let db = dir.path().join("reliability.db");
    let conn = sqlite::open_and_migrate(db.to_str().expect("utf8 path")).expect("migrate");

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let incoming = TcpListenerStream::new(listener);
    let svc = QuoteLedgerServiceServer::new(LedgerService::with_reliability(conn, limits));

    tokio::spawn(async move {
        Server::builder()
            .add_service(svc)
            .serve_with_incoming(incoming)
            .await
            .expect("server");
    });

    if !startup_delay.is_zero() {
        tokio::time::sleep(startup_delay).await;
    }
    format!("http://{addr}")
}

async fn start_server(limits: ReliabilityLimits) -> String {
    start_server_with_startup_delay(limits, Duration::from_millis(150)).await
}

#[tokio::test]
async fn append_stream_times_out_when_idle() {
    let endpoint = start_server(ReliabilityLimits {
        append_idle_timeout: Duration::from_millis(150),
        append_max_commands: 8,
    })
    .await;
    let mut client = QuoteLedgerServiceClient::connect(endpoint)
        .await
        .expect("connect");

    let (_tx, rx) = tokio::sync::mpsc::channel::<AppendCommandRequest>(1);
    let err = client
        .append_commands(ReceiverStream::new(rx))
        .await
        .expect_err("idle stream should timeout");
    assert_eq!(err.code(), Code::DeadlineExceeded);
}

/// Append idle timeout is driven by Tokio timers; with a paused clock we can assert behavior
/// without sleeping for hundreds of milliseconds of wall time (see L-03).
#[tokio::test(start_paused = true)]
async fn append_stream_idle_timeout_with_paused_clock() {
    let endpoint = start_server_with_startup_delay(
        ReliabilityLimits {
            append_idle_timeout: Duration::from_millis(100),
            append_max_commands: 8,
        },
        Duration::ZERO,
    )
    .await;

    for _ in 0..8 {
        tokio::task::yield_now().await;
    }

    let mut client = QuoteLedgerServiceClient::connect(endpoint)
        .await
        .expect("connect");

    let (_tx, rx) = tokio::sync::mpsc::channel::<AppendCommandRequest>(1);
    let append = tokio::spawn(async move { client.append_commands(ReceiverStream::new(rx)).await });

    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_millis(250)).await;

    let err = append
        .await
        .expect("join append task")
        .expect_err("idle stream should timeout");
    assert_eq!(err.code(), Code::DeadlineExceeded);
}

#[tokio::test]
async fn append_stream_rejects_too_many_commands() {
    let endpoint = start_server(ReliabilityLimits {
        append_idle_timeout: Duration::from_secs(5),
        append_max_commands: 1,
    })
    .await;
    let mut client = QuoteLedgerServiceClient::connect(endpoint)
        .await
        .expect("connect");

    let cmd = QuoteCommand {
        kind: Some(CmdKind::CreateQuote(CreateQuote {
            currency_code: "USD".into(),
            jurisdiction_id: "US-CA".into(),
        })),
    };

    let (tx, rx) = tokio::sync::mpsc::channel::<AppendCommandRequest>(4);
    tx.send(AppendCommandRequest {
        client_command_id: "cc-1".into(),
        quote_id: "q-reliability-1".into(),
        command: Some(cmd.clone()),
    })
    .await
    .expect("send first");
    tx.send(AppendCommandRequest {
        client_command_id: "cc-2".into(),
        quote_id: "q-reliability-1".into(),
        command: Some(cmd),
    })
    .await
    .expect("send second");
    drop(tx);

    let err = client
        .append_commands(ReceiverStream::new(rx))
        .await
        .expect_err("too many commands should fail");
    assert_eq!(err.code(), Code::ResourceExhausted);
}
