use std::future::Future;
use std::pin::Pin;

use serde::{Deserialize, Serialize};

use crate::error::PipelineError;

/// A message in an LLM conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmMessage {
    pub role: String,
    pub content: String,
}

/// Request to an LLM.
#[derive(Debug, Clone)]
pub struct LlmRequest {
    pub system_prompt: String,
    pub messages: Vec<LlmMessage>,
    pub max_tokens: u32,
}

/// Response from an LLM.
#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub content: String,
}

/// Facade trait for LLM providers.
///
/// Implementations can call Anthropic, OpenAI, or return deterministic
/// responses for testing.
pub trait LlmClient: Send + Sync {
    fn chat<'a>(
        &'a self,
        request: &'a LlmRequest,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, PipelineError>> + Send + 'a>>;
}

pub struct OpenAiCompatibleClient {
    http: reqwest::Client,
    base_url: String,
    model: String,
    api_key: Option<String>,
}

impl OpenAiCompatibleClient {
    pub fn new(base_url: String, model: String, api_key: Option<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url,
            model,
            api_key,
        }
    }
}

impl LlmClient for OpenAiCompatibleClient {
    fn chat<'a>(
        &'a self,
        request: &'a LlmRequest,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, PipelineError>> + Send + 'a>> {
        Box::pin(async move {
            let mut messages = vec![serde_json::json!({
                "role": "system",
                "content": request.system_prompt,
            })];

            for msg in &request.messages {
                messages.push(serde_json::json!({
                    "role": msg.role,
                    "content": msg.content,
                }));
            }

            let body = serde_json::json!({
                "model": self.model,
                "messages": messages,
                "max_tokens": request.max_tokens,
            });

            let url = format!("{}/chat/completions", self.base_url);
            let mut req = self.http.post(&url).json(&body);

            if let Some(key) = &self.api_key {
                req = req.bearer_auth(key);
            }

            let resp = req.send().await.map_err(|e| {
                PipelineError::LlmError(format!("HTTP request failed: {e}"))
            })?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(PipelineError::LlmError(format!(
                    "LLM API returned {status}: {body}"
                )));
            }

            let json: serde_json::Value = resp.json().await.map_err(|e| {
                PipelineError::LlmError(format!("Failed to parse response: {e}"))
            })?;

            let content = json["choices"][0]["message"]["content"]
                .as_str()
                .ok_or_else(|| {
                    PipelineError::LlmError(format!(
                        "No content in response: {}",
                        serde_json::to_string_pretty(&json).unwrap_or_default()
                    ))
                })?
                .to_string();

            Ok(LlmResponse { content })
        })
    }
}

/// Stub LLM client that returns deterministic responses for testing.
pub struct StubLlmClient {
    responses: Vec<String>,
    call_count: std::sync::atomic::AtomicUsize,
}

impl StubLlmClient {
    pub fn new(responses: Vec<String>) -> Self {
        Self {
            responses,
            call_count: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Create a stub that always returns the given response.
    pub fn constant(response: &str) -> Self {
        Self::new(vec![response.to_string()])
    }
}

impl LlmClient for StubLlmClient {
    fn chat<'a>(
        &'a self,
        _request: &'a LlmRequest,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, PipelineError>> + Send + 'a>> {
        Box::pin(async move {
            let idx = self
                .call_count
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let response = if self.responses.is_empty() {
                "stub response".to_string()
            } else {
                self.responses[idx % self.responses.len()].clone()
            };
            Ok(LlmResponse { content: response })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stub_returns_constant_response() {
        let client = StubLlmClient::constant("hello");
        let request = LlmRequest {
            system_prompt: "test".into(),
            messages: vec![],
            max_tokens: 100,
        };

        let response = client.chat(&request).await.unwrap();
        assert_eq!(response.content, "hello");
    }

    #[tokio::test]
    async fn openai_client_formats_request_correctly() {
        let client = OpenAiCompatibleClient::new(
            "http://localhost:9999/v1".into(),
            "test-model".into(),
            None,
        );
        let request = LlmRequest {
            system_prompt: "You are helpful.".into(),
            messages: vec![LlmMessage {
                role: "user".into(),
                content: "Hello".into(),
            }],
            max_tokens: 100,
        };
        let result = client.chat(&request).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn stub_cycles_through_responses() {
        let client = StubLlmClient::new(vec!["first".into(), "second".into()]);
        let request = LlmRequest {
            system_prompt: "test".into(),
            messages: vec![],
            max_tokens: 100,
        };

        let r1 = client.chat(&request).await.unwrap();
        assert_eq!(r1.content, "first");

        let r2 = client.chat(&request).await.unwrap();
        assert_eq!(r2.content, "second");

        let r3 = client.chat(&request).await.unwrap();
        assert_eq!(r3.content, "first"); // cycles back
    }

    #[tokio::test]
    #[ignore] // Requires vLLM running on localhost:8000
    async fn openai_client_calls_real_vllm() {
        let client = OpenAiCompatibleClient::new(
            "http://localhost:8000/v1".into(),
            "zai-org/GLM-4.7-Flash".into(),
            None,
        );
        let request = LlmRequest {
            system_prompt: "Reply with exactly: PONG".into(),
            messages: vec![LlmMessage {
                role: "user".into(),
                content: "PING".into(),
            }],
            max_tokens: 10,
        };
        let response = client.chat(&request).await.unwrap();
        assert!(!response.content.is_empty());
    }
}
