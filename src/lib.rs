//! Quote ledger: append-only events, gRPC API, SQLite persistence (see repo epics).

pub mod v1 {
    tonic::include_proto!("quote_ledger.v1");
}

use std::pin::Pin;

use tokio_stream::Stream;
use tonic::{Request, Response, Status};
use v1::quote_ledger_service_server::{QuoteLedgerService, QuoteLedgerServiceServer};
use v1::{AppendCommandRequest, AppendCommandsResponse, QuoteUpdate, SubscribeQuoteRequest};

pub mod sqlite;

pub struct LedgerService;

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
    ) -> Result<Response<<Self as QuoteLedgerService>::SubscribeQuoteStream>, Status> {
        Err(Status::unimplemented(
            "subscribe_quote will snapshot then tail with after_seq replay",
        ))
    }
}

pub fn grpc_server(service: LedgerService) -> QuoteLedgerServiceServer<LedgerService> {
    QuoteLedgerServiceServer::new(service)
}
