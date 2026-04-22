/// Domain validation / reducer failures (map to gRPC status in the transport layer).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DomainError {
    #[error("quote is already initialized")]
    QuoteAlreadyCreated,

    #[error("create_quote is invalid because a quote already exists")]
    DuplicateCreateQuote,

    #[error("invalid field: {field}")]
    InvalidField { field: &'static str },

    #[error("quote must be created before this operation")]
    QuoteNotCreated,

    #[error("quote is finalized and cannot be modified")]
    QuoteAlreadyFinalized,

    #[error("line_id already exists on this quote")]
    DuplicateLineId,

    #[error("quantity must be positive")]
    InvalidQuantity,

    #[error("unit_minor must be non-negative")]
    InvalidUnitMinor,

    #[error("integer overflow in money math")]
    IntegerOverflow,

    #[error("finalize requires at least one line item")]
    CannotFinalizeWithoutLines,
}
