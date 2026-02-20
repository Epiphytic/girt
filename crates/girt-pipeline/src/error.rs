use thiserror::Error;

#[derive(Debug, Error)]
pub enum PipelineError {
    #[error("queue error: {0}")]
    QueueError(String),

    #[error("LLM call failed: {0}")]
    LlmError(String),

    #[error("compilation failed: {0}")]
    CompilationError(String),

    #[error("QA test failed: {0}")]
    QaError(String),

    #[error("security audit failed: {0}")]
    SecurityError(String),

    #[error("circuit breaker triggered after {attempts} attempts: {summary}")]
    CircuitBreaker { attempts: u32, summary: String },

    #[error("publish error: {0}")]
    PublishError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),
}
