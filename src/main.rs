use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use tonic::transport::Server;
use tonic_reflection::server::Builder as ReflectionBuilder;

use quote_ledger::v1::quote_ledger_service_server::QuoteLedgerServiceServer;
use quote_ledger::{sqlite, AuthInterceptor, LedgerService, FILE_DESCRIPTOR_SET};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let addr = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:50051".to_string())
        .parse()?;

    let metrics_addr: std::net::SocketAddr = std::env::var("METRICS_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:9090".into())
        .parse()
        .map_err(|e| format!("METRICS_ADDR: {e}"))?;

    let prometheus = PrometheusBuilder::new()
        .install_recorder()
        .map_err(|e| format!("prometheus recorder: {e}"))?;

    let metrics_app = Router::new()
        .route("/metrics", get(metrics_scrape))
        .with_state(prometheus);

    let metrics_listener = tokio::net::TcpListener::bind(metrics_addr).await?;
    tracing::info!(%metrics_addr, "metrics: GET /metrics (Prometheus text format)");
    tokio::spawn(async move {
        if let Err(e) = axum::serve(metrics_listener, metrics_app).await {
            tracing::error!(error = %e, "metrics server stopped");
        }
    });

    let db_path = std::env::var("QUOTE_LEDGER_DB").unwrap_or_else(|_| "quote_ledger.db".into());
    let conn = sqlite::open_and_migrate(&db_path)?;
    tracing::info!(path = %db_path, "sqlite ready");

    let auth = AuthInterceptor::from_env_var("QUOTE_LEDGER_AUTH_TOKEN")
        .map_err(|e| format!("QUOTE_LEDGER_AUTH_TOKEN: {e}"))?;

    let ledger_service = LedgerService::new(conn);
    let in_flight = ledger_service.in_flight_counter();
    let ledger = QuoteLedgerServiceServer::with_interceptor(ledger_service, auth);

    let reflection = ReflectionBuilder::configure()
        .register_encoded_file_descriptor_set(FILE_DESCRIPTOR_SET)
        .build_v1()?;

    tracing::info!(%addr, "quote_ledger listening (reflection enabled for grpcurl)");

    Server::builder()
        .add_service(reflection)
        .add_service(ledger)
        .serve_with_shutdown(addr, shutdown_signal(in_flight))
        .await?;

    tracing::info!("shutdown complete");
    Ok(())
}

async fn metrics_scrape(State(handle): State<PrometheusHandle>) -> impl IntoResponse {
    handle.run_upkeep();
    let body = handle.render();
    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
}

async fn shutdown_signal(in_flight: Arc<AtomicUsize>) {
    if tokio::signal::ctrl_c().await.is_err() {
        return;
    }

    tracing::info!("shutdown signal received; draining in-flight appends (up to 30s)");

    const DRAIN_TIMEOUT: Duration = Duration::from_secs(30);
    const TICK: Duration = Duration::from_millis(200);
    let deadline = Instant::now() + DRAIN_TIMEOUT;

    loop {
        let n = in_flight.load(Ordering::SeqCst);
        if n == 0 {
            tracing::info!("drain complete (no in-flight appends)");
            break;
        }
        if Instant::now() >= deadline {
            tracing::warn!(
                in_flight = n,
                "drain deadline reached; proceeding with shutdown"
            );
            break;
        }
        tracing::info!(in_flight = n, "waiting for in-flight appends to finish");
        tokio::time::sleep(TICK).await;
    }
}
