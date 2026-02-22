use chrono::{DateTime, Utc};
use girt_core::spec::CapabilitySpec;
use serde::{Deserialize, Serialize};

use crate::llm::TokenUsage;

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
    /// Architect's explicit complexity signal. When `Some(High)`, the Orchestrator
    /// runs the Planner before the Engineer regardless of structural triggers.
    #[serde(default)]
    pub complexity_hint: Option<ComplexityHint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpecAction {
    Build,
    RecommendExtend,
}

/// Complexity signal from the Architect. Determines whether the Planner runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComplexityHint {
    Low,
    High,
}

/// Structured implementation brief produced by the Planner agent.
///
/// The Engineer receives this alongside the refined spec. Following the plan
/// is mandatory; deviations require an explanatory code comment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImplementationPlan {
    /// All input validation that must occur before any external calls,
    /// with exact constraints (max lengths, allowed char sets, etc.).
    pub validation_layer: String,
    /// Threat model: for each input field, what can an attacker do and
    /// what mitigations are required (CRLF injection, path traversal, etc.).
    pub security_notes: String,
    /// Step-by-step API call sequence with error handling for each step.
    pub api_sequence: String,
    /// Identified edge cases and the required handling for each.
    pub edge_cases: String,
    /// Language-specific patterns, crate recommendations, and things to avoid
    /// in WASM+WASI (e.g. std::thread, blocking I/O patterns).
    pub implementation_guidance: String,
}

/// Supported build target languages.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetLanguage {
    #[default]
    Rust,
    Go,
    AssemblyScript,
}

impl std::fmt::Display for TargetLanguage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rust => write!(f, "rust"),
            Self::Go => write!(f, "go"),
            Self::AssemblyScript => write!(f, "assemblyscript"),
        }
    }
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
    /// Severity tier. Defaults to High when not specified (safe default).
    #[serde(default)]
    pub severity: BugTicketSeverity,
    pub input: serde_json::Value,
    pub expected: String,
    pub actual: String,
    pub remediation_directive: String,
}

impl BugTicket {
    /// A ticket is blocking (triggers a fix loop) if severity is Critical or High.
    pub fn is_blocking(&self) -> bool {
        matches!(
            self.severity,
            BugTicketSeverity::Critical | BugTicketSeverity::High
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BugTicketType {
    FunctionalDefect,
    SecurityVulnerability,
}

/// Severity tier for bug tickets.
///
/// Critical and High tickets block the build and trigger Engineer fix loops.
/// Medium and Low tickets are reported in the artifact but do not block.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BugTicketSeverity {
    Critical,
    #[default]
    High,
    Medium,
    Low,
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

/// Per-iteration timing + token breakdown (one entry per Engineer→QA→RedTeam cycle).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IterationTimings {
    pub iteration: u32,
    pub engineer_ms: u64,
    pub engineer_tokens: TokenUsage,
    pub qa_ms: u64,
    pub qa_tokens: TokenUsage,
    pub red_team_ms: u64,
    pub red_team_tokens: TokenUsage,
}

/// Full timing + token breakdown for a pipeline run.
///
/// Use this to identify which stage is the bottleneck and most expensive.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StageTimings {
    /// Architect phase (spec refinement).
    pub architect_ms: u64,
    pub architect_tokens: TokenUsage,
    /// Planner phase (`None` if skipped for low-complexity specs).
    pub planner_ms: Option<u64>,
    pub planner_tokens: Option<TokenUsage>,
    /// Per-iteration timings (one entry per Engineer→QA→RedTeam cycle).
    pub iterations: Vec<IterationTimings>,
    /// Total wall-clock time for the full pipeline run.
    pub total_ms: u64,
}

impl StageTimings {
    pub fn total_engineer_ms(&self) -> u64 {
        self.iterations.iter().map(|i| i.engineer_ms).sum()
    }
    pub fn total_qa_ms(&self) -> u64 {
        self.iterations.iter().map(|i| i.qa_ms).sum()
    }
    pub fn total_red_team_ms(&self) -> u64 {
        self.iterations.iter().map(|i| i.red_team_ms).sum()
    }
    pub fn total_input_tokens(&self) -> u64 {
        self.architect_tokens.input_tokens
            + self.planner_tokens.as_ref().map_or(0, |t| t.input_tokens)
            + self.iterations.iter().map(|i| {
                i.engineer_tokens.input_tokens
                    + i.qa_tokens.input_tokens
                    + i.red_team_tokens.input_tokens
            }).sum::<u64>()
    }
    pub fn total_output_tokens(&self) -> u64 {
        self.architect_tokens.output_tokens
            + self.planner_tokens.as_ref().map_or(0, |t| t.output_tokens)
            + self.iterations.iter().map(|i| {
                i.engineer_tokens.output_tokens
                    + i.qa_tokens.output_tokens
                    + i.red_team_tokens.output_tokens
            }).sum::<u64>()
    }
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
    /// Per-stage timing breakdown for this pipeline run.
    pub timings: StageTimings,
    /// True if the pipeline hit the iteration limit and proceeded past it
    /// (on_circuit_breaker = "ask" or "proceed"). The build was shipped
    /// despite remaining blocking tickets — see escalated_tickets.
    #[serde(default)]
    pub escalated: bool,
    /// Blocking tickets that were unresolved when the pipeline was escalated.
    /// Empty when escalated = false.
    #[serde(default)]
    pub escalated_tickets: Vec<BugTicket>,
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

/// Predefined resource limit tiers for policy generation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceTier {
    /// Minimal resources for simple stateless transforms (64 MB, 5s timeout).
    Minimal,
    /// Standard resources for typical tools (128 MB, 15s timeout).
    #[default]
    Standard,
    /// Extended resources for data-heavy or network-bound tools (512 MB, 60s timeout).
    Extended,
}

impl ResourceTier {
    /// Convert tier to concrete resource limits.
    pub fn to_resources(&self) -> PolicyResources {
        match self {
            Self::Minimal => PolicyResources {
                memory_mb: 64,
                fuel: 100_000_000,
                timeout_seconds: 5,
                max_response_bytes: 1_048_576, // 1 MB
            },
            Self::Standard => PolicyResources {
                memory_mb: 128,
                fuel: 500_000_000,
                timeout_seconds: 15,
                max_response_bytes: 5_242_880, // 5 MB
            },
            Self::Extended => PolicyResources {
                memory_mb: 512,
                fuel: 2_000_000_000,
                timeout_seconds: 60,
                max_response_bytes: 20_971_520, // 20 MB
            },
        }
    }
}

/// Maximum allowed resource limits (hard ceiling).
const MAX_MEMORY_MB: u32 = 1024;
const MAX_TIMEOUT_SECONDS: u32 = 120;
const MAX_RESPONSE_BYTES: u64 = 52_428_800; // 50 MB

impl PolicyResources {
    /// Validate that resource limits are within acceptable bounds.
    pub fn validate(&self) -> Result<(), String> {
        if self.memory_mb > MAX_MEMORY_MB {
            return Err(format!(
                "memory_mb {} exceeds maximum {}",
                self.memory_mb, MAX_MEMORY_MB
            ));
        }
        if self.timeout_seconds > MAX_TIMEOUT_SECONDS {
            return Err(format!(
                "timeout_seconds {} exceeds maximum {}",
                self.timeout_seconds, MAX_TIMEOUT_SECONDS
            ));
        }
        if self.max_response_bytes > MAX_RESPONSE_BYTES {
            return Err(format!(
                "max_response_bytes {} exceeds maximum {}",
                self.max_response_bytes, MAX_RESPONSE_BYTES
            ));
        }
        Ok(())
    }
}

impl PolicyYaml {
    /// Create a policy from a spec using the default (Standard) resource tier.
    pub fn from_spec(spec: &CapabilitySpec) -> Self {
        Self::from_spec_with_tier(spec, &ResourceTier::default())
    }

    /// Create a policy from a spec with a specific resource tier.
    pub fn from_spec_with_tier(spec: &CapabilitySpec, tier: &ResourceTier) -> Self {
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
            resources: tier.to_resources(),
        }
    }

    /// Infer the appropriate resource tier from a spec's constraints.
    pub fn infer_tier(spec: &CapabilitySpec) -> ResourceTier {
        let has_network = !spec.constraints.network.is_empty();
        let has_storage = !spec.constraints.storage.is_empty();

        if has_network && has_storage {
            ResourceTier::Extended
        } else if has_network || has_storage {
            ResourceTier::Standard
        } else {
            ResourceTier::Minimal
        }
    }
}
