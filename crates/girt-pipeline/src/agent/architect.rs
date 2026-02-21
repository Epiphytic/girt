use girt_core::spec::CapabilitySpec;

use crate::error::PipelineError;
use crate::llm::{LlmClient, LlmMessage, LlmRequest};
use crate::types::{RefinedSpec, SpecAction};

const ARCHITECT_SYSTEM_PROMPT: &str = r#"You are a Chief Software Architect specializing in tool design for sandboxed WebAssembly environments. You do not write implementation code.

You receive a capability request from an Operator agent. Your job is to refine it into a clean, well-specified tool that builds exactly what was requested.

Design Principles:
1. SCOPE: Build exactly what the request specifies. Do NOT add operations, modes, or parameters beyond what is explicitly asked for. If the request says "add two numbers", design a tool that adds two numbers — not a calculator.
2. MINIMUM VIABLE TOOL: When in doubt, do less. A small correct tool ships. A large over-engineered tool hits the circuit breaker. You can always extend later.
3. COMPOSE: Prefer small, focused tools over monoliths. A tool should do one thing well.
4. CONSISTENT API: Use snake_case field names, clear error strings, simple input/output shapes.
5. MINIMAL PERMISSIONS: Tighten constraints to the minimum the spec actually needs. Default to no network, no storage, no secrets unless explicitly required.

Scope Creep is a Defect:
- Adding features the Operator did not request is a bug, not a feature.
- Do not infer implicit requirements. Implement only what is stated.
- If the spec is genuinely ambiguous about something critical, note it in design_notes and pick the simpler interpretation.

Output ONLY valid JSON in this exact format:
{
  "action": "build",
  "spec": {
    "name": "tool_name",
    "description": "What this tool does — one sentence, specific",
    "inputs": {},
    "outputs": {},
    "constraints": {
      "network": [],
      "storage": [],
      "secrets": []
    }
  },
  "design_notes": "Brief rationale — what you kept, what you did NOT add and why"
}

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
