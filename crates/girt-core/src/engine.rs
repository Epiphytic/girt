use crate::decision::{Decision, DecisionLayer as DecisionLayerEnum, GateKind, LayeredDecision};
use crate::error::DecisionError;
use crate::layers::DecisionLayer;
use crate::layers::cache::CacheLayer;
use crate::layers::cli_check::CliCheckLayer;
use crate::layers::hitl::HitlLayer;
use crate::layers::llm::LlmEvaluationLayer;
use crate::layers::policy::PolicyRulesLayer;
use crate::layers::registry::RegistryLookupLayer;
use crate::spec::GateInput;

/// The Hookwise decision engine -- orchestrates the cascade of layers.
///
/// Each gate (Creation, Execution) evaluates a request through progressively
/// more expensive layers, short-circuiting as soon as a confident decision is reached.
pub struct DecisionEngine {
    creation_layers: CreationLayers,
    execution_layers: ExecutionLayers,
}

/// Layers for the Creation Gate ("Should this tool be built?")
pub struct CreationLayers {
    pub policy: PolicyRulesLayer,
    pub cache: CacheLayer,
    pub registry: RegistryLookupLayer,
    pub cli_check: CliCheckLayer,
    pub llm: LlmEvaluationLayer,
    pub hitl: HitlLayer,
}

/// Layers for the Execution Gate ("Should this invocation proceed?")
pub struct ExecutionLayers {
    pub policy: PolicyRulesLayer,
    pub cache: CacheLayer,
    pub llm: LlmEvaluationLayer,
    pub hitl: HitlLayer,
}

impl DecisionEngine {
    pub fn new(creation_layers: CreationLayers, execution_layers: ExecutionLayers) -> Self {
        Self {
            creation_layers,
            execution_layers,
        }
    }

    /// Create an engine with default/stub layers for development and testing.
    pub fn with_defaults() -> Self {
        Self {
            creation_layers: CreationLayers {
                policy: PolicyRulesLayer::with_defaults(),
                cache: CacheLayer::new(),
                registry: RegistryLookupLayer::new(vec![]),
                cli_check: CliCheckLayer::with_defaults(),
                llm: LlmEvaluationLayer::with_stub(),
                hitl: HitlLayer::with_default(),
            },
            execution_layers: ExecutionLayers {
                policy: PolicyRulesLayer::with_defaults(),
                cache: CacheLayer::new(),
                llm: LlmEvaluationLayer::with_stub(),
                hitl: HitlLayer::with_default(),
            },
        }
    }

    /// Evaluate a request through the appropriate gate cascade.
    pub async fn evaluate(
        &self,
        gate: GateKind,
        input: &GateInput,
    ) -> Result<LayeredDecision, DecisionError> {
        match gate {
            GateKind::Creation => self.evaluate_creation(input).await,
            GateKind::Execution => self.evaluate_execution(input).await,
        }
    }

    /// Access the creation cache for storing decisions after the fact.
    pub fn creation_cache(&self) -> &CacheLayer {
        &self.creation_layers.cache
    }

    /// Access the execution cache for storing decisions after the fact.
    pub fn execution_cache(&self) -> &CacheLayer {
        &self.execution_layers.cache
    }

    async fn evaluate_creation(&self, input: &GateInput) -> Result<LayeredDecision, DecisionError> {
        let layers: Vec<(&dyn DecisionLayer, DecisionLayerEnum)> = vec![
            (&self.creation_layers.policy, DecisionLayerEnum::PolicyRules),
            (&self.creation_layers.cache, DecisionLayerEnum::Cache),
            (
                &self.creation_layers.registry,
                DecisionLayerEnum::RegistryLookup,
            ),
            (&self.creation_layers.cli_check, DecisionLayerEnum::CliCheck),
            (&self.creation_layers.llm, DecisionLayerEnum::LlmEvaluation),
            (&self.creation_layers.hitl, DecisionLayerEnum::Hitl),
        ];

        self.run_cascade(&layers, input, GateKind::Creation).await
    }

    async fn evaluate_execution(
        &self,
        input: &GateInput,
    ) -> Result<LayeredDecision, DecisionError> {
        let layers: Vec<(&dyn DecisionLayer, DecisionLayerEnum)> = vec![
            (
                &self.execution_layers.policy,
                DecisionLayerEnum::PolicyRules,
            ),
            (&self.execution_layers.cache, DecisionLayerEnum::Cache),
            (&self.execution_layers.llm, DecisionLayerEnum::LlmEvaluation),
            (&self.execution_layers.hitl, DecisionLayerEnum::Hitl),
        ];

        self.run_cascade(&layers, input, GateKind::Execution).await
    }

    async fn run_cascade(
        &self,
        layers: &[(&dyn DecisionLayer, DecisionLayerEnum)],
        input: &GateInput,
        gate: GateKind,
    ) -> Result<LayeredDecision, DecisionError> {
        for (layer, layer_enum) in layers {
            tracing::debug!(
                gate = %gate,
                layer = layer.name(),
                "Evaluating layer"
            );

            match layer.evaluate(input).await {
                Ok(Some(decision)) => {
                    tracing::info!(
                        gate = %gate,
                        layer = layer.name(),
                        decision = ?decision,
                        "Layer produced decision"
                    );

                    let result = LayeredDecision {
                        decision: decision.clone(),
                        layer: layer_enum.clone(),
                        rationale: None,
                    };

                    // Cache terminal decisions for future lookups
                    if decision.is_terminal() {
                        let hash = input.hash();
                        let cache = match gate {
                            GateKind::Creation => &self.creation_layers.cache,
                            GateKind::Execution => &self.execution_layers.cache,
                        };
                        cache.store(hash, decision).await;
                    }

                    return Ok(result);
                }
                Ok(None) => {
                    tracing::debug!(
                        gate = %gate,
                        layer = layer.name(),
                        "Layer passed through"
                    );
                }
                Err(e) => {
                    // Layer error: log and continue to next layer (fail-open within cascade)
                    tracing::error!(
                        gate = %gate,
                        layer = layer.name(),
                        error = %e,
                        "Layer error, skipping"
                    );
                }
            }
        }

        // All layers exhausted without a decision. This should not happen
        // because HITL is always the last layer and always produces a decision.
        // But if somehow it does, deny by default.
        tracing::error!(
            gate = %gate,
            "All cascade layers exhausted without a decision, defaulting to deny"
        );
        Ok(LayeredDecision {
            decision: Decision::Deny {
                reason: "All cascade layers exhausted without producing a decision".into(),
            },
            layer: DecisionLayerEnum::Hitl,
            rationale: Some("Fallback deny: no layer produced a decision".into()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{CapabilityConstraints, CapabilitySpec, ExecutionRequest};

    fn make_creation_input(name: &str, desc: &str) -> GateInput {
        GateInput::Creation(CapabilitySpec {
            name: name.into(),
            description: desc.into(),
            inputs: serde_json::Value::Null,
            outputs: serde_json::Value::Null,
            constraints: CapabilityConstraints::default(),
        })
    }

    fn make_execution_input(name: &str) -> GateInput {
        GateInput::Execution(ExecutionRequest {
            tool_name: name.into(),
            arguments: serde_json::Value::Null,
        })
    }

    #[tokio::test]
    async fn creation_gate_denies_shell_exec() {
        let engine = DecisionEngine::with_defaults();
        let input = make_creation_input("shell_exec", "Run shell commands");

        let result = engine.evaluate(GateKind::Creation, &input).await.unwrap();

        assert!(matches!(result.decision, Decision::Deny { .. }));
        assert_eq!(result.layer, DecisionLayerEnum::PolicyRules);
    }

    #[tokio::test]
    async fn creation_gate_allows_math() {
        let engine = DecisionEngine::with_defaults();
        let input = make_creation_input("math_add", "Add two numbers");

        let result = engine.evaluate(GateKind::Creation, &input).await.unwrap();

        assert!(matches!(result.decision, Decision::Allow));
        assert_eq!(result.layer, DecisionLayerEnum::PolicyRules);
    }

    #[tokio::test]
    async fn creation_gate_defers_to_cli() {
        let engine = DecisionEngine::with_defaults();
        let input = make_creation_input("json_query", "Query JSON documents");

        let result = engine.evaluate(GateKind::Creation, &input).await.unwrap();

        assert!(matches!(result.decision, Decision::Defer { .. }));
        assert_eq!(result.layer, DecisionLayerEnum::CliCheck);
    }

    #[tokio::test]
    async fn creation_gate_caches_terminal_decisions() {
        let engine = DecisionEngine::with_defaults();
        let input = make_creation_input("shell_exec", "Run shell commands");

        // First evaluation
        engine.evaluate(GateKind::Creation, &input).await.unwrap();

        // Should now be cached
        assert!(engine.creation_cache().len().await > 0);
    }

    #[tokio::test]
    async fn creation_gate_unknown_tool_reaches_llm() {
        let engine = DecisionEngine::with_defaults();
        let input = make_creation_input("github_issues", "Fetch GitHub issues with filtering");

        let result = engine.evaluate(GateKind::Creation, &input).await.unwrap();

        // Stub LLM returns Ask, which is a terminal decision from LLM layer
        assert_eq!(result.layer, DecisionLayerEnum::LlmEvaluation);
    }

    #[tokio::test]
    async fn execution_gate_denies_shell_exec() {
        let engine = DecisionEngine::with_defaults();
        let input = make_execution_input("shell_exec");

        let result = engine.evaluate(GateKind::Execution, &input).await.unwrap();

        assert!(matches!(result.decision, Decision::Deny { .. }));
        assert_eq!(result.layer, DecisionLayerEnum::PolicyRules);
    }

    #[tokio::test]
    async fn execution_gate_unknown_tool_reaches_llm() {
        let engine = DecisionEngine::with_defaults();
        let input = make_execution_input("some_approved_tool");

        let result = engine.evaluate(GateKind::Execution, &input).await.unwrap();

        // Stub LLM returns Ask
        assert!(matches!(result.decision, Decision::Ask { .. }));
    }
}
