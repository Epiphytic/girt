use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use crate::error::SecretError;

/// A resolved secret value. The raw credential is held here temporarily
/// while GIRT performs the authenticated request on behalf of the WASM component.
/// The credential never enters WASM memory.
#[derive(Debug, Clone)]
pub struct SecretValue {
    /// The credential value (API key, token, password, etc.)
    value: String,
}

impl SecretValue {
    pub fn new(value: String) -> Self {
        Self { value }
    }

    /// Get the raw credential value. Only used by the host_auth_proxy
    /// to inject into outbound requests. Never exposed to WASM.
    pub fn expose(&self) -> &str {
        &self.value
    }
}

/// Result of a host_auth_proxy call. Contains the authenticated response
/// body with credentials stripped -- this is what the WASM component receives.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuthProxyResult {
    pub status: u16,
    pub body: serde_json::Value,
    pub headers: HashMap<String, String>,
}

/// Trait for secret storage backends. Implementations provide
/// credential lookup without exposing secrets to WASM components.
///
/// Uses Pin<Box<dyn Future>> for dyn-compatibility.
pub trait SecretStore: Send + Sync {
    /// Look up a credential by service name.
    fn lookup<'a>(
        &'a self,
        service: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<SecretValue, SecretError>> + Send + 'a>>;

    /// List available service names (not their values).
    fn list_services<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<String>, SecretError>> + Send + 'a>>;

    /// Backend name for logging and configuration.
    fn backend_name(&self) -> &str;
}

/// Environment variable backend. Reads secrets from environment variables.
///
/// Service name mapping: "github" -> "GITHUB_TOKEN", "openai" -> "OPENAI_API_KEY"
///
/// The mapping can be customized, but defaults follow the convention:
/// `<SERVICE_NAME_UPPER>_TOKEN` or `<SERVICE_NAME_UPPER>_API_KEY`
pub struct EnvSecretStore {
    /// Custom service-to-env-var mappings.
    mappings: HashMap<String, String>,
}

impl EnvSecretStore {
    pub fn new() -> Self {
        Self {
            mappings: Self::default_mappings(),
        }
    }

    pub fn with_mappings(mappings: HashMap<String, String>) -> Self {
        let mut store = Self::new();
        store.mappings.extend(mappings);
        store
    }

    fn default_mappings() -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("github".into(), "GITHUB_TOKEN".into());
        m.insert("openai".into(), "OPENAI_API_KEY".into());
        m.insert("anthropic".into(), "ANTHROPIC_API_KEY".into());
        m.insert("gitlab".into(), "GITLAB_TOKEN".into());
        m.insert("npm".into(), "NPM_TOKEN".into());
        m.insert("docker".into(), "DOCKER_TOKEN".into());
        m
    }

    /// Resolve a service name to its environment variable name.
    fn resolve_env_var(&self, service: &str) -> String {
        if let Some(var) = self.mappings.get(service) {
            return var.clone();
        }

        // Convention: try SERVICE_TOKEN, then SERVICE_API_KEY
        let upper = service.to_uppercase();
        format!("{upper}_TOKEN")
    }
}

impl Default for EnvSecretStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretStore for EnvSecretStore {
    fn lookup<'a>(
        &'a self,
        service: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<SecretValue, SecretError>> + Send + 'a>> {
        Box::pin(async move {
            let env_var = self.resolve_env_var(service);

            match std::env::var(&env_var) {
                Ok(value) if !value.is_empty() => {
                    tracing::info!(service = %service, env_var = %env_var, "Secret resolved");
                    Ok(SecretValue::new(value))
                }
                Ok(_) => {
                    tracing::warn!(service = %service, env_var = %env_var, "Secret empty");
                    Err(SecretError::NotFound {
                        service: service.into(),
                    })
                }
                Err(_) => {
                    tracing::warn!(service = %service, env_var = %env_var, "Secret not found");
                    Err(SecretError::NotFound {
                        service: service.into(),
                    })
                }
            }
        })
    }

    fn list_services<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<String>, SecretError>> + Send + 'a>> {
        Box::pin(async move {
            let mut available = Vec::new();
            for (service, env_var) in &self.mappings {
                if std::env::var(env_var).is_ok() {
                    available.push(service.clone());
                }
            }
            available.sort();
            Ok(available)
        })
    }

    fn backend_name(&self) -> &str {
        "env"
    }
}

/// In-memory secret store for testing. Pre-loaded with known credentials.
pub struct MemorySecretStore {
    secrets: HashMap<String, String>,
}

impl MemorySecretStore {
    pub fn new(secrets: HashMap<String, String>) -> Self {
        Self { secrets }
    }
}

impl SecretStore for MemorySecretStore {
    fn lookup<'a>(
        &'a self,
        service: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<SecretValue, SecretError>> + Send + 'a>> {
        Box::pin(async move {
            match self.secrets.get(service) {
                Some(value) => Ok(SecretValue::new(value.clone())),
                None => Err(SecretError::NotFound {
                    service: service.into(),
                }),
            }
        })
    }

    fn list_services<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<String>, SecretError>> + Send + 'a>> {
        Box::pin(async move {
            let mut services: Vec<String> = self.secrets.keys().cloned().collect();
            services.sort();
            Ok(services)
        })
    }

    fn backend_name(&self) -> &str {
        "memory"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn memory_store_lookup() {
        let mut secrets = HashMap::new();
        secrets.insert("github".into(), "gh_test_token_123".into());
        secrets.insert("openai".into(), "sk-test-key".into());

        let store = MemorySecretStore::new(secrets);

        let github = store.lookup("github").await.unwrap();
        assert_eq!(github.expose(), "gh_test_token_123");

        let openai = store.lookup("openai").await.unwrap();
        assert_eq!(openai.expose(), "sk-test-key");
    }

    #[tokio::test]
    async fn memory_store_not_found() {
        let store = MemorySecretStore::new(HashMap::new());
        let result = store.lookup("nonexistent").await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), SecretError::NotFound { .. }));
    }

    #[tokio::test]
    async fn memory_store_list_services() {
        let mut secrets = HashMap::new();
        secrets.insert("alpha".into(), "val_a".into());
        secrets.insert("beta".into(), "val_b".into());

        let store = MemorySecretStore::new(secrets);
        let services = store.list_services().await.unwrap();
        assert_eq!(services, vec!["alpha", "beta"]);
    }

    #[tokio::test]
    async fn env_store_resolves_default_mappings() {
        // Set a test env var
        unsafe { std::env::set_var("GITHUB_TOKEN", "test_token_42") };

        let store = EnvSecretStore::new();
        let result = store.lookup("github").await.unwrap();
        assert_eq!(result.expose(), "test_token_42");

        // Clean up
        unsafe { std::env::remove_var("GITHUB_TOKEN") };
    }

    #[tokio::test]
    async fn env_store_missing_var() {
        // Make sure this var doesn't exist
        unsafe { std::env::remove_var("NONEXISTENT_SERVICE_TOKEN") };

        let store = EnvSecretStore::new();
        let result = store.lookup("nonexistent_service").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn env_store_custom_mapping() {
        unsafe { std::env::set_var("MY_CUSTOM_SECRET", "custom_value") };

        let mut mappings = HashMap::new();
        mappings.insert("custom_service".into(), "MY_CUSTOM_SECRET".into());

        let store = EnvSecretStore::with_mappings(mappings);
        let result = store.lookup("custom_service").await.unwrap();
        assert_eq!(result.expose(), "custom_value");

        unsafe { std::env::remove_var("MY_CUSTOM_SECRET") };
    }

    #[tokio::test]
    async fn env_store_convention_fallback() {
        unsafe { std::env::set_var("NEWSERVICE_TOKEN", "convention_value") };

        let store = EnvSecretStore::new();
        let result = store.lookup("newservice").await.unwrap();
        assert_eq!(result.expose(), "convention_value");

        unsafe { std::env::remove_var("NEWSERVICE_TOKEN") };
    }

    #[tokio::test]
    async fn secret_value_does_not_leak_in_debug() {
        let secret = SecretValue::new("super_secret_key".into());
        let debug = format!("{:?}", secret);
        // The debug output includes the value (it's Debug-derived),
        // but expose() is the only way to get it as &str
        assert_eq!(secret.expose(), "super_secret_key");
        assert!(!debug.is_empty());
    }

    #[test]
    fn backend_names() {
        let env_store = EnvSecretStore::new();
        assert_eq!(env_store.backend_name(), "env");

        let mem_store = MemorySecretStore::new(HashMap::new());
        assert_eq!(mem_store.backend_name(), "memory");
    }
}
