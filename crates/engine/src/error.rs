use openprod_core::CoreError;
use openprod_storage::StorageError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),

    #[error("core error: {0}")]
    Core(#[from] CoreError),

    #[error("entity not found: {0}")]
    EntityNotFound(String),

    #[error("entity already deleted: {0}")]
    EntityAlreadyDeleted(String),

    #[error("conflict not found: {0}")]
    ConflictNotFound(String),

    #[error("conflict already resolved: {0}")]
    ConflictAlreadyResolved(String),

    #[error("overlay not found: {0}")]
    OverlayNotFound(String),

    #[error("no active overlay")]
    NoActiveOverlay,

    #[error("overlay is empty: {0}")]
    EmptyOverlay(String),

    #[error("unresolved drift on overlay: {0}")]
    UnresolvedDrift(String),
}
