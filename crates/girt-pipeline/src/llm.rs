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
}
