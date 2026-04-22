use std::time::Duration;

use quote_ledger::v1::quote_command::Kind as CmdKind;
use quote_ledger::v1::quote_ledger_service_client::QuoteLedgerServiceClient;
use quote_ledger::v1::quote_ledger_service_server::QuoteLedgerServiceServer;
use quote_ledger::v1::{AppendCommandRequest, CreateQuote, QuoteCommand, SubscribeQuoteRequest};
use quote_ledger::{grpc_server, sqlite, AuthInterceptor, LedgerService};
use tokio::net::TcpListener;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::metadata::MetadataValue;
use tonic::transport::Server;
use tonic::Code;
use tonic::Request;

async fn start_server(auth: Option<&str>) -> String {
    let dir = Box::leak(Box::new(tempfile::tempdir().expect("tempdir")));
    let db = dir.path().join("auth.db");
    let conn = sqlite::open_and_migrate(db.to_str().expect("utf8 path")).expect("migrate");

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let incoming = TcpListenerStream::new(listener);

    match auth {
        Some(token) => {
            let interceptor = AuthInterceptor::required(token);
            let svc =
                QuoteLedgerServiceServer::with_interceptor(LedgerService::new(conn), interceptor);
            tokio::spawn(async move {
                Server::builder()
                    .add_service(svc)
                    .serve_with_incoming(incoming)
                    .await
                    .expect("server");
            });
        }
        None => {
            let svc = grpc_server(LedgerService::new(conn));
            tokio::spawn(async move {
                Server::builder()
                    .add_service(svc)
                    .serve_with_incoming(incoming)
                    .await
                    .expect("server");
            });
        }
    };

    tokio::time::sleep(Duration::from_millis(150)).await;
    format!("http://{addr}")
}

#[tokio::test]
async fn append_requires_bearer_token_when_enabled() {
    let endpoint = start_server(Some("secret-token")).await;
    let mut client = QuoteLedgerServiceClient::connect(endpoint)
        .await
        .expect("connect");

    let cmd = QuoteCommand {
        kind: Some(CmdKind::CreateQuote(CreateQuote {
            currency_code: "USD".into(),
            jurisdiction_id: "US-CA".into(),
        })),
    };
    let (tx, rx) = tokio::sync::mpsc::channel::<AppendCommandRequest>(2);
    tx.send(AppendCommandRequest {
        client_command_id: "cc-1".into(),
        quote_id: "q-auth-1".into(),
        command: Some(cmd),
    })
    .await
    .expect("send");
    drop(tx);

    let err = client
        .append_commands(ReceiverStream::new(rx))
        .await
        .expect_err("missing auth should fail");
    assert_eq!(err.code(), Code::Unauthenticated);
}

#[tokio::test]
async fn subscribe_accepts_valid_bearer_token() {
    let endpoint = start_server(Some("secret-token")).await;
    let mut client = QuoteLedgerServiceClient::connect(endpoint)
        .await
        .expect("connect");

    let mut req = Request::new(SubscribeQuoteRequest {
        quote_id: "q-auth-2".into(),
        after_seq: 0,
    });
    req.metadata_mut().insert(
        "authorization",
        MetadataValue::from_static("Bearer secret-token"),
    );

    let response = client.subscribe_quote(req).await.expect("authorized");
    let mut stream = response.into_inner();
    let first = stream
        .message()
        .await
        .expect("stream")
        .expect("first frame");
    assert!(first.kind.is_some());
}

#[tokio::test]
async fn append_rejects_invalid_quote_id_characters() {
    let endpoint = start_server(None).await;
    let mut client = QuoteLedgerServiceClient::connect(endpoint)
        .await
        .expect("connect");

    let cmd = QuoteCommand {
        kind: Some(CmdKind::CreateQuote(CreateQuote {
            currency_code: "USD".into(),
            jurisdiction_id: "US-CA".into(),
        })),
    };
    let (tx, rx) = tokio::sync::mpsc::channel::<AppendCommandRequest>(2);
    tx.send(AppendCommandRequest {
        client_command_id: "cc-1".into(),
        quote_id: "bad id".into(),
        command: Some(cmd),
    })
    .await
    .expect("send");
    drop(tx);

    let err = client
        .append_commands(ReceiverStream::new(rx))
        .await
        .expect_err("invalid quote_id");
    assert_eq!(err.code(), Code::InvalidArgument);
}
