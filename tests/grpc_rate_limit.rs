//! Global gRPC rate limit (GRPC_RATE_LIMIT_RPS) enforced via `LedgerGrpcInterceptor`.

use std::num::NonZeroU32;
use std::time::Duration;

use quote_ledger::v1::quote_ledger_service_client::QuoteLedgerServiceClient;
use quote_ledger::v1::quote_ledger_service_server::QuoteLedgerServiceServer;
use quote_ledger::v1::SubscribeQuoteRequest;
use quote_ledger::{sqlite, AuthInterceptor, LedgerGrpcInterceptor, LedgerService};
use tokio::net::TcpListener;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::metadata::MetadataValue;
use tonic::transport::Server;
use tonic::Code;
use tonic::Request;

async fn start_limited_server(rps: NonZeroU32, auth_token: Option<&str>) -> String {
    let dir = Box::leak(Box::new(tempfile::tempdir().expect("tempdir")));
    let db = dir.path().join("rate_limit.db");
    let conn = sqlite::open_and_migrate(db.to_str().expect("utf8 path")).expect("migrate");

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let incoming = TcpListenerStream::new(listener);

    let auth = match auth_token {
        Some(token) => AuthInterceptor::required(token),
        None => AuthInterceptor::from_env_var("QUOTE_LEDGER_AUTH_TOKEN_UNUSED_FOR_GRPC_RATE_TEST")
            .expect("auth env"),
    };
    let interceptor = LedgerGrpcInterceptor::new(auth, Some(rps));
    let svc = QuoteLedgerServiceServer::with_interceptor(LedgerService::new(conn), interceptor);

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

#[tokio::test]
async fn second_subscribe_hits_global_rate_limit() {
    let endpoint = start_limited_server(NonZeroU32::new(1).expect("nz"), None).await;
    let mut client = QuoteLedgerServiceClient::connect(endpoint)
        .await
        .expect("connect");

    let _first = client
        .subscribe_quote(SubscribeQuoteRequest {
            quote_id: "q-rate-1".into(),
            after_seq: 0,
        })
        .await
        .expect("first rpc");

    let err = client
        .subscribe_quote(SubscribeQuoteRequest {
            quote_id: "q-rate-1".into(),
            after_seq: 0,
        })
        .await
        .expect_err("second rpc should be rate limited");
    assert_eq!(err.code(), Code::ResourceExhausted);
}

#[tokio::test]
async fn unauthenticated_request_does_not_consume_rate_limit_budget() {
    let endpoint =
        start_limited_server(NonZeroU32::new(1).expect("nz"), Some("secret-token")).await;
    let mut client = QuoteLedgerServiceClient::connect(endpoint)
        .await
        .expect("connect");

    let unauth_err = client
        .subscribe_quote(SubscribeQuoteRequest {
            quote_id: "q-rate-2".into(),
            after_seq: 0,
        })
        .await
        .expect_err("missing auth should fail");
    assert_eq!(unauth_err.code(), Code::Unauthenticated);

    let mut req = Request::new(SubscribeQuoteRequest {
        quote_id: "q-rate-2".into(),
        after_seq: 0,
    });
    req.metadata_mut().insert(
        "authorization",
        MetadataValue::from_static("Bearer secret-token"),
    );
    let _ok = client.subscribe_quote(req).await.expect(
        "authorized request should still pass (unauthenticated call must not consume limiter budget)",
    );
}
