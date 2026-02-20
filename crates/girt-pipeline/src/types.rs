use chrono::{DateTime, Utc};
use girt_core::spec::CapabilitySpec;
use serde::{Deserialize, Serialize};

/// A capability request in the build queue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityRequest {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub source: RequestSource,
    pub spec: CapabilitySpec,
    pub status: RequestStatus,
    pub priority: Priority,
    pub attempts: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestSource {
    Operator,
    Cli,
    Hook,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Low,
    #[default]
    Normal,
    High,
}

impl CapabilityRequest {
    pub fn new(spec: CapabilitySpec, source: RequestSource) -> Self {
        Self {
            id: format!("req_{}", uuid::Uuid::new_v4().simple()),
            timestamp: Utc::now(),
            source,
            spec,
            status: RequestStatus::Pending,
            priority: Priority::default(),
            attempts: 0,
        }
    }
}

/// The Architect's refined tool specification output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefinedSpec {
    pub action: SpecAction,
    pub spec: CapabilitySpec,
    pub design_notes: String,
    pub extend_target: Option<String>,
    pub extend_features: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpecAction {
    Build,
    RecommendExtend,
}

/// The Engineer's build output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildOutput {
    pub source_code: String,
    pub wit_definition: String,
    pub policy_yaml: String,
    pub language: String,
}

/// A bug ticket from QA or Red Team back to the Engineer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BugTicket {
    pub target: String,
    pub ticket_type: BugTicketType,
    pub input: serde_json::Value,
    pub expected: String,
    pub actual: String,
    pub remediation_directive: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BugTicketType {
    FunctionalDefect,
    SecurityVulnerability,
}

/// QA test results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QaResult {
    pub passed: bool,
    pub tests_run: u32,
    pub tests_passed: u32,
    pub tests_failed: u32,
    pub bug_tickets: Vec<BugTicket>,
}

/// Red Team audit results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityResult {
    pub passed: bool,
    pub exploits_attempted: u32,
    pub exploits_succeeded: u32,
    pub bug_tickets: Vec<BugTicket>,
}

/// The final build artifact ready for publishing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildArtifact {
    pub spec: CapabilitySpec,
    pub refined_spec: RefinedSpec,
    pub build_output: BuildOutput,
    pub qa_result: QaResult,
    pub security_result: SecurityResult,
    pub build_iterations: u32,
}

/// Wassette policy.yaml content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyYaml {
    pub version: String,
    pub permissions: PolicyPermissions,
    pub resources: PolicyResources,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyPermissions {
    pub network: NetworkPermissions,
    pub storage: serde_json::Value,
    pub environment: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkPermissions {
    pub allow: Vec<NetworkHost>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkHost {
    pub host: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyResources {
    pub memory_mb: u32,
    pub fuel: u64,
    pub timeout_seconds: u32,
    pub max_response_bytes: u64,
}

impl PolicyYaml {
    pub fn from_spec(spec: &CapabilitySpec) -> Self {
        Self {
            version: "1.0".into(),
            permissions: PolicyPermissions {
                network: NetworkPermissions {
                    allow: spec
                        .constraints
                        .network
                        .iter()
                        .map(|h| NetworkHost { host: h.clone() })
                        .collect(),
                },
                storage: serde_json::json!({}),
                environment: serde_json::json!({}),
            },
            resources: PolicyResources {
                memory_mb: 128,
                fuel: 500_000_000,
                timeout_seconds: 15,
                max_response_bytes: 5_242_880,
            },
        }
    }
}
