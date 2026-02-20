use serde::{Deserialize, Serialize};

/// Tri-state decision outcome from a gate evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Decision {
    /// Request is approved to proceed.
    Allow,
    /// Request is denied with a reason.
    Deny { reason: String },
    /// Request should be redirected to an existing capability (Creation Gate only).
    Defer { target: DeferTarget },
    /// Decision requires human input.
    Ask { prompt: String, context: String },
}

impl Decision {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Decision::Allow | Decision::Deny { .. })
    }
}

/// What a DEFER decision redirects to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeferTarget {
    /// An existing tool in a registry.
    RegistryTool {
        registry: String,
        tool_name: String,
        version: String,
    },
    /// A native CLI utility.
    CliUtility { name: String, description: String },
    /// An existing tool that should be extended.
    ExtendTool {
        tool_name: String,
        suggested_features: Vec<String>,
    },
}

/// Which layer of the cascade produced the decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DecisionLayer {
    PolicyRules,
    Cache,
    RegistryLookup,
    CliCheck,
    LlmEvaluation,
    Hitl,
}

impl std::fmt::Display for DecisionLayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecisionLayer::PolicyRules => write!(f, "policy_rules"),
            DecisionLayer::Cache => write!(f, "cache"),
            DecisionLayer::RegistryLookup => write!(f, "registry_lookup"),
            DecisionLayer::CliCheck => write!(f, "cli_check"),
            DecisionLayer::LlmEvaluation => write!(f, "llm_evaluation"),
            DecisionLayer::Hitl => write!(f, "hitl"),
        }
    }
}

/// A decision paired with the layer that produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayeredDecision {
    pub decision: Decision,
    pub layer: DecisionLayer,
    pub rationale: Option<String>,
}

/// The type of gate being evaluated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateKind {
    /// "Should this tool be built?"
    Creation,
    /// "Should this invocation proceed?"
    Execution,
}

impl std::fmt::Display for GateKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GateKind::Creation => write!(f, "creation"),
            GateKind::Execution => write!(f, "execution"),
        }
    }
}
