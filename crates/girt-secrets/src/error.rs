use thiserror::Error;

#[derive(Debug, Error)]
pub enum SecretError {
    #[error("secret not found: {service}")]
    NotFound { service: String },

    #[error("secret store unavailable: {0}")]
    StoreUnavailable(String),

    #[error("secret store configuration error: {0}")]
    ConfigError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}
