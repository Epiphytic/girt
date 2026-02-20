use thiserror::Error;

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("Component not found: {0}")]
    ComponentNotFound(String),

    #[error("Tool not found: {0}")]
    ToolNotFound(String),

    #[error("Compilation failed: {0}")]
    CompilationFailed(String),

    #[error("Instantiation failed: {0}")]
    InstantiationFailed(String),

    #[error("Invocation failed: {0}")]
    InvocationFailed(String),

    #[error("Tool returned error: {0}")]
    ToolError(String),

    #[error("Storage error: {0}")]
    StorageError(String),

    #[error("Invalid component metadata: {0}")]
    InvalidMetadata(String),

    #[error(transparent)]
    Anyhow(#[from] anyhow::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
