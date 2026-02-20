use crate::decision::{Decision, DeferTarget};
use crate::error::DecisionError;
use crate::layers::DecisionLayer;
use crate::spec::GateInput;

/// CLI/native check layer — checks if a well-known CLI utility already handles
/// the requested capability better than a WASM tool.
///
/// This layer only applies to Creation Gate (not Execution Gate).
pub struct CliCheckLayer {
    known_utilities: Vec<CliUtility>,
}

#[derive(Debug, Clone)]
pub struct CliUtility {
    pub name: String,
    pub description: String,
    /// Keywords that trigger a match against this utility.
    pub keywords: Vec<String>,
}

impl CliCheckLayer {
    pub fn new(utilities: Vec<CliUtility>) -> Self {
        Self {
            known_utilities: utilities,
        }
    }

    /// Create a layer with a default list of well-known CLI utilities.
    pub fn with_defaults() -> Self {
        Self {
            known_utilities: default_utilities(),
        }
    }
}

impl DecisionLayer for CliCheckLayer {
    fn name(&self) -> &str {
        "cli_check"
    }

    fn evaluate<'a>(
        &'a self,
        input: &'a GateInput,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Option<Decision>, DecisionError>> + Send + 'a>,
    > {
        Box::pin(async move {
            let spec = match input {
                GateInput::Creation(spec) => spec,
                GateInput::Execution(_) => return Ok(None),
            };

            let name_lower = spec.name.to_lowercase();
            let desc_lower = spec.description.to_lowercase();

            for utility in &self.known_utilities {
                let matched = utility.keywords.iter().any(|kw| {
                    let kw_lower = kw.to_lowercase();
                    name_lower.contains(&kw_lower) || desc_lower.contains(&kw_lower)
                });

                if matched {
                    tracing::info!(
                        utility = %utility.name,
                        "CLI utility match found: DEFER"
                    );
                    return Ok(Some(Decision::Defer {
                        target: DeferTarget::CliUtility {
                            name: utility.name.clone(),
                            description: utility.description.clone(),
                        },
                    }));
                }
            }

            Ok(None)
        })
    }
}

fn default_utilities() -> Vec<CliUtility> {
    vec![
        CliUtility {
            name: "jq".into(),
            description: "Command-line JSON processor".into(),
            keywords: vec!["jq".into(), "json_query".into(), "json_filter".into()],
        },
        CliUtility {
            name: "curl".into(),
            description: "Transfer data with URLs".into(),
            keywords: vec!["curl".into()],
        },
        CliUtility {
            name: "ripgrep".into(),
            description: "Recursively search directories for a regex pattern".into(),
            keywords: vec!["ripgrep".into(), "rg".into()],
        },
        CliUtility {
            name: "sed".into(),
            description: "Stream editor for filtering and transforming text".into(),
            keywords: vec!["sed".into(), "stream_edit".into()],
        },
        CliUtility {
            name: "awk".into(),
            description: "Pattern scanning and processing language".into(),
            keywords: vec!["awk".into()],
        },
        CliUtility {
            name: "git".into(),
            description: "Distributed version control system".into(),
            keywords: vec!["git_clone".into(), "git_commit".into(), "git_push".into()],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{CapabilityConstraints, CapabilitySpec};

    fn make_spec(name: &str, desc: &str) -> GateInput {
        GateInput::Creation(CapabilitySpec {
            name: name.into(),
            description: desc.into(),
            inputs: serde_json::Value::Null,
            outputs: serde_json::Value::Null,
            constraints: CapabilityConstraints::default(),
        })
    }

    #[tokio::test]
    async fn defers_to_jq_for_json_query() {
        let layer = CliCheckLayer::with_defaults();
        let input = make_spec("json_query", "Query JSON documents");

        let result = layer.evaluate(&input).await.unwrap();
        match result {
            Some(Decision::Defer {
                target: DeferTarget::CliUtility { name, .. },
            }) => assert_eq!(name, "jq"),
            other => panic!("Expected Defer to jq, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn defers_to_ripgrep() {
        let layer = CliCheckLayer::with_defaults();
        let input = make_spec("ripgrep_search", "Search files with ripgrep");

        let result = layer.evaluate(&input).await.unwrap();
        assert!(matches!(
            result,
            Some(Decision::Defer {
                target: DeferTarget::CliUtility { .. }
            })
        ));
    }

    #[tokio::test]
    async fn passes_through_unknown_tool() {
        let layer = CliCheckLayer::with_defaults();
        let input = make_spec("github_issues", "Fetch GitHub issues");

        let result = layer.evaluate(&input).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn skips_execution_requests() {
        let layer = CliCheckLayer::with_defaults();
        let input = GateInput::Execution(crate::spec::ExecutionRequest {
            tool_name: "jq".into(),
            arguments: serde_json::Value::Null,
        });

        let result = layer.evaluate(&input).await.unwrap();
        assert!(result.is_none());
    }
}
