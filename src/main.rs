use std::pin::Pin;

use tokio_stream::Stream;
use tonic::{transport::Server, Request, Response, Status};

pub mod sqlite;

mod quote_ledger {
    tonic::include_proto!("quote_ledger.v1");
}

use quote_ledger::quote_ledger_service_server::{QuoteLedgerService, QuoteLedgerServiceServer};
use quote_ledger::{
    AppendCommandRequest, AppendCommandsResponse, QuoteUpdate, SubscribeQuoteRequest,
};

#[derive(Default)]
struct LedgerService;

#[tonic::async_trait]
impl QuoteLedgerService for LedgerService {
    async fn append_commands(
        &self,
        _request: Request<tonic::Streaming<AppendCommandRequest>>,
    ) -> Result<Response<AppendCommandsResponse>, Status> {
        Err(Status::unimplemented(
            "append_commands will validate, persist, and assign seq (see project epics)",
        ))
    }

    type SubscribeQuoteStream =
        Pin<Box<dyn Stream<Item = Result<QuoteUpdate, Status>> + Send + 'static>>;

    async fn subscribe_quote(
        &self,
        _request: Request<SubscribeQuoteRequest>,
    ) -> Result<Response<Self::SubscribeQuoteStream>, Status> {
        Err(Status::unimplemented(
            "subscribe_quote will snapshot then tail with after_seq replay",
        ))
    }
}

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
    let _conn = sqlite::open_and_migrate(&db_path)?;
    tracing::info!(path = %db_path, "sqlite ready");

    let service = LedgerService::default();

    tracing::info!(%addr, "quote_ledger listening");

    Server::builder()
        .add_service(QuoteLedgerServiceServer::new(service))
        .serve(addr)
        .await?;

    Ok(())
}
