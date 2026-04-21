use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),

    #[error(transparent)]
    Domain(#[from] crate::domain::DomainError),

    #[error("append contained zero events")]
    EmptyAppend,

    #[error("after_seq {after_seq} is ahead of head {last_seq}")]
    InvalidAfterSeq { after_seq: u64, last_seq: u64 },

    #[error("corrupt stored payload: {0}")]
    Corrupt(String),
}
