//! Quote ledger: append-only events, gRPC API, SQLite persistence (see repo epics).

pub mod v1 {
    tonic::include_proto!("quote_ledger.v1");
}

pub mod domain;
pub mod sqlite;

mod auth;
mod config;
mod error;
mod grpc_interceptor;
mod ledger;
mod mapping;
mod store;

pub use auth::AuthInterceptor;
pub use config::ServiceConfig;
pub use error::StoreError;
pub use grpc_interceptor::LedgerGrpcInterceptor;
pub use ledger::{grpc_server, LedgerService, ReliabilityLimits};

/// gRPC reflection (`grpcurl` / Postman) — generated in `build.rs`.
pub const FILE_DESCRIPTOR_SET: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/quote_ledger_descriptor.bin"));
