use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::decision::Decision;
use crate::error::DecisionError;
use crate::layers::DecisionLayer;
use crate::spec::{CapabilitySpec, ExecutionRequest, GateInput};

/// Pattern-matching policy rules for known-good and known-bad requests.
///
/// Rules are evaluated in order. The first matching rule produces the decision.
/// If no rule matches, the layer passes through to the next cascade layer.
pub struct PolicyRulesLayer {
    deny_patterns: Vec<PolicyPattern>,
    allow_patterns: Vec<PolicyPattern>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyPattern {
    pub description: String,
    pub name_pattern: Option<String>,
    pub description_pattern: Option<String>,
    pub constraint_patterns: Option<ConstraintPatterns>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstraintPatterns {
    pub network_deny: Option<Vec<String>>,
    pub storage_deny: Option<Vec<String>>,
    pub secrets_deny: Option<Vec<String>>,
}

impl PolicyRulesLayer {
    pub fn new(deny_patterns: Vec<PolicyPattern>, allow_patterns: Vec<PolicyPattern>) -> Self {
        Self {
            deny_patterns,
            allow_patterns,
        }
    }

    /// Create a layer with sensible default deny/allow patterns.
    pub fn with_defaults() -> Self {
        Self {
            deny_patterns: default_deny_patterns(),
            allow_patterns: default_allow_patterns(),
        }
    }

    fn matches_spec(pattern: &PolicyPattern, spec: &CapabilitySpec) -> bool {
        if let Some(name_pat) = &pattern.name_pattern
            && let Ok(re) = Regex::new(name_pat)
            && re.is_match(&spec.name)
        {
            return true;
        }

        if let Some(desc_pat) = &pattern.description_pattern
            && let Ok(re) = Regex::new(desc_pat)
            && re.is_match(&spec.description)
        {
            return true;
        }

        if let Some(constraint_pats) = &pattern.constraint_patterns
            && Self::matches_constraints(constraint_pats, spec)
        {
            return true;
        }

        false
    }

    fn matches_constraints(patterns: &ConstraintPatterns, spec: &CapabilitySpec) -> bool {
        if let Some(network_deny) = &patterns.network_deny {
            for deny_pat in network_deny {
                if let Ok(re) = Regex::new(deny_pat)
                    && spec
                        .constraints
                        .network
                        .iter()
                        .any(|host| re.is_match(host))
                {
                    return true;
                }
            }
        }

        if let Some(storage_deny) = &patterns.storage_deny {
            for deny_pat in storage_deny {
                if let Ok(re) = Regex::new(deny_pat)
                    && spec
                        .constraints
                        .storage
                        .iter()
                        .any(|path| re.is_match(path))
                {
                    return true;
                }
            }
        }

        false
    }

    fn matches_execution(pattern: &PolicyPattern, req: &ExecutionRequest) -> bool {
        if let Some(name_pat) = &pattern.name_pattern
            && let Ok(re) = Regex::new(name_pat)
            && re.is_match(&req.tool_name)
        {
            return true;
        }
        false
    }
}

impl DecisionLayer for PolicyRulesLayer {
    fn name(&self) -> &str {
        "policy_rules"
    }

    fn evaluate<'a>(
        &'a self,
        input: &'a GateInput,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Option<Decision>, DecisionError>> + Send + 'a>,
    > {
        Box::pin(async move {
            // Check deny patterns first (deny takes priority)
            for pattern in &self.deny_patterns {
                let matched = match input {
                    GateInput::Creation(spec) => Self::matches_spec(pattern, spec),
                    GateInput::Execution(req) => Self::matches_execution(pattern, req),
                };

                if matched {
                    tracing::info!(
                        pattern = %pattern.description,
                        "Policy rule matched: DENY"
                    );
                    return Ok(Some(Decision::Deny {
                        reason: format!("Policy rule: {}", pattern.description),
                    }));
                }
            }

            // Check allow patterns
            for pattern in &self.allow_patterns {
                let matched = match input {
                    GateInput::Creation(spec) => Self::matches_spec(pattern, spec),
                    GateInput::Execution(req) => Self::matches_execution(pattern, req),
                };

                if matched {
                    tracing::info!(
                        pattern = %pattern.description,
                        "Policy rule matched: ALLOW"
                    );
                    return Ok(Some(Decision::Allow));
                }
            }

            // No rule matched -- pass through to next layer
            Ok(None)
        })
    }
}

/// Known-dangerous patterns that should be auto-denied.
fn default_deny_patterns() -> Vec<PolicyPattern> {
    vec![
        PolicyPattern {
            description: "Shell execution access".into(),
            name_pattern: Some(r"(?i)(shell_exec|run_command|system_call|exec_cmd)".into()),
            description_pattern: Some(
                r"(?i)(execute.*shell|run.*command|system.*exec|spawn.*process)".into(),
            ),
            constraint_patterns: None,
        },
        PolicyPattern {
            description: "Credential extraction".into(),
            name_pattern: Some(
                r"(?i)(steal|extract|dump|harvest).*(cred|secret|token|key|password)".into(),
            ),
            description_pattern: Some(
                r"(?i)(steal|extract|dump|harvest).*(cred|secret|token|key|password)".into(),
            ),
            constraint_patterns: None,
        },
        PolicyPattern {
            description: "Filesystem root access".into(),
            name_pattern: None,
            description_pattern: Some(r"(?i)(read|write|access).*/etc/(shadow|passwd)".into()),
            constraint_patterns: Some(ConstraintPatterns {
                network_deny: None,
                storage_deny: Some(vec![
                    r"^/$".into(),
                    r"^/etc".into(),
                    r"^/root".into(),
                    r"^/proc".into(),
                    r"^/sys".into(),
                ]),
                secrets_deny: None,
            }),
        },
        PolicyPattern {
            description: "Cloud metadata SSRF".into(),
            name_pattern: None,
            description_pattern: None,
            constraint_patterns: Some(ConstraintPatterns {
                network_deny: Some(vec![
                    r"169\.254\.169\.254".into(),
                    r"metadata\.google\.internal".into(),
                    r"metadata\.azure\.com".into(),
                ]),
                storage_deny: None,
                secrets_deny: None,
            }),
        },
        PolicyPattern {
            description: "Wildcard network access".into(),
            name_pattern: None,
            description_pattern: None,
            constraint_patterns: Some(ConstraintPatterns {
                network_deny: Some(vec![r"^\*$".into(), r"^\*\.".into()]),
                storage_deny: None,
                secrets_deny: None,
            }),
        },
    ]
}

/// Known-safe patterns that can be auto-allowed.
fn default_allow_patterns() -> Vec<PolicyPattern> {
    vec![
        PolicyPattern {
            description: "Pure math operations".into(),
            name_pattern: Some(r"(?i)^(math|calc|convert|compute)_".into()),
            description_pattern: Some(r"(?i)(mathematical|arithmetic|conversion|calculate)".into()),
            constraint_patterns: None,
        },
        PolicyPattern {
            description: "String/text operations".into(),
            name_pattern: Some(r"(?i)^(string|text|format|parse|encode|decode|regex)_".into()),
            description_pattern: None,
            constraint_patterns: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::CapabilityConstraints;

    fn make_spec(name: &str, desc: &str) -> GateInput {
        GateInput::Creation(CapabilitySpec {
            name: name.into(),
            description: desc.into(),
            inputs: serde_json::Value::Null,
            outputs: serde_json::Value::Null,
            constraints: CapabilityConstraints::default(),
        })
    }

    fn make_spec_with_constraints(name: &str, network: Vec<&str>, storage: Vec<&str>) -> GateInput {
        GateInput::Creation(CapabilitySpec {
            name: name.into(),
            description: "test".into(),
            inputs: serde_json::Value::Null,
            outputs: serde_json::Value::Null,
            constraints: CapabilityConstraints {
                network: network.into_iter().map(String::from).collect(),
                storage: storage.into_iter().map(String::from).collect(),
                secrets: vec![],
            },
        })
    }

    #[tokio::test]
    async fn denies_shell_execution() {
        let layer = PolicyRulesLayer::with_defaults();
        let input = make_spec("shell_exec", "Execute shell commands");

        let result = layer.evaluate(&input).await.unwrap();
        assert!(matches!(result, Some(Decision::Deny { .. })));
    }

    #[tokio::test]
    async fn denies_credential_extraction() {
        let layer = PolicyRulesLayer::with_defaults();
        let input = make_spec("extract_credentials", "Extract user credentials from vault");

        let result = layer.evaluate(&input).await.unwrap();
        assert!(matches!(result, Some(Decision::Deny { .. })));
    }

    #[tokio::test]
    async fn denies_root_filesystem_access() {
        let layer = PolicyRulesLayer::with_defaults();
        let input = make_spec_with_constraints("file_tool", vec![], vec!["/etc"]);

        let result = layer.evaluate(&input).await.unwrap();
        assert!(matches!(result, Some(Decision::Deny { .. })));
    }

    #[tokio::test]
    async fn denies_cloud_metadata_ssrf() {
        let layer = PolicyRulesLayer::with_defaults();
        let input = make_spec_with_constraints("http_fetch", vec!["169.254.169.254"], vec![]);

        let result = layer.evaluate(&input).await.unwrap();
        assert!(matches!(result, Some(Decision::Deny { .. })));
    }

    #[tokio::test]
    async fn allows_math_operations() {
        let layer = PolicyRulesLayer::with_defaults();
        let input = make_spec("math_convert", "Convert temperature units");

        let result = layer.evaluate(&input).await.unwrap();
        assert!(matches!(result, Some(Decision::Allow)));
    }

    #[tokio::test]
    async fn allows_string_operations() {
        let layer = PolicyRulesLayer::with_defaults();
        let input = make_spec("string_format", "Format a template string");

        let result = layer.evaluate(&input).await.unwrap();
        assert!(matches!(result, Some(Decision::Allow)));
    }

    #[tokio::test]
    async fn passes_through_unknown_spec() {
        let layer = PolicyRulesLayer::with_defaults();
        let input = make_spec("github_issues", "Fetch GitHub issues with filtering");

        let result = layer.evaluate(&input).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn deny_takes_priority_over_allow() {
        let layer = PolicyRulesLayer::new(
            vec![PolicyPattern {
                description: "deny all".into(),
                name_pattern: Some(r".*".into()),
                description_pattern: None,
                constraint_patterns: None,
            }],
            vec![PolicyPattern {
                description: "allow all".into(),
                name_pattern: Some(r".*".into()),
                description_pattern: None,
                constraint_patterns: None,
            }],
        );
        let input = make_spec("anything", "anything");

        let result = layer.evaluate(&input).await.unwrap();
        assert!(matches!(result, Some(Decision::Deny { .. })));
    }
}
