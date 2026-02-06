use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("invalid signature")]
    InvalidSignature,

    #[error("HLC drift too large: remote is {delta_ms}ms ahead (max {max_ms}ms)")]
    HlcDriftTooLarge { delta_ms: u64, max_ms: u64 },

    #[error("invalid operation: {0}")]
    InvalidOperation(String),

    #[error("invalid data: {0}")]
    InvalidData(String),
}
