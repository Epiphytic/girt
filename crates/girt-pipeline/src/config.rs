use std::path::Path;
use std::sync::Arc;

use serde::Deserialize;

use crate::error::PipelineError;
use crate::llm::{LlmClient, OpenAiCompatibleClient, StubLlmClient};

#[derive(Debug, Deserialize)]
pub struct GirtConfig {
    pub llm: LlmConfig,
    #[serde(default)]
    pub registry: RegistryConfig,
    #[serde(default)]
    pub build: BuildConfig,
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
    pub fn from_file(path: &Path) -> Result<Self, PipelineError> {
        let content = std::fs::read_to_string(path).map_err(PipelineError::IoError)?;
        toml::from_str(&content).map_err(|e| {
            PipelineError::LlmError(format!("Failed to parse config: {e}"))
        })
    }

    pub fn build_llm_client(&self) -> Arc<dyn LlmClient> {
        match self.llm.provider {
            LlmProvider::OpenAiCompatible => {
                let api_key = std::env::var("GIRT_LLM_API_KEY")
                    .ok()
                    .or_else(|| self.llm.api_key.clone());
                Arc::new(OpenAiCompatibleClient::new(
                    self.llm.base_url.clone(),
                    self.llm.model.clone(),
                    api_key,
                ))
            }
            LlmProvider::Stub => Arc::new(StubLlmClient::constant("stub response")),
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
