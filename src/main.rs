use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use quote_ledger::v1::quote_ledger_service_server::QuoteLedgerServiceServer;
use quote_ledger::{
    sqlite, AuthInterceptor, LedgerGrpcInterceptor, LedgerService, ServiceConfig,
    FILE_DESCRIPTOR_SET,
};
use tonic::transport::Server;
use tonic_reflection::server::Builder as ReflectionBuilder;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cfg = ServiceConfig::from_env_and_args().map_err(|e| {
        Box::<dyn std::error::Error + Send + Sync>::from(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            e,
        ))
    })?;

    let prometheus = PrometheusBuilder::new()
        .install_recorder()
        .map_err(|e| format!("prometheus recorder: {e}"))?;

    let metrics_app = Router::new()
        .route("/metrics", get(metrics_scrape))
        .with_state(prometheus);

    let metrics_listener = tokio::net::TcpListener::bind(cfg.metrics_listen).await?;
    tracing::info!(addr = %cfg.metrics_listen, "metrics: GET /metrics (Prometheus text format)");
    tokio::spawn(async move {
        if let Err(e) = axum::serve(metrics_listener, metrics_app).await {
            tracing::error!(error = %e, "metrics server stopped");
        }
    });

    let conn = sqlite::open_and_migrate(&cfg.db_path)?;
    tracing::info!(path = %cfg.db_path, "sqlite ready");

    let auth = AuthInterceptor::from_env_var("QUOTE_LEDGER_AUTH_TOKEN")
        .map_err(|e| format!("QUOTE_LEDGER_AUTH_TOKEN: {e}"))?;

    tracing::info!(
        grpc = %cfg.grpc_listen,
        append_idle_timeout_ms = %cfg.reliability.append_idle_timeout.as_millis(),
        append_max_commands = cfg.reliability.append_max_commands,
        grpc_concurrency_limit = cfg.grpc_concurrency_limit,
        grpc_keepalive_interval_ms = %cfg.grpc_keepalive_interval.as_millis(),
        grpc_keepalive_timeout_ms = %cfg.grpc_keepalive_timeout.as_millis(),
        reflection_enabled = cfg.reflection_enabled,
        tls_enabled = cfg.grpc_tls_cert.is_some(),
        grpc_rate_limit_rps = ?cfg.grpc_rate_limit_rps,
        "service configuration"
    );

    let ledger_service = LedgerService::with_reliability(conn, cfg.reliability.clone());
    let in_flight = ledger_service.in_flight_counter();
    let interceptor = LedgerGrpcInterceptor::new(auth, cfg.grpc_rate_limit_rps);
    let ledger = QuoteLedgerServiceServer::with_interceptor(ledger_service, interceptor);

    let mut server_builder = Server::builder()
        .concurrency_limit_per_connection(cfg.grpc_concurrency_limit)
        .http2_keepalive_interval(Some(cfg.grpc_keepalive_interval))
        .http2_keepalive_timeout(Some(cfg.grpc_keepalive_timeout));

    if let (Some(cert_path), Some(key_path)) = (&cfg.grpc_tls_cert, &cfg.grpc_tls_key) {
        let cert = std::fs::read(cert_path)?;
        let key = std::fs::read(key_path)?;
        let identity = tonic::transport::Identity::from_pem(cert, key);
        let tls = tonic::transport::ServerTlsConfig::new().identity(identity);
        server_builder = server_builder.tls_config(tls)?;
    }

    tracing::info!(addr = %cfg.grpc_listen, "quote_ledger listening");

    if cfg.reflection_enabled {
        let reflection = ReflectionBuilder::configure()
            .register_encoded_file_descriptor_set(FILE_DESCRIPTOR_SET)
            .build_v1()?;
        server_builder
            .add_service(reflection)
            .add_service(ledger)
            .serve_with_shutdown(cfg.grpc_listen, shutdown_signal(in_flight))
            .await?;
    } else {
        server_builder
            .add_service(ledger)
            .serve_with_shutdown(cfg.grpc_listen, shutdown_signal(in_flight))
            .await?;
    }

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
