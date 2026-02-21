use crate::error::PipelineError;
use crate::llm::{LlmClient, LlmMessage, LlmRequest};
use crate::types::{ImplementationPlan, RefinedSpec};

const PLANNER_SYSTEM_PROMPT: &str = r#"You are a Senior Security Architect and Implementation Planner for sandboxed WebAssembly components. You do not write code. You produce implementation plans.

You receive a tool spec that has already been refined by an Architect. Your job is to think through the full implementation before any code is written. Be specific and exhaustive — the Engineer will treat your plan as the authoritative reference and must not deviate without documenting why.

Think through each of the following areas carefully:

1. VALIDATION LAYER
   For each input field: what must be validated before any external calls?
   Be specific: exact max lengths, allowed character sets, format requirements (e.g. "must be all digits 1-20 chars"), sanitization rules (strip CRLF, escape HTML entities, remove mention triggers).
   State the order of validation — fail fast on cheap checks before expensive ones.

2. SECURITY NOTES (Threat Model)
   For each input field: what can a malicious caller do?
   Cover: CRLF/header injection (for HTTP-touching fields), path traversal (for fields used in URLs), resource exhaustion (unbounded loops, missing timeouts, oversized payloads), identity spoofing (username vs user ID), prompt injection (if any field ends up in LLM context).
   State the specific mitigation for each threat.

3. API SEQUENCE
   List every external call in order. For each:
   - Exact endpoint and HTTP method
   - Which inputs map to which request fields (and how they're encoded)
   - What the success response looks like (status code, key fields)
   - What error cases are possible and how each is handled
   - Any polling logic: how long to sleep between polls, how to honor the timeout, termination condition

4. EDGE CASES
   Document the required behavior for:
   - Empty or minimal inputs (empty lists, zero values, blank strings)
   - Maximum / boundary values (longest allowed strings, largest allowed numbers)
   - Timeout scenarios: what happens when the deadline expires mid-operation
   - Partial failure: what to return if some calls succeed and others fail
   - Concurrent callers (if the component might be called multiple times simultaneously)

5. IMPLEMENTATION GUIDANCE
   WASM+WASI-specific patterns:
   - Which crates work well (wasi, wasi-http, serde_json, etc.) and which to avoid
   - How to structure the sleep/poll loop without blocking the WASM runtime
   - How to handle WASI HTTP response body reading (streaming vs buffered)
   - What NOT to use (std::thread, tokio, async/await at the top level, etc.)
   - Any encoding/escaping specifics (percent-encoding, JSON escaping, etc.)

Output ONLY valid JSON in this exact format:
{
  "validation_layer": "...",
  "security_notes": "...",
  "api_sequence": "...",
  "edge_cases": "...",
  "implementation_guidance": "..."
}

Each field must be a single string (use \\n for newlines within the string). Be thorough — the Engineer has no other reference. Do not include any text outside the JSON object."#;

/// The Planner agent produces a structured implementation brief for complex tools.
///
/// Runs between the Architect and Engineer when the spec meets complexity
/// triggers (network calls, secrets, user-supplied string inputs, async/polling).
/// The Engineer receives both the refined spec and the plan, and must follow
/// the plan's validation strategy and API sequence.
pub struct PlannerAgent<'a> {
    llm: &'a dyn LlmClient,
}

impl<'a> PlannerAgent<'a> {
    pub fn new(llm: &'a dyn LlmClient) -> Self {
        Self { llm }
    }

    /// Produce an implementation plan for the given refined spec.
    pub async fn plan(&self, spec: &RefinedSpec) -> Result<ImplementationPlan, PipelineError> {
        let spec_json = serde_json::to_string_pretty(spec)
            .map_err(|e| PipelineError::LlmError(format!("Failed to serialize spec: {e}")))?;

        let request = LlmRequest {
            system_prompt: PLANNER_SYSTEM_PROMPT.into(),
            messages: vec![LlmMessage {
                role: "user".into(),
                content: format!(
                    "Produce an implementation plan for this tool spec:\n\n{spec_json}"
                ),
            }],
            max_tokens: 4000,
        };

        let response = self.llm.chat(&request).await?;

        let plan: ImplementationPlan =
            super::extract_json(&response.content).ok_or_else(|| {
                tracing::warn!(
                    raw_response = %response.content,
                    "Planner response did not contain valid JSON"
                );
                PipelineError::LlmError(format!(
                    "Failed to parse planner response as JSON. Raw: {}",
                    &response.content[..response.content.len().min(200)]
                ))
            })?;

        tracing::info!(
            spec_name = %spec.spec.name,
            "Planner produced implementation plan"
        );
        Ok(plan)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::StubLlmClient;
    use crate::types::SpecAction;
    use girt_core::spec::{CapabilityConstraints, CapabilitySpec};

    fn make_refined_spec() -> RefinedSpec {
        RefinedSpec {
            action: SpecAction::Build,
            spec: CapabilitySpec {
                name: "discord_approval".into(),
                description: "Post a Discord message and wait for reaction approval".into(),
                inputs: serde_json::json!({
                    "channel_id": "string",
                    "question": "string",
                    "timeout_secs": "u32"
                }),
                outputs: serde_json::json!({
                    "approved": "bool",
                    "responder": "string"
                }),
                constraints: CapabilityConstraints {
                    network: vec!["discord.com".into()],
                    storage: vec![],
                    secrets: vec!["DISCORD_BOT_TOKEN".into()],
                },
            },
            design_notes: "Polls Discord reactions".into(),
            extend_target: None,
            extend_features: None,
            complexity_hint: None,
        }
    }

    #[tokio::test]
    async fn parses_plan_from_valid_json_response() {
        let response = serde_json::json!({
            "validation_layer": "Validate channel_id is all digits, 1-20 chars. Cap timeout_secs at 3600, minimum 30.",
            "security_notes": "channel_id: must not contain slashes (URL injection). bot_token: strip CRLF before use in headers.",
            "api_sequence": "1. POST /channels/{channel_id}/messages. 2. Poll /channels/{channel_id}/messages/{msg_id}/reactions.",
            "edge_cases": "Empty authorized_users: accept first respondent. Timeout expires: return approved=false.",
            "implementation_guidance": "Use wasi-http for HTTP. No tokio. Sleep in a loop with min(10s, remaining)."
        });

        let client = StubLlmClient::constant(&response.to_string());
        let agent = PlannerAgent::new(&client);
        let spec = make_refined_spec();

        let plan = agent.plan(&spec).await.unwrap();
        assert!(plan.validation_layer.contains("channel_id"));
        assert!(plan.security_notes.contains("CRLF"));
        assert!(plan.api_sequence.contains("POST"));
        assert!(!plan.edge_cases.is_empty());
        assert!(!plan.implementation_guidance.is_empty());
    }

    #[tokio::test]
    async fn returns_error_on_non_json_response() {
        let client = StubLlmClient::constant("I cannot produce a plan for this spec.");
        let agent = PlannerAgent::new(&client);
        let spec = make_refined_spec();

        let result = agent.plan(&spec).await;
        assert!(result.is_err());
        assert!(matches!(result, Err(PipelineError::LlmError(_))));
    }

    #[tokio::test]
    async fn parses_plan_from_code_fence_response() {
        let response = "Here is the plan:\n```json\n{\"validation_layer\":\"validate inputs\",\"security_notes\":\"check injection\",\"api_sequence\":\"call discord\",\"edge_cases\":\"handle timeout\",\"implementation_guidance\":\"use wasi-http\"}\n```";

        let client = StubLlmClient::constant(response);
        let agent = PlannerAgent::new(&client);
        let spec = make_refined_spec();

        let plan = agent.plan(&spec).await.unwrap();
        assert_eq!(plan.validation_layer, "validate inputs");
    }
}
