use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// A capability spec describing what tool is being requested.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilitySpec {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub inputs: serde_json::Value,
    #[serde(default)]
    pub outputs: serde_json::Value,
    #[serde(default)]
    pub constraints: CapabilityConstraints,
}

/// Security constraints for a capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CapabilityConstraints {
    #[serde(default)]
    pub network: Vec<String>,
    #[serde(default)]
    pub storage: Vec<String>,
    #[serde(default)]
    pub secrets: Vec<String>,
}

impl CapabilitySpec {
    /// Compute a stable SHA-256 hash of this spec for cache keying.
    ///
    /// Uses canonical JSON serialization (serde_json sorts keys by default when
    /// the input is a struct with named fields).
    pub fn spec_hash(&self) -> String {
        let canonical = serde_json::to_string(self).unwrap_or_default();
        let mut hasher = Sha256::new();
        hasher.update(canonical.as_bytes());
        let result = hasher.finalize();
        hex::encode(result)
    }
}

/// An execution request describing a tool invocation being evaluated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionRequest {
    pub tool_name: String,
    #[serde(default)]
    pub arguments: serde_json::Value,
}

impl ExecutionRequest {
    /// Compute a hash for cache keying.
    pub fn request_hash(&self) -> String {
        let canonical = serde_json::to_string(self).unwrap_or_default();
        let mut hasher = Sha256::new();
        hasher.update(canonical.as_bytes());
        let result = hasher.finalize();
        hex::encode(result)
    }
}

/// Union type for what a gate evaluates.
#[derive(Debug, Clone)]
pub enum GateInput {
    Creation(CapabilitySpec),
    Execution(ExecutionRequest),
}

impl GateInput {
    pub fn hash(&self) -> String {
        match self {
            GateInput::Creation(spec) => spec.spec_hash(),
            GateInput::Execution(req) => req.request_hash(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_hash_is_deterministic() {
        let spec = CapabilitySpec {
            name: "test_tool".into(),
            description: "A test tool".into(),
            inputs: serde_json::json!({"param": "string"}),
            outputs: serde_json::json!({"result": "string"}),
            constraints: CapabilityConstraints::default(),
        };

        let h1 = spec.spec_hash();
        let h2 = spec.spec_hash();
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64); // SHA-256 hex = 64 chars
    }

    #[test]
    fn different_specs_produce_different_hashes() {
        let spec1 = CapabilitySpec {
            name: "tool_a".into(),
            description: "Tool A".into(),
            inputs: serde_json::Value::Null,
            outputs: serde_json::Value::Null,
            constraints: CapabilityConstraints::default(),
        };
        let spec2 = CapabilitySpec {
            name: "tool_b".into(),
            description: "Tool B".into(),
            inputs: serde_json::Value::Null,
            outputs: serde_json::Value::Null,
            constraints: CapabilityConstraints::default(),
        };

        assert_ne!(spec1.spec_hash(), spec2.spec_hash());
    }
}
