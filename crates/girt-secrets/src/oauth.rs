//! Anthropic OAuth token store for GIRT.
//!
//! Wraps the [`anthropic_auth`] crate to provide:
//! - A `girt auth login` flow (PKCE, Max or Console mode)
//! - File-backed token persistence (`~/.config/girt/auth.json`)
//! - Automatic token refresh on expiry
//!
//! ## Credential resolution order (in `girt-proxy`)
//!
//! 1. `ANTHROPIC_API_KEY` env var
//! 2. This store — `AnthropicOAuthStore::get_valid_token()` (auto-refreshes)
//! 3. OpenClaw `auth-profiles.json`
//! 4. `api_key` in `girt.toml`

use std::path::PathBuf;

use thiserror::Error;

// Re-export key types so callers only need to import from `girt_secrets`.
pub use anthropic_auth::{OAuthFlow, OAuthMode};
use anthropic_auth::{AsyncOAuthClient, OAuthConfig, TokenSet};

// ── Errors ────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum OAuthStoreError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("No credentials stored. Run `girt auth login`.")]
    NoTokenStored,

    #[error("Authentication error: {0}")]
    Auth(String),
}

// ── Status ────────────────────────────────────────────────────────────────────

/// Summary of stored token state (no refresh triggered).
#[derive(Debug, Clone)]
pub struct TokenStatus {
    /// First 16 characters of the access token for display purposes.
    pub access_token_prefix: String,
    /// Unix timestamp (seconds) when the access token expires.
    pub expires_at_unix: u64,
    /// Whether the token is expired (or expiring within 5 minutes).
    pub is_expired: bool,
    /// Whether a refresh token is stored.
    pub has_refresh_token: bool,
}

// ── Store ─────────────────────────────────────────────────────────────────────

/// File-backed Anthropic OAuth token store.
///
/// Tokens are persisted as JSON at `~/.config/girt/auth.json` (or a custom path
/// for testing). The stored format is [`anthropic_auth::TokenSet`], which is
/// already `Serialize + Deserialize`.
pub struct AnthropicOAuthStore {
    token_path: PathBuf,
}

impl AnthropicOAuthStore {
    /// Create a store using the default path: `~/.config/girt/auth.json`.
    pub fn new() -> Self {
        let path = dirs::home_dir()
            .expect("could not resolve home directory")
            .join(".config")
            .join("girt")
            .join("auth.json");
        Self { token_path: path }
    }

    /// Create a store with a custom token path. Useful for tests.
    pub fn with_path(path: PathBuf) -> Self {
        Self { token_path: path }
    }

    // ── Login flow ────────────────────────────────────────────────────────────

    /// Start a new OAuth PKCE login flow.
    ///
    /// Returns an [`OAuthFlow`] whose `authorization_url` the user must visit.
    /// After authorizing, they receive a `code#state` string which should be
    /// passed to [`complete_login`].
    ///
    /// This method is synchronous — no I/O is performed.
    pub fn start_login_flow(mode: OAuthMode) -> Result<OAuthFlow, OAuthStoreError> {
        let client = AsyncOAuthClient::new(OAuthConfig::default())
            .map_err(|e| OAuthStoreError::Auth(e.to_string()))?;
        client
            .start_flow(mode)
            .map_err(|e| OAuthStoreError::Auth(e.to_string()))
    }

    /// Exchange the authorization response for tokens and persist them.
    ///
    /// `response` is the `code#state` string the user pastes after authorizing
    /// (Anthropic returns authorization responses in `code#state` format).
    pub async fn complete_login(
        &self,
        response: &str,
        flow: &OAuthFlow,
    ) -> Result<(), OAuthStoreError> {
        let client = AsyncOAuthClient::new(OAuthConfig::default())
            .map_err(|e| OAuthStoreError::Auth(e.to_string()))?;
        let tokens = client
            .exchange_code(response, &flow.state, &flow.verifier)
            .await
            .map_err(|e| OAuthStoreError::Auth(e.to_string()))?;
        self.save_tokens(&tokens).await
    }

    // ── Token access ──────────────────────────────────────────────────────────

    /// Return a valid access token, refreshing automatically if expired.
    ///
    /// Returns `Ok(None)` when no token file exists (not logged in).
    pub async fn get_valid_token(&self) -> Result<Option<String>, OAuthStoreError> {
        let tokens = match self.load_tokens().await {
            Ok(t) => t,
            Err(OAuthStoreError::NoTokenStored) => return Ok(None),
            Err(e) => return Err(e),
        };

        if tokens.is_expired() {
            tracing::debug!("OAuth token expired or expiring soon — refreshing");
            let client = AsyncOAuthClient::new(OAuthConfig::default())
                .map_err(|e| OAuthStoreError::Auth(e.to_string()))?;
            let refreshed = client
                .refresh_token(&tokens.refresh_token)
                .await
                .map_err(|e| OAuthStoreError::Auth(e.to_string()))?;
            let access_token = refreshed.access_token.clone();
            self.save_tokens(&refreshed).await?;
            Ok(Some(access_token))
        } else {
            Ok(Some(tokens.access_token))
        }
    }

    /// Return token status without triggering a refresh.
    ///
    /// Returns `Ok(None)` when not logged in.
    pub async fn status(&self) -> Result<Option<TokenStatus>, OAuthStoreError> {
        match self.load_tokens().await {
            Ok(tokens) => Ok(Some(TokenStatus {
                access_token_prefix: tokens.access_token.chars().take(16).collect(),
                expires_at_unix: tokens.expires_at,
                is_expired: tokens.is_expired(),
                has_refresh_token: !tokens.refresh_token.is_empty(),
            })),
            Err(OAuthStoreError::NoTokenStored) => Ok(None),
            Err(e) => Err(e),
        }
    }

    // ── Logout ────────────────────────────────────────────────────────────────

    /// Delete stored credentials.
    pub fn logout(&self) -> Result<(), OAuthStoreError> {
        if self.token_path.exists() {
            std::fs::remove_file(&self.token_path).map_err(OAuthStoreError::Io)?;
            tracing::info!(path = %self.token_path.display(), "OAuth credentials removed");
        }
        Ok(())
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    async fn load_tokens(&self) -> Result<TokenSet, OAuthStoreError> {
        if !self.token_path.exists() {
            return Err(OAuthStoreError::NoTokenStored);
        }
        let content = tokio::fs::read_to_string(&self.token_path).await?;
        let tokens: TokenSet = serde_json::from_str(&content)?;
        Ok(tokens)
    }

    async fn save_tokens(&self, tokens: &TokenSet) -> Result<(), OAuthStoreError> {
        if let Some(parent) = self.token_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let content = serde_json::to_string_pretty(tokens)?;
        tokio::fs::write(&self.token_path, &content).await?;
        tracing::debug!(
            path = %self.token_path.display(),
            expires_at = tokens.expires_at,
            "OAuth credentials saved"
        );
        Ok(())
    }
}

impl Default for AnthropicOAuthStore {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn temp_store() -> (AnthropicOAuthStore, TempDir) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("auth.json");
        (AnthropicOAuthStore::with_path(path), dir)
    }

    #[tokio::test]
    async fn get_valid_token_returns_none_when_no_file() {
        let (store, _dir) = temp_store();
        let result = store.get_valid_token().await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn status_returns_none_when_no_file() {
        let (store, _dir) = temp_store();
        let result = store.status().await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn get_valid_token_returns_token_from_file() {
        let (store, _dir) = temp_store();

        // Write a valid (non-expired) TokenSet directly
        let tokens = TokenSet {
            access_token: "sk-ant-oat-test-token".into(),
            refresh_token: "refresh-token-abc".into(),
            // expires far in the future
            expires_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 3600,
        };
        store.save_tokens(&tokens).await.unwrap();

        let token = store.get_valid_token().await.unwrap();
        assert_eq!(token, Some("sk-ant-oat-test-token".into()));
    }

    #[tokio::test]
    async fn status_reports_correctly() {
        let (store, _dir) = temp_store();

        let future_expiry = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 3600;

        let tokens = TokenSet {
            access_token: "sk-ant-oat-preview1234567890".into(),
            refresh_token: "rfrsh".into(),
            expires_at: future_expiry,
        };
        store.save_tokens(&tokens).await.unwrap();

        let status = store.status().await.unwrap().unwrap();
        assert!(!status.is_expired);
        assert!(status.has_refresh_token);
        assert_eq!(status.expires_at_unix, future_expiry);
        // "sk-ant-oat-preview1234567890"[..16] == "sk-ant-oat-previ"
        assert_eq!(status.access_token_prefix, "sk-ant-oat-previ");
    }

    #[tokio::test]
    async fn logout_removes_file() {
        let (store, _dir) = temp_store();

        let tokens = TokenSet {
            access_token: "tok".into(),
            refresh_token: "ref".into(),
            expires_at: 9999999999,
        };
        store.save_tokens(&tokens).await.unwrap();
        assert!(store.token_path.exists());

        store.logout().unwrap();
        assert!(!store.token_path.exists());

        // Idempotent second call
        store.logout().unwrap();
    }

    #[test]
    fn start_login_flow_returns_auth_url() {
        let flow = AnthropicOAuthStore::start_login_flow(OAuthMode::Max).unwrap();
        assert!(!flow.authorization_url.is_empty());
        assert!(
            flow.authorization_url.contains("claude.ai"),
            "Max mode should use claude.ai: {}",
            flow.authorization_url
        );
        assert!(!flow.state.is_empty());
        assert!(!flow.verifier.is_empty());
    }

    #[test]
    fn start_login_flow_console_uses_console_domain() {
        let flow = AnthropicOAuthStore::start_login_flow(OAuthMode::Console).unwrap();
        assert!(
            flow.authorization_url.contains("console.anthropic.com"),
            "Console mode should use console.anthropic.com: {}",
            flow.authorization_url
        );
    }
}
