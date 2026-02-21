use girt_core::spec::CapabilitySpec;

use crate::error::PipelineError;
use crate::llm::{LlmClient, LlmMessage, LlmRequest};
use crate::types::{RefinedSpec, SpecAction};

const ARCHITECT_SYSTEM_PROMPT: &str = r#"You are a Chief Software Architect specializing in tool design for sandboxed WebAssembly environments. You do not write implementation code.

PRIMARY PURPOSE:
The calling agent provided a high-level description of what they want. They should not have to think about implementation details. Your job is to translate their intent into a complete, secure, implementable specification — making all the decisions the caller shouldn't have to make. The caller's context is precious; own the complexity so they don't have to.

Your responsibilities:
- Decide which external APIs or protocols to use (e.g. Discord REST API, WASI HTTP)
- Add security requirements the caller didn't mention (input validation, injection prevention, resource limits)
- Define precise input/output schemas with validation constraints
- Specify safe defaults (timeouts, size limits, rate limits)
- Make implementation-guiding decisions the Engineer needs to write correct code

Design Principles:
1. SCOPE: Do not add features the caller did not request. If they asked for approval via Discord reactions, do not add a web dashboard. Scope creep is a defect.
2. COMPLETE SPEC: Within the requested scope, be thorough. Specify validation rules, error conditions, edge cases, and security requirements. The Engineer should not have to guess.
3. MINIMUM VIABLE TOOL: Prefer simple and correct over powerful and broken. One thing done well beats ten things done poorly.
4. SECURE BY DEFAULT: For any tool that handles credentials, makes network calls, or processes user input — proactively add: input validation, injection prevention, resource limits, and credential hygiene requirements. Do not wait for the caller to ask.
5. CONSISTENT API: snake_case fields, clear error strings, predictable shapes.
6. MINIMAL PERMISSIONS: Tighten constraints to exactly what the spec needs.

Output ONLY valid JSON in this exact format:
{
  "action": "build",
  "spec": {
    "name": "tool_name",
    "description": "Complete implementation spec: what the tool does, the exact API/protocol it uses, all validation requirements, all security constraints, and precise behaviour for edge cases. This description is the Engineer's only reference — make it complete.",
    "inputs": {},
    "outputs": {},
    "constraints": {
      "network": [],
      "storage": [],
      "secrets": []
    }
  },
  "complexity_hint": "low",
  "design_notes": "What decisions you made on behalf of the caller and why"
}

complexity_hint must be "low" or "high". Set "high" when the tool: makes outbound HTTP calls, handles user-supplied secrets or credentials, involves polling/timing loops, or processes multiple user-supplied string inputs that could be injection surfaces. Set "low" for pure computation with no I/O.

Do not include any text outside the JSON object."#;

/// The Architect agent refines a narrow capability request into a robust,
/// generic, reusable tool specification.
pub struct ArchitectAgent<'a> {
    llm: &'a dyn LlmClient,
}

impl<'a> ArchitectAgent<'a> {
    pub fn new(llm: &'a dyn LlmClient) -> Self {
        Self { llm }
    }

    pub async fn refine(&self, spec: &CapabilitySpec) -> Result<RefinedSpec, PipelineError> {
        let spec_json = serde_json::to_string_pretty(spec)
            .map_err(|e| PipelineError::LlmError(format!("Failed to serialize spec: {e}")))?;

        let request = LlmRequest {
            system_prompt: ARCHITECT_SYSTEM_PROMPT.into(),
            messages: vec![LlmMessage {
                role: "user".into(),
                content: format!(
                    "Refine this capability request into a robust tool spec:\n\n{spec_json}"
                ),
            }],
            max_tokens: 2000,
        };

        let response = self.llm.chat(&request).await?;

        // Parse the JSON response (handles code fences and surrounding text)
        let refined: RefinedSpec = super::extract_json(&response.content).ok_or_else(|| {
            tracing::warn!(
                raw_response = %response.content,
                "Architect response did not contain valid JSON, using original spec"
            );
            PipelineError::LlmError(format!(
                "Failed to parse architect response as JSON. Raw response: {}",
                &response.content[..response.content.len().min(200)]
            ))
        })?;

        tracing::info!(
            action = ?refined.action,
            name = %refined.spec.name,
            "Architect refined spec"
        );

        Ok(refined)
    }

    /// Fallback: if the Architect LLM call fails, pass through the original spec unrefined.
    pub fn passthrough(spec: &CapabilitySpec) -> RefinedSpec {
        RefinedSpec {
            action: SpecAction::Build,
            spec: spec.clone(),
            design_notes: "Unrefined spec (Architect unavailable)".into(),
            extend_target: None,
            extend_features: None,
            complexity_hint: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::StubLlmClient;
    use girt_core::spec::CapabilityConstraints;

    fn make_spec() -> CapabilitySpec {
        CapabilitySpec {
            name: "fetch_github_issues".into(),
            description: "Fetch open GitHub issues for a repo".into(),
            inputs: serde_json::json!({"repo": "string"}),
            outputs: serde_json::json!({"issues": "array"}),
            constraints: CapabilityConstraints {
                network: vec!["api.github.com".into()],
                storage: vec![],
                secrets: vec!["GITHUB_TOKEN".into()],
            },
        }
    }

    #[tokio::test]
    async fn refines_spec_from_llm_response() {
        let response = serde_json::json!({
            "action": "build",
            "spec": {
                "name": "github_issues",
                "description": "Query and manage GitHub issues with filtering and pagination",
                "inputs": {"repo": "string", "state": "string", "labels": "array"},
                "outputs": {"items": "array", "next_cursor": "string"},
                "constraints": {
                    "network": ["api.github.com"],
                    "storage": [],
                    "secrets": ["GITHUB_TOKEN"]
                }
            },
            "design_notes": "Generalized from single-repo fetch to full issue query tool"
        });

        let client = StubLlmClient::constant(&response.to_string());
        let agent = ArchitectAgent::new(&client);
        let spec = make_spec();

        let refined = agent.refine(&spec).await.unwrap();
        assert_eq!(refined.action, SpecAction::Build);
        assert_eq!(refined.spec.name, "github_issues");
        assert!(!refined.design_notes.is_empty());
    }

    #[tokio::test]
    async fn passthrough_preserves_original_spec() {
        let spec = make_spec();
        let refined = ArchitectAgent::passthrough(&spec);

        assert_eq!(refined.action, SpecAction::Build);
        assert_eq!(refined.spec.name, "fetch_github_issues");
    }
}
