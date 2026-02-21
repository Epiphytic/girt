use std::path::Path;
use std::sync::Arc;

use serde::Deserialize;

use crate::error::PipelineError;
use crate::llm::{AnthropicLlmClient, LlmClient, OpenAiCompatibleClient, StubLlmClient};

#[derive(Debug, Deserialize)]
pub struct GirtConfig {
    pub llm: LlmConfig,
    #[serde(default)]
    pub registry: RegistryConfig,
    #[serde(default)]
    pub build: BuildConfig,
    #[serde(default)]
    pub pipeline: PipelineConfig,
    #[serde(default)]
    pub security: SecurityConfig,
}

/// Security and gate configuration.
#[derive(Debug, Default, Deserialize)]
pub struct SecurityConfig {
    /// Controls Creation Gate evaluation mode:
    /// - `"llm"` (default): full LLM + HITL approval required
    /// - `"policy_only"`: policy rules enforced, LLM/HITL bypassed
    ///   **Use only for bootstrapping** — switch back to "llm" after.
    #[serde(default = "default_creation_gate")]
    pub creation_gate: String,
}

fn default_creation_gate() -> String {
    "llm".into()
}

/// Pipeline-level configuration.
#[derive(Debug, Deserialize)]
pub struct PipelineConfig {
    /// Path to a coding standards file (e.g. ~/.claude/CLAUDE.md).
    /// When set, the contents are injected into the Engineer's system prompt
    /// so generated code follows your project's conventions.
    pub coding_standards_path: Option<String>,

    /// Maximum Engineer → QA/RedTeam iterations before the circuit breaker
    /// triggers and the build is failed. Default: 3. Increase for complex
    /// security-sensitive components that legitimately need more passes.
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
}

fn default_max_iterations() -> u32 {
    3
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            coding_standards_path: None,
            max_iterations: default_max_iterations(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct LlmConfig {
    pub provider: LlmProvider,
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default = "default_model")]
    pub model: String,
    pub api_key: Option<String>,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
}

fn default_base_url() -> String {
    "http://localhost:8000/v1".into()
}
fn default_model() -> String {
    "zai-org/GLM-4.7-Flash".into()
}
fn default_max_tokens() -> u32 {
    4096
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub enum LlmProvider {
    #[serde(rename = "anthropic")]
    Anthropic,
    #[serde(rename = "openai-compatible")]
    OpenAiCompatible,
    #[serde(rename = "stub")]
    Stub,
}

#[derive(Debug, Default, Deserialize)]
pub struct RegistryConfig {
    #[serde(default = "default_registry_url")]
    pub url: String,
    pub token: Option<String>,
}

fn default_registry_url() -> String {
    "ghcr.io/epiphytic/girt-tools".into()
}

#[derive(Debug, Default, Deserialize)]
pub struct BuildConfig {
    #[serde(default = "default_language")]
    pub default_language: String,
    #[serde(default = "default_tier")]
    pub default_tier: String,
}

fn default_language() -> String {
    "rust".into()
}
fn default_tier() -> String {
    "standard".into()
}

impl GirtConfig {
    /// Load and return the coding standards content, if a path is configured.
    ///
    /// Expands `~` to the home directory. Returns `None` silently if the path
    /// isn't set or the file doesn't exist (non-fatal — standards are optional).
    pub fn load_coding_standards(&self) -> Option<String> {
        let raw = self.pipeline.coding_standards_path.as_deref()?;
        let expanded = if raw.starts_with('~') {
            dirs::home_dir()?.join(&raw[2..])
        } else {
            std::path::PathBuf::from(raw)
        };
        match std::fs::read_to_string(&expanded) {
            Ok(content) => {
                tracing::info!(path = %expanded.display(), "Loaded coding standards");
                Some(content)
            }
            Err(e) => {
                tracing::warn!(
                    path = %expanded.display(),
                    error = %e,
                    "Could not load coding standards — continuing without them"
                );
                None
            }
        }
    }

    pub fn from_file(path: &Path) -> Result<Self, PipelineError> {
        let content = std::fs::read_to_string(path).map_err(PipelineError::IoError)?;
        toml::from_str(&content).map_err(|e| {
            PipelineError::LlmError(format!("Failed to parse config: {e}"))
        })
    }

    pub fn build_llm_client(&self) -> Result<Arc<dyn LlmClient>, PipelineError> {
        match self.llm.provider {
            LlmProvider::Anthropic => {
                // from_env_or checks: ANTHROPIC_API_KEY → openclaw auth-profiles → api_key in toml
                let client = AnthropicLlmClient::from_env_or(
                    self.llm.model.clone(),
                    self.llm.api_key.clone(),
                )?;
                Ok(Arc::new(client))
            }
            LlmProvider::OpenAiCompatible => {
                let api_key = std::env::var("GIRT_LLM_API_KEY")
                    .ok()
                    .or_else(|| self.llm.api_key.clone());
                Ok(Arc::new(OpenAiCompatibleClient::new(
                    self.llm.base_url.clone(),
                    self.llm.model.clone(),
                    api_key,
                )))
            }
            LlmProvider::Stub => Ok(Arc::new(StubLlmClient::constant("stub response"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_config() {
        let toml_str = r#"
[llm]
provider = "openai-compatible"
base_url = "http://localhost:8000/v1"
model = "zai-org/GLM-4.7-Flash"
"#;
        let config: GirtConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.llm.provider, LlmProvider::OpenAiCompatible);
        assert_eq!(config.llm.base_url, "http://localhost:8000/v1");
        assert_eq!(config.llm.model, "zai-org/GLM-4.7-Flash");
        assert_eq!(config.llm.max_tokens, 4096);
    }

    #[test]
    fn parses_stub_provider() {
        let toml_str = r#"
[llm]
provider = "stub"
"#;
        let config: GirtConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.llm.provider, LlmProvider::Stub);
    }

    #[test]
    fn parses_anthropic_provider() {
        let toml_str = r#"
[llm]
provider = "anthropic"
model = "claude-sonnet-4-5"
api_key = "sk-ant-test"
"#;
        let config: GirtConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.llm.provider, LlmProvider::Anthropic);
        assert_eq!(config.llm.model, "claude-sonnet-4-5");
    }

    #[test]
    fn build_llm_client_stub_succeeds() {
        let toml_str = r#"[llm]
provider = "stub"
"#;
        let config: GirtConfig = toml::from_str(toml_str).unwrap();
        assert!(config.build_llm_client().is_ok());
    }

    #[test]
    fn build_llm_client_anthropic_with_inline_key_succeeds() {
        // An api_key in girt.toml is the last-resort fallback.
        // This always works regardless of env or openclaw config.
        let toml_str = r#"[llm]
provider = "anthropic"
model = "claude-sonnet-4-5"
api_key = "sk-ant-test-key"
"#;
        let config: GirtConfig = toml::from_str(toml_str).unwrap();
        assert!(config.build_llm_client().is_ok());
    }

    #[test]
    fn build_llm_client_anthropic_resolution_order() {
        // Credential resolution: ANTHROPIC_API_KEY > openclaw auth-profiles > api_key in toml.
        // If an explicit key is in toml it always wins as a final fallback.
        // We can't reliably test the "no credentials anywhere" case in CI
        // because a developer machine may have openclaw configured.
        let toml_str = r#"[llm]
provider = "anthropic"
model = "claude-sonnet-4-5"
api_key = "sk-ant-fallback"
"#;
        let config: GirtConfig = toml::from_str(toml_str).unwrap();
        // Should succeed via api_key fallback even with no env var
        assert!(config.build_llm_client().is_ok());
    }

    #[test]
    fn parses_full_config() {
        let toml_str = r#"
[llm]
provider = "openai-compatible"
base_url = "http://localhost:11434/v1"
model = "llama3"
api_key = "sk-test"
max_tokens = 8192

[registry]
url = "ghcr.io/epiphytic/girt-tools"

[build]
default_language = "rust"
default_tier = "standard"
"#;
        let config: GirtConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.llm.api_key, Some("sk-test".into()));
        assert_eq!(config.llm.max_tokens, 8192);
        assert_eq!(config.registry.url, "ghcr.io/epiphytic/girt-tools");
        assert_eq!(config.build.default_language, "rust");
    }
}
