use tonic::transport::Server;

use quote_ledger::{grpc_server, sqlite, LedgerService};

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

    let service = grpc_server(LedgerService::new(conn));

    tracing::info!(%addr, "quote_ledger listening");

    Server::builder().add_service(service).serve(addr).await?;

    Ok(())
}
