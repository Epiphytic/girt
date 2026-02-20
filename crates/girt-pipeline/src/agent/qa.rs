use crate::error::PipelineError;
use crate::llm::{LlmClient, LlmMessage, LlmRequest};
use crate::types::{BugTicket, BugTicketType, BuildOutput, QaResult, RefinedSpec};

const QA_SYSTEM_PROMPT: &str = r#"You are a QA Automation Engineer. You are given a tool specification and its implementation.

Your objective is to verify functional correctness.

Generate test cases covering:
1. Standard use cases (happy path)
2. Edge cases (empty inputs, boundary values, unicode)
3. Malformed inputs (wrong types, missing fields, oversized payloads)

Output ONLY valid JSON:
{
  "passed": true/false,
  "tests_run": <number>,
  "tests_passed": <number>,
  "tests_failed": <number>,
  "bug_tickets": [
    {
      "target": "engineer",
      "ticket_type": "functional_defect",
      "input": <the failing input>,
      "expected": "what should happen",
      "actual": "what actually happened",
      "remediation_directive": "specific fix instruction"
    }
  ]
}

If all tests pass, set passed=true and bug_tickets=[].
Do not include any text outside the JSON object."#;

/// The QA agent verifies functional correctness of a built component.
pub struct QaAgent<'a> {
    llm: &'a dyn LlmClient,
}

impl<'a> QaAgent<'a> {
    pub fn new(llm: &'a dyn LlmClient) -> Self {
        Self { llm }
    }

    pub async fn test(
        &self,
        spec: &RefinedSpec,
        build: &BuildOutput,
    ) -> Result<QaResult, PipelineError> {
        let request = LlmRequest {
            system_prompt: QA_SYSTEM_PROMPT.into(),
            messages: vec![LlmMessage {
                role: "user".into(),
                content: format!(
                    "Spec:\n{}\n\nSource code:\n{}\n\nWIT:\n{}\n\nPolicy:\n{}",
                    serde_json::to_string_pretty(&spec.spec).unwrap_or_default(),
                    build.source_code,
                    build.wit_definition,
                    build.policy_yaml,
                ),
            }],
            max_tokens: 2000,
        };

        let response = self.llm.chat(&request).await?;

        let result: QaResult = match super::extract_json(&response.content) {
            Some(r) => r,
            None => {
                tracing::warn!(
                    raw_response = %response.content,
                    "QA response did not contain valid JSON, defaulting to fail"
                );
                QaResult {
                    passed: false,
                    tests_run: 0,
                    tests_passed: 0,
                    tests_failed: 0,
                    bug_tickets: vec![],
                }
            }
        };

        tracing::info!(
            passed = result.passed,
            tests_run = result.tests_run,
            tests_passed = result.tests_passed,
            tests_failed = result.tests_failed,
            bug_tickets = result.bug_tickets.len(),
            "QA testing complete"
        );

        Ok(result)
    }

    /// Create a passing QA result for testing.
    pub fn passing_result() -> QaResult {
        QaResult {
            passed: true,
            tests_run: 5,
            tests_passed: 5,
            tests_failed: 0,
            bug_tickets: vec![],
        }
    }

    /// Create a failing QA result with a bug ticket for testing.
    pub fn failing_result(directive: &str) -> QaResult {
        QaResult {
            passed: false,
            tests_run: 5,
            tests_passed: 3,
            tests_failed: 2,
            bug_tickets: vec![BugTicket {
                target: "engineer".into(),
                ticket_type: BugTicketType::FunctionalDefect,
                input: serde_json::json!({"test": "failing_input"}),
                expected: "correct output".into(),
                actual: "incorrect output".into(),
                remediation_directive: directive.into(),
            }],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::StubLlmClient;
    use crate::types::SpecAction;
    use girt_core::spec::{CapabilityConstraints, CapabilitySpec};

    fn make_test_context() -> (RefinedSpec, BuildOutput) {
        let spec = RefinedSpec {
            action: SpecAction::Build,
            spec: CapabilitySpec {
                name: "test_tool".into(),
                description: "A test tool".into(),
                inputs: serde_json::Value::Null,
                outputs: serde_json::Value::Null,
                constraints: CapabilityConstraints::default(),
            },
            design_notes: "test".into(),
            extend_target: None,
            extend_features: None,
        };
        let build = BuildOutput {
            source_code: "fn main() {}".into(),
            wit_definition: "package test:tool;".into(),
            policy_yaml: "version: \"1.0\"".into(),
            language: "rust".into(),
        };
        (spec, build)
    }

    #[tokio::test]
    async fn parses_passing_qa_result() {
        let response = serde_json::json!({
            "passed": true,
            "tests_run": 5,
            "tests_passed": 5,
            "tests_failed": 0,
            "bug_tickets": []
        });

        let client = StubLlmClient::constant(&response.to_string());
        let agent = QaAgent::new(&client);
        let (spec, build) = make_test_context();

        let result = agent.test(&spec, &build).await.unwrap();
        assert!(result.passed);
        assert_eq!(result.tests_run, 5);
        assert!(result.bug_tickets.is_empty());
    }

    #[tokio::test]
    async fn parses_failing_qa_result() {
        let response = serde_json::json!({
            "passed": false,
            "tests_run": 5,
            "tests_passed": 3,
            "tests_failed": 2,
            "bug_tickets": [{
                "target": "engineer",
                "ticket_type": "functional_defect",
                "input": {"value": -1},
                "expected": "error response",
                "actual": "panic",
                "remediation_directive": "Add bounds checking"
            }]
        });

        let client = StubLlmClient::constant(&response.to_string());
        let agent = QaAgent::new(&client);
        let (spec, build) = make_test_context();

        let result = agent.test(&spec, &build).await.unwrap();
        assert!(!result.passed);
        assert_eq!(result.bug_tickets.len(), 1);
        assert_eq!(
            result.bug_tickets[0].ticket_type,
            BugTicketType::FunctionalDefect
        );
    }
}
