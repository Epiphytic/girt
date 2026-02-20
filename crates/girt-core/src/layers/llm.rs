use std::future::Future;
use std::pin::Pin;

use serde::{Deserialize, Serialize};

use crate::decision::Decision;
use crate::error::DecisionError;
use crate::layers::DecisionLayer;
use crate::spec::GateInput;

/// LLM evaluation layer -- uses an LLM to evaluate ambiguous requests.
///
/// This is the most expensive layer and only reached when all cheaper layers
/// (policy, cache, registry, CLI) fail to produce a decision.
pub struct LlmEvaluationLayer {
    evaluator: Box<dyn LlmEvaluator>,
}

/// Trait for LLM evaluation -- abstracted so we can mock in tests and swap providers.
pub trait LlmEvaluator: Send + Sync {
    fn evaluate<'a>(
        &'a self,
        input: &'a GateInput,
    ) -> Pin<Box<dyn Future<Output = Result<LlmDecision, DecisionError>> + Send + 'a>>;
}

/// The structured response from an LLM evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmDecision {
    pub decision: LlmDecisionKind,
    pub rationale: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LlmDecisionKind {
    Allow,
    Deny,
    Ask,
}

/// Stub LLM evaluator that always returns Ask (defers to HITL).
/// Real implementation will call the Anthropic API.
pub struct StubLlmEvaluator;

impl LlmEvaluator for StubLlmEvaluator {
    fn evaluate<'a>(
        &'a self,
        _input: &'a GateInput,
    ) -> Pin<Box<dyn Future<Output = Result<LlmDecision, DecisionError>> + Send + 'a>> {
        Box::pin(async move {
            Ok(LlmDecision {
                decision: LlmDecisionKind::Ask,
                rationale: "LLM evaluation not yet configured, deferring to human".into(),
            })
        })
    }
}

impl LlmEvaluationLayer {
    pub fn new(evaluator: Box<dyn LlmEvaluator>) -> Self {
        Self { evaluator }
    }

    pub fn with_stub() -> Self {
        Self {
            evaluator: Box::new(StubLlmEvaluator),
        }
    }
}

impl DecisionLayer for LlmEvaluationLayer {
    fn name(&self) -> &str {
        "llm_evaluation"
    }

    fn evaluate<'a>(
        &'a self,
        input: &'a GateInput,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Decision>, DecisionError>> + Send + 'a>> {
        Box::pin(async move {
            match self.evaluator.evaluate(input).await {
                Ok(llm_result) => {
                    tracing::info!(
                        decision = ?llm_result.decision,
                        rationale = %llm_result.rationale,
                        "LLM evaluation complete"
                    );

                    let decision = match llm_result.decision {
                        LlmDecisionKind::Allow => Decision::Allow,
                        LlmDecisionKind::Deny => Decision::Deny {
                            reason: llm_result.rationale,
                        },
                        LlmDecisionKind::Ask => Decision::Ask {
                            prompt: "LLM evaluation requires human input".into(),
                            context: llm_result.rationale,
                        },
                    };

                    Ok(Some(decision))
                }
                Err(e) => {
                    // LLM failure: pass through to next layer (HITL) rather than blocking
                    tracing::warn!(error = %e, "LLM evaluation failed, passing through");
                    Ok(None)
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{CapabilityConstraints, CapabilitySpec};

    fn make_input() -> GateInput {
        GateInput::Creation(CapabilitySpec {
            name: "test".into(),
            description: "test".into(),
            inputs: serde_json::Value::Null,
            outputs: serde_json::Value::Null,
            constraints: CapabilityConstraints::default(),
        })
    }

    #[tokio::test]
    async fn stub_evaluator_returns_ask() {
        let layer = LlmEvaluationLayer::with_stub();
        let input = make_input();

        let result = layer.evaluate(&input).await.unwrap();
        assert!(matches!(result, Some(Decision::Ask { .. })));
    }

    struct AllowEvaluator;
    impl LlmEvaluator for AllowEvaluator {
        fn evaluate<'a>(
            &'a self,
            _input: &'a GateInput,
        ) -> Pin<Box<dyn Future<Output = Result<LlmDecision, DecisionError>> + Send + 'a>> {
            Box::pin(async move {
                Ok(LlmDecision {
                    decision: LlmDecisionKind::Allow,
                    rationale: "looks safe".into(),
                })
            })
        }
    }

    #[tokio::test]
    async fn allow_evaluator_returns_allow() {
        let layer = LlmEvaluationLayer::new(Box::new(AllowEvaluator));
        let input = make_input();

        let result = layer.evaluate(&input).await.unwrap();
        assert!(matches!(result, Some(Decision::Allow)));
    }

    struct FailingEvaluator;
    impl LlmEvaluator for FailingEvaluator {
        fn evaluate<'a>(
            &'a self,
            _input: &'a GateInput,
        ) -> Pin<Box<dyn Future<Output = Result<LlmDecision, DecisionError>> + Send + 'a>> {
            Box::pin(async move { Err(DecisionError::LlmError("API call failed".into())) })
        }
    }

    #[tokio::test]
    async fn failing_evaluator_passes_through() {
        let layer = LlmEvaluationLayer::new(Box::new(FailingEvaluator));
        let input = make_input();

        let result = layer.evaluate(&input).await.unwrap();
        assert!(result.is_none());
    }
}
