use crate::decision::{Decision, DeferTarget};
use crate::error::DecisionError;
use crate::layers::DecisionLayer;
use crate::spec::GateInput;

/// Registry lookup layer — checks if a matching tool already exists in configured OCI registries.
///
/// If a match is found, returns a DEFER decision pointing to the existing tool.
/// This layer only applies to Creation Gate (not Execution Gate).
pub struct RegistryLookupLayer {
    registries: Vec<RegistryConfig>,
}

#[derive(Debug, Clone)]
pub struct RegistryConfig {
    pub name: String,
    pub url: String,
}

/// A tool found in a registry.
#[derive(Debug, Clone)]
pub struct RegistryToolMatch {
    pub registry: String,
    pub tool_name: String,
    pub version: String,
    pub description: String,
}

/// Trait for querying OCI registries. Abstracted for testability.
pub trait RegistryClient: Send + Sync {
    fn search(
        &self,
        registry: &RegistryConfig,
        query: &str,
    ) -> impl std::future::Future<Output = Result<Vec<RegistryToolMatch>, DecisionError>> + Send;
}

/// Stub registry client that always returns empty results.
/// Phase 1 does not implement real OCI queries — that comes in Phase 2.
pub struct StubRegistryClient;

impl RegistryClient for StubRegistryClient {
    async fn search(
        &self,
        _registry: &RegistryConfig,
        _query: &str,
    ) -> Result<Vec<RegistryToolMatch>, DecisionError> {
        Ok(vec![])
    }
}

impl RegistryLookupLayer {
    pub fn new(registries: Vec<RegistryConfig>) -> Self {
        Self { registries }
    }
}

impl DecisionLayer for RegistryLookupLayer {
    fn name(&self) -> &str {
        "registry_lookup"
    }

    fn evaluate<'a>(
        &'a self,
        input: &'a GateInput,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Option<Decision>, DecisionError>> + Send + 'a>,
    > {
        Box::pin(async move {
            // Registry lookup only applies to Creation Gate
            let spec = match input {
                GateInput::Creation(spec) => spec,
                GateInput::Execution(_) => return Ok(None),
            };

            let client = StubRegistryClient;

            for registry in &self.registries {
                match client.search(registry, &spec.name).await {
                    Ok(matches) => {
                        if let Some(tool_match) = matches.first() {
                            tracing::info!(
                                registry = %tool_match.registry,
                                tool = %tool_match.tool_name,
                                version = %tool_match.version,
                                "Registry match found: DEFER"
                            );
                            return Ok(Some(Decision::Defer {
                                target: DeferTarget::RegistryTool {
                                    registry: tool_match.registry.clone(),
                                    tool_name: tool_match.tool_name.clone(),
                                    version: tool_match.version.clone(),
                                },
                            }));
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            registry = %registry.name,
                            error = %e,
                            "Registry lookup failed, skipping"
                        );
                    }
                }
            }

            Ok(None)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{CapabilityConstraints, CapabilitySpec, ExecutionRequest};

    fn make_creation_input(name: &str) -> GateInput {
        GateInput::Creation(CapabilitySpec {
            name: name.into(),
            description: "test".into(),
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
    async fn stub_client_passes_through() {
        let layer = RegistryLookupLayer::new(vec![RegistryConfig {
            name: "test".into(),
            url: "oci://test.registry".into(),
        }]);
        let input = make_creation_input("some_tool");

        let result = layer.evaluate(&input).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn skips_execution_requests() {
        let layer = RegistryLookupLayer::new(vec![RegistryConfig {
            name: "test".into(),
            url: "oci://test.registry".into(),
        }]);
        let input = make_execution_input("some_tool");

        let result = layer.evaluate(&input).await.unwrap();
        assert!(result.is_none());
    }
}
