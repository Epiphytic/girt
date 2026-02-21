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
                    contains_word(&name_lower, &kw_lower)
                        || contains_word(&desc_lower, &kw_lower)
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

/// Check whether `text` contains `word` as a whole word (bounded by
/// non-alphanumeric characters or string start/end).
///
/// This prevents short keywords like "sed" matching inside words like
/// "elapsed" or "described".
fn contains_word(text: &str, word: &str) -> bool {
    let mut start = 0;
    while let Some(pos) = text[start..].find(word) {
        let abs = start + pos;
        let before_ok = abs == 0
            || !text[..abs]
                .chars()
                .next_back()
                .map(|c| c.is_alphanumeric() || c == '_')
                .unwrap_or(false);
        let after_ok = abs + word.len() >= text.len()
            || !text[abs + word.len()..]
                .chars()
                .next()
                .map(|c| c.is_alphanumeric() || c == '_')
                .unwrap_or(false);
        if before_ok && after_ok {
            return true;
        }
        start = abs + 1;
    }
    false
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

    /// "elapsed" contains the substring "sed" but is NOT a whole-word match.
    /// A Discord bot using "elapsed time" in its description should not be
    /// deferred to sed.
    #[tokio::test]
    async fn elapsed_does_not_match_sed() {
        let layer = CliCheckLayer::with_defaults();
        let input = make_spec(
            "discord_approval",
            "Post an approval request to Discord. Uses wasi:clocks/wall-clock \
             to track elapsed time for polling.",
        );
        let result = layer.evaluate(&input).await.unwrap();
        assert!(
            result.is_none(),
            "discord_approval should not be deferred to sed via 'elapsed'"
        );
    }

    #[test]
    fn contains_word_whole_word_match() {
        assert!(contains_word("use sed to edit", "sed"));
        assert!(contains_word("sed filter", "sed"));
        assert!(contains_word("run sed", "sed"));
        assert!(contains_word("sed", "sed"));
    }

    #[test]
    fn contains_word_no_partial_match() {
        assert!(!contains_word("elapsed time", "sed"));
        assert!(!contains_word("described above", "sed"));
        assert!(!contains_word("discord", "sed"));
        assert!(!contains_word("messages", "sed"));
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
