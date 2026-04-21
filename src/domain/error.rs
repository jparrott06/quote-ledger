/// Domain validation / reducer failures (map to gRPC status in the transport layer).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DomainError {
    #[error("quote is already initialized")]
    QuoteAlreadyCreated,

    #[error("create_quote is invalid because a quote already exists")]
    DuplicateCreateQuote,

    #[error("invalid field: {field}")]
    InvalidField { field: &'static str },
}
