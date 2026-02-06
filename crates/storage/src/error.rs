use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("constraint violation: {0}")]
    ConstraintViolation(String),

    #[error("entity collision: {entity_id}")]
    EntityCollision { entity_id: String },

    #[error("core error: {0}")]
    Core(#[from] openprod_core::CoreError),
}
