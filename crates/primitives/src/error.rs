#[derive(Debug, thiserror::Error)]
pub enum ChainSpecError {
    #[error("The chain {0} is not supported")]
    ChainNotSupported(u64),

    #[error("This conversion is not allowed")]
    InvalidConversion,

    #[error("Serde error: {0}")]
    Json(#[from] serde_json::Error),
}
