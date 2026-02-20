use thiserror::Error;

#[derive(Debug, Error)]
pub enum DecisionError {
    #[error("policy layer error: {0}")]
    PolicyError(String),

    #[error("cache layer error: {0}")]
    CacheError(String),

    #[error("registry lookup error: {0}")]
    RegistryError(String),

    #[error("CLI check error: {0}")]
    CliCheckError(String),

    #[error("LLM evaluation error: {0}")]
    LlmError(String),

    #[error("HITL layer error: {0}")]
    HitlError(String),

    #[error("serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),
}
