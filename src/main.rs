use tonic::transport::Server;
use tonic_reflection::server::Builder as ReflectionBuilder;

use quote_ledger::{grpc_server, sqlite, LedgerService, FILE_DESCRIPTOR_SET};

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

    let db_path = std::env::var("QUOTE_LEDGER_DB").unwrap_or_else(|_| "quote_ledger.db".into());
    let conn = sqlite::open_and_migrate(&db_path)?;
    tracing::info!(path = %db_path, "sqlite ready");

    let ledger = grpc_server(LedgerService::new(conn));

    let reflection = ReflectionBuilder::configure()
        .register_encoded_file_descriptor_set(FILE_DESCRIPTOR_SET)
        .build_v1()?;

    tracing::info!(%addr, "quote_ledger listening (reflection enabled for grpcurl)");

    Server::builder()
        .add_service(reflection)
        .add_service(ledger)
        .serve_with_shutdown(addr, shutdown_signal())
        .await?;

    tracing::info!("shutdown complete");
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutdown signal received");
}
