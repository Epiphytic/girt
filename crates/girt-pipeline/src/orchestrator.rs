use crate::agent::architect::ArchitectAgent;
use crate::agent::engineer::EngineerAgent;
use crate::agent::planner::PlannerAgent;
use crate::agent::qa::QaAgent;
use crate::agent::red_team::RedTeamAgent;
use crate::error::PipelineError;
use crate::llm::LlmClient;
use std::time::Instant;

use crate::config::CircuitBreakerPolicy;
use crate::llm::TokenUsage;
use crate::types::{
    BugTicket, BugTicketSeverity, BuildArtifact, CapabilityRequest, ComplexityHint,
    ImplementationPlan, IterationTimings, RefinedSpec, SpecAction, StageTimings,
};

/// Default maximum build-fix iterations. Override via `pipeline.max_iterations` in girt.toml.
const DEFAULT_MAX_ITERATIONS: u32 = 3;

/// Result of a single pipeline run.
#[derive(Debug)]
pub enum PipelineOutcome {
    /// Successfully built and verified artifact.
    Built(Box<BuildArtifact>),
    /// Architect recommended extending an existing tool instead of building.
    RecommendExtend {
        target: String,
        features: Vec<String>,
    },
    /// Pipeline failed after exhausting retries.
    Failed(PipelineError),
}

/// Orchestrates the Architect -> Engineer -> QA + Red Team pipeline.
///
/// The orchestrator runs the full build pipeline for a capability request:
/// 1. Architect refines the spec
/// 2. Engineer generates code (with optional coding standards injected)
/// 3. QA and Red Team validate in parallel (conceptually)
/// 4. If bugs found, loop back to Engineer with fix directives (max 3 iterations)
/// 5. Return the final artifact or failure
pub struct Orchestrator<'a> {
    llm: &'a dyn LlmClient,
    /// Optional coding standards to inject into the Engineer's system prompt.
    coding_standards: Option<String>,
    /// Maximum Engineer → QA/RedTeam iterations before the circuit breaker fires.
    max_iterations: u32,
    /// What to do when the iteration limit is reached with blocking tickets remaining.
    circuit_breaker_policy: CircuitBreakerPolicy,
}

impl<'a> Orchestrator<'a> {
    pub fn new(llm: &'a dyn LlmClient) -> Self {
        Self {
            llm,
            coding_standards: None,
            max_iterations: DEFAULT_MAX_ITERATIONS,
            circuit_breaker_policy: CircuitBreakerPolicy::default(),
        }
    }

    /// Attach coding standards to be passed to the Engineer agent.
    pub fn with_standards(mut self, standards: Option<String>) -> Self {
        self.coding_standards = standards;
        self
    }

    /// Override the circuit breaker iteration limit.
    pub fn with_max_iterations(mut self, max_iterations: u32) -> Self {
        self.max_iterations = max_iterations;
        self
    }

    /// Set the circuit breaker escalation policy.
    pub fn with_circuit_breaker_policy(mut self, policy: CircuitBreakerPolicy) -> Self {
        self.circuit_breaker_policy = policy;
        self
    }

    /// Run the full pipeline for a capability request.
    pub async fn run(&self, request: &CapabilityRequest) -> PipelineOutcome {
        let pipeline_start = Instant::now();

        // Phase 1: Architect refines the spec
        let arch_start = Instant::now();
        let (refined, architect_tokens) = match self.architect_phase(&request.spec).await {
            Ok(result) => result,
            Err(e) => {
                tracing::warn!(error = %e, "Architect failed, using passthrough spec");
                (ArchitectAgent::passthrough(&request.spec), TokenUsage::default())
            }
        };
        let architect_ms = arch_start.elapsed().as_millis() as u64;
        tracing::info!(architect_ms, input_tokens = architect_tokens.input_tokens,
            output_tokens = architect_tokens.output_tokens, "Architect phase complete");

        // Check if architect recommends extending instead of building
        if refined.action == SpecAction::RecommendExtend {
            return PipelineOutcome::RecommendExtend {
                target: refined.extend_target.unwrap_or_default(),
                features: refined.extend_features.unwrap_or_default(),
            };
        }

        // Phase 1.5: Planner (runs for complex specs before Engineer)
        let planner_start = Instant::now();
        let planner_result = self.planner_phase(&refined).await;
        let (plan, planner_ms, planner_tokens) = if let Some((p, u)) = planner_result {
            let ms = planner_start.elapsed().as_millis() as u64;
            tracing::info!(planner_ms = ms, input_tokens = u.input_tokens,
                output_tokens = u.output_tokens, "Planner phase complete");
            (Some(p), Some(ms), Some(u))
        } else {
            (None, None, None)
        };

        let partial_timings = StageTimings {
            architect_ms,
            architect_tokens,
            planner_ms,
            planner_tokens,
            iterations: vec![],
            total_ms: 0,
        };

        // Phase 2-4: Build loop with QA and Red Team validation
        match self.build_loop(&refined, plan, partial_timings, pipeline_start).await {
            Ok(artifact) => PipelineOutcome::Built(artifact),
            Err(e) => PipelineOutcome::Failed(e),
        }
    }

    async fn architect_phase(
        &self,
        spec: &girt_core::spec::CapabilitySpec,
    ) -> Result<(RefinedSpec, TokenUsage), PipelineError> {
        let architect = ArchitectAgent::new(self.llm);
        let (refined, usage) = architect.refine(spec).await?;
        tracing::info!(name = %refined.spec.name, action = ?refined.action, "Spec refined");
        Ok((refined, usage))
    }

    /// Run the Planner if the spec meets complexity triggers; return `None` on skip or failure.
    async fn planner_phase(&self, spec: &RefinedSpec) -> Option<(ImplementationPlan, TokenUsage)> {
        if !needs_planner(spec) {
            tracing::debug!(spec_name = %spec.spec.name, "Planner skipped (low complexity)");
            return None;
        }

        tracing::info!(spec_name = %spec.spec.name, "Running Planner (complex spec detected)");
        match PlannerAgent::new(self.llm).plan(spec).await {
            Ok((plan, usage)) => {
                tracing::info!(spec_name = %spec.spec.name, "Planner completed");
                Some((plan, usage))
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    spec_name = %spec.spec.name,
                    "Planner failed; proceeding without implementation plan"
                );
                None
            }
        }
    }

    async fn build_loop(
        &self,
        spec: &RefinedSpec,
        plan: Option<ImplementationPlan>,
        mut timings: StageTimings,
        pipeline_start: Instant,
    ) -> Result<Box<BuildArtifact>, PipelineError> {
        let engineer = EngineerAgent::new(self.llm)
            .with_standards(self.coding_standards.clone())
            .with_plan(plan);
        let qa = QaAgent::new(self.llm);
        let red_team = RedTeamAgent::new(self.llm);

        let eng_start = Instant::now();
        let (mut build_output, mut last_engineer_tokens) = engineer.build(spec).await?;
        let mut last_engineer_ms = eng_start.elapsed().as_millis() as u64;
        let mut iteration = 1u32;

        loop {
            tracing::info!(iteration, "Build iteration starting");

            let qa_start = Instant::now();
            let (qa_result, qa_tokens) = qa.test(spec, &build_output).await?;
            let qa_ms = qa_start.elapsed().as_millis() as u64;

            let rt_start = Instant::now();
            let (security_result, red_team_tokens) = red_team.audit(spec, &build_output).await?;
            let red_team_ms = rt_start.elapsed().as_millis() as u64;

            timings.iterations.push(IterationTimings {
                iteration,
                engineer_ms: last_engineer_ms,
                engineer_tokens: last_engineer_tokens.clone(),
                qa_ms,
                qa_tokens,
                red_team_ms,
                red_team_tokens,
            });
            tracing::info!(
                iteration, last_engineer_ms, qa_ms, red_team_ms,
                "Iteration timings"
            );

            // Collect all bug tickets from both agents
            let mut all_tickets: Vec<BugTicket> = Vec::new();
            all_tickets.extend(qa_result.bug_tickets.iter().cloned());
            all_tickets.extend(security_result.bug_tickets.iter().cloned());

            // Split into blocking (Critical/High) and advisory (Medium/Low)
            let blocking: Vec<&BugTicket> =
                all_tickets.iter().filter(|t| t.is_blocking()).collect();
            let advisory: Vec<&BugTicket> =
                all_tickets.iter().filter(|t| !t.is_blocking()).collect();

            if !advisory.is_empty() {
                tracing::info!(
                    count = advisory.len(),
                    "Advisory tickets (Medium/Low) — reported, not blocking"
                );
            }

            // Pass when no blocking tickets remain (advisory tickets are fine)
            if blocking.is_empty() {
                timings.total_ms = pipeline_start.elapsed().as_millis() as u64;
                tracing::info!(
                    iterations = iteration,
                    advisory = advisory.len(),
                    total_ms = timings.total_ms,
                    architect_ms = timings.architect_ms,
                    planner_ms = ?timings.planner_ms,
                    total_engineer_ms = timings.total_engineer_ms(),
                    total_qa_ms = timings.total_qa_ms(),
                    total_red_team_ms = timings.total_red_team_ms(),
                    total_input_tokens = timings.total_input_tokens(),
                    total_output_tokens = timings.total_output_tokens(),
                    "Pipeline passed — stage timing + token summary"
                );
                return Ok(Box::new(BuildArtifact {
                    spec: spec.spec.clone(),
                    refined_spec: spec.clone(),
                    build_output,
                    qa_result,
                    security_result,
                    build_iterations: iteration,
                    timings,
                    escalated: false,
                    escalated_tickets: vec![],
                }));
            }

            // Iteration limit reached with blocking tickets remaining
            if iteration >= self.max_iterations {
                timings.total_ms = pipeline_start.elapsed().as_millis() as u64;
                let remaining: Vec<BugTicket> = blocking.iter().map(|t| (*t).clone()).collect();
                let summary = format_ticket_summary(&blocking);

                match &self.circuit_breaker_policy {
                    CircuitBreakerPolicy::Fail => {
                        tracing::error!(
                            iteration, blocking = blocking.len(),
                            "Circuit breaker: failing pipeline (on_circuit_breaker=fail)"
                        );
                        return Err(PipelineError::CircuitBreaker {
                            attempts: iteration,
                            summary,
                        });
                    }
                    CircuitBreakerPolicy::Ask => {
                        // Future: route through approval WASM.
                        // Now: fall through to Proceed with a warning.
                        tracing::warn!(
                            iteration, blocking = blocking.len(),
                            "Circuit breaker: no approval mechanism configured, \
                             failing open (on_circuit_breaker=ask, no WASM available)"
                        );
                        // fall through to Proceed
                        tracing::warn!(summary, "Proceeding with unresolved blocking tickets");
                        return Ok(Box::new(BuildArtifact {
                            spec: spec.spec.clone(),
                            refined_spec: spec.clone(),
                            build_output,
                            qa_result,
                            security_result,
                            build_iterations: iteration,
                            timings,
                            escalated: true,
                            escalated_tickets: remaining,
                        }));
                    }
                    CircuitBreakerPolicy::Proceed => {
                        tracing::warn!(
                            iteration, blocking = blocking.len(),
                            "Circuit breaker: proceeding with unresolved tickets \
                             (on_circuit_breaker=proceed)"
                        );
                        tracing::warn!(summary, "Unresolved blocking tickets");
                        return Ok(Box::new(BuildArtifact {
                            spec: spec.spec.clone(),
                            refined_spec: spec.clone(),
                            build_output,
                            qa_result,
                            security_result,
                            build_iterations: iteration,
                            timings,
                            escalated: true,
                            escalated_tickets: remaining,
                        }));
                    }
                }
            }

            // Fix: pick the first BLOCKING ticket and send it back to engineer
            if let Some(ticket) = blocking.first() {
                tracing::info!(
                    iteration,
                    severity = ?ticket.severity,
                    ticket_type = ?ticket.ticket_type,
                    "Sending fix directive to engineer"
                );
                let fix_start = Instant::now();
                let (fixed_output, fix_tokens) = engineer.fix(spec, &build_output, ticket).await?;
                build_output = fixed_output;
                last_engineer_ms = fix_start.elapsed().as_millis() as u64;
                last_engineer_tokens = fix_tokens;
            }

            iteration += 1;
        }
    }

    /// Run the pipeline with an already-refined spec (skips Architect phase).
    /// Useful when the decision engine has already produced a spec.
    pub async fn run_from_spec(&self, spec: &RefinedSpec) -> PipelineOutcome {
        if spec.action == SpecAction::RecommendExtend {
            return PipelineOutcome::RecommendExtend {
                target: spec.extend_target.clone().unwrap_or_default(),
                features: spec.extend_features.clone().unwrap_or_default(),
            };
        }

        let pipeline_start = Instant::now();
        let planner_start = Instant::now();
        let planner_result = self.planner_phase(spec).await;
        let (plan, planner_ms, planner_tokens) = if let Some((p, u)) = planner_result {
            (Some(p), Some(planner_start.elapsed().as_millis() as u64), Some(u))
        } else {
            (None, None, None)
        };

        let timings = StageTimings {
            architect_ms: 0,
            architect_tokens: TokenUsage::default(),
            planner_ms,
            planner_tokens,
            iterations: vec![],
            total_ms: 0,
        };

        match self.build_loop(spec, plan, timings, pipeline_start).await {
            Ok(artifact) => PipelineOutcome::Built(artifact),
            Err(e) => PipelineOutcome::Failed(e),
        }
    }
}

/// Determine whether the Planner agent should run for this spec.
///
/// Triggers on structural signals (network calls, secrets, async/polling
/// language in the description, multiple user-supplied string inputs) as well
/// as an explicit `complexity_hint: "high"` from the Architect.
fn needs_planner(spec: &RefinedSpec) -> bool {
    // Explicit override from Architect
    if spec.complexity_hint == Some(ComplexityHint::High) {
        return true;
    }

    let constraints = &spec.spec.constraints;

    // Any outbound network calls → potentially injection surfaces + error handling
    if !constraints.network.is_empty() {
        return true;
    }

    // Any secrets → credential hygiene required
    if !constraints.secrets.is_empty() {
        return true;
    }

    // Async/polling language → timing edge cases, loop termination
    let desc = spec.spec.description.to_lowercase();
    if desc.contains("poll") || desc.contains("wait") || desc.contains("timeout") {
        return true;
    }

    // Multiple user-supplied string inputs → potential injection surfaces
    if let Some(obj) = spec.spec.inputs.as_object() {
        let string_inputs = obj
            .values()
            .filter(|v| {
                v.as_str()
                    .map(|s| s.to_lowercase().contains("string") || s.to_lowercase().contains("str"))
                    .unwrap_or(false)
            })
            .count();
        if string_inputs >= 2 {
            return true;
        }
    }

    false
}

fn format_ticket_summary(tickets: &[&BugTicket]) -> String {
    tickets
        .iter()
        .enumerate()
        .map(|(i, t)| {
            format!(
                "#{}: [{:?}/{:?}] expected: {}, actual: {}",
                i + 1,
                t.severity,
                t.ticket_type,
                t.expected,
                t.actual
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::StubLlmClient;
    use crate::types::{RequestSource, SpecAction};
    use girt_core::spec::{CapabilityConstraints, CapabilitySpec};

    fn make_request() -> CapabilityRequest {
        CapabilityRequest::new(
            CapabilitySpec {
                name: "test_tool".into(),
                description: "A test tool".into(),
                inputs: serde_json::json!({"value": "string"}),
                outputs: serde_json::json!({"result": "string"}),
                constraints: CapabilityConstraints::default(),
            },
            RequestSource::Operator,
        )
    }

    fn make_refined_spec() -> RefinedSpec {
        RefinedSpec {
            action: SpecAction::Build,
            spec: CapabilitySpec {
                name: "test_tool".into(),
                description: "A test tool".into(),
                inputs: serde_json::json!({"value": "string"}),
                outputs: serde_json::json!({"result": "string"}),
                constraints: CapabilityConstraints::default(),
            },
            design_notes: "test".into(),
            extend_target: None,
            extend_features: None,
            complexity_hint: None,
        }
    }

    /// Builds a StubLlmClient that returns:
    /// 1. Architect response (refine)
    /// 2. Engineer response (build)
    /// 3. QA response (passing)
    /// 4. Red Team response (passing)
    fn make_happy_path_client() -> StubLlmClient {
        let architect_resp = serde_json::json!({
            "action": "build",
            "spec": {
                "name": "test_tool",
                "description": "A test tool",
                "inputs": {"value": "string"},
                "outputs": {"result": "string"},
                "constraints": {"network": [], "storage": [], "secrets": []}
            },
            "design_notes": "Simple tool"
        });

        let engineer_resp = serde_json::json!({
            "source_code": "fn main() {}",
            "wit_definition": "package test:tool;",
            "policy_yaml": "version: \"1.0\"",
            "language": "rust"
        });

        let qa_resp = serde_json::json!({
            "passed": true,
            "tests_run": 5,
            "tests_passed": 5,
            "tests_failed": 0,
            "bug_tickets": []
        });

        let security_resp = serde_json::json!({
            "passed": true,
            "exploits_attempted": 6,
            "exploits_succeeded": 0,
            "bug_tickets": []
        });

        StubLlmClient::new(vec![
            architect_resp.to_string(),
            engineer_resp.to_string(),
            qa_resp.to_string(),
            security_resp.to_string(),
        ])
    }

    #[tokio::test]
    async fn happy_path_builds_artifact() {
        let client = make_happy_path_client();
        let orchestrator = Orchestrator::new(&client);
        let request = make_request();

        let outcome = orchestrator.run(&request).await;
        match outcome {
            PipelineOutcome::Built(artifact) => {
                assert_eq!(artifact.build_iterations, 1);
                assert!(artifact.qa_result.passed);
                assert!(artifact.security_result.passed);
            }
            other => panic!("Expected Built, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn recommend_extend_skips_build() {
        let architect_resp = serde_json::json!({
            "action": "recommend_extend",
            "spec": {
                "name": "test_tool",
                "description": "A test tool",
                "inputs": {},
                "outputs": {},
                "constraints": {"network": [], "storage": [], "secrets": []}
            },
            "design_notes": "Extend existing tool",
            "extend_target": "existing_tool",
            "extend_features": ["new_feature"]
        });

        let client = StubLlmClient::constant(&architect_resp.to_string());
        let orchestrator = Orchestrator::new(&client);
        let request = make_request();

        let outcome = orchestrator.run(&request).await;
        match outcome {
            PipelineOutcome::RecommendExtend { target, features } => {
                assert_eq!(target, "existing_tool");
                assert_eq!(features, vec!["new_feature"]);
            }
            other => panic!("Expected RecommendExtend, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn fix_loop_succeeds_on_second_iteration() {
        // Engineer initial build, QA fails, Engineer fix, QA passes, Red Team passes
        let engineer_resp = serde_json::json!({
            "source_code": "fn main() { /* v1 */ }",
            "wit_definition": "package test:tool;",
            "policy_yaml": "version: \"1.0\"",
            "language": "rust"
        });

        let qa_fail = serde_json::json!({
            "passed": false,
            "tests_run": 5,
            "tests_passed": 3,
            "tests_failed": 2,
            "bug_tickets": [{
                "target": "engineer",
                "ticket_type": "functional_defect",
                "input": {"value": -1},
                "expected": "error response",
                "actual": "panic",
                "remediation_directive": "Add bounds checking"
            }]
        });

        let security_pass = serde_json::json!({
            "passed": true,
            "exploits_attempted": 6,
            "exploits_succeeded": 0,
            "bug_tickets": []
        });

        let engineer_fix = serde_json::json!({
            "source_code": "fn main() { /* v2 fixed */ }",
            "wit_definition": "package test:tool;",
            "policy_yaml": "version: \"1.0\"",
            "language": "rust"
        });

        let qa_pass = serde_json::json!({
            "passed": true,
            "tests_run": 5,
            "tests_passed": 5,
            "tests_failed": 0,
            "bug_tickets": []
        });

        let security_pass2 = serde_json::json!({
            "passed": true,
            "exploits_attempted": 6,
            "exploits_succeeded": 0,
            "bug_tickets": []
        });

        // Sequence: engineer build -> qa fail -> security pass -> engineer fix -> qa pass -> security pass
        let client = StubLlmClient::new(vec![
            engineer_resp.to_string(),
            qa_fail.to_string(),
            security_pass.to_string(),
            engineer_fix.to_string(),
            qa_pass.to_string(),
            security_pass2.to_string(),
        ]);

        let orchestrator = Orchestrator::new(&client);
        let spec = make_refined_spec();

        let outcome = orchestrator.run_from_spec(&spec).await;
        match outcome {
            PipelineOutcome::Built(artifact) => {
                assert_eq!(artifact.build_iterations, 2);
                assert!(artifact.qa_result.passed);
                assert!(artifact.build_output.source_code.contains("v2 fixed"));
            }
            other => panic!("Expected Built, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn circuit_breaker_triggers_after_max_iterations() {
        let engineer_resp = serde_json::json!({
            "source_code": "fn main() { /* broken */ }",
            "wit_definition": "package test:tool;",
            "policy_yaml": "version: \"1.0\"",
            "language": "rust"
        });

        let qa_fail = serde_json::json!({
            "passed": false,
            "tests_run": 5,
            "tests_passed": 0,
            "tests_failed": 5,
            "bug_tickets": [{
                "target": "engineer",
                "ticket_type": "functional_defect",
                "input": {"value": "bad"},
                "expected": "correct",
                "actual": "wrong",
                "remediation_directive": "Fix everything"
            }]
        });

        let security_fail = serde_json::json!({
            "passed": false,
            "exploits_attempted": 6,
            "exploits_succeeded": 3,
            "bug_tickets": [{
                "target": "engineer",
                "ticket_type": "security_vulnerability",
                "input": {"exploit": "payload"},
                "expected": "blocked",
                "actual": "succeeded",
                "remediation_directive": "Add validation"
            }]
        });

        // Each iteration: engineer -> qa_fail -> security_fail -> engineer fix (3 iterations)
        // Iteration 1: build(0) -> qa(1) -> sec(2) -> fix(3)
        // Iteration 2: qa(4) -> sec(5) -> fix(6)
        // Iteration 3: qa(7) -> sec(8) -> circuit breaker
        let client = StubLlmClient::new(vec![
            engineer_resp.to_string(),
            qa_fail.to_string(),
            security_fail.to_string(),
            engineer_resp.to_string(), // fix attempt 1
            qa_fail.to_string(),
            security_fail.to_string(),
            engineer_resp.to_string(), // fix attempt 2
            qa_fail.to_string(),
            security_fail.to_string(),
        ]);

        let orchestrator = Orchestrator::new(&client)
            .with_circuit_breaker_policy(crate::config::CircuitBreakerPolicy::Fail);
        let spec = make_refined_spec();

        let outcome = orchestrator.run_from_spec(&spec).await;
        match outcome {
            PipelineOutcome::Failed(PipelineError::CircuitBreaker { attempts, summary }) => {
                assert_eq!(attempts, 3);
                assert!(!summary.is_empty());
            }
            other => panic!("Expected Failed(CircuitBreaker), got {:?}", other),
        }
    }

    #[tokio::test]
    async fn circuit_breaker_proceeds_when_policy_is_proceed() {
        let engineer_resp = serde_json::json!({
            "source_code": "fn main() {}",
            "wit_definition": "package test:tool;",
            "policy_yaml": "version: \"1.0\"",
            "language": "rust"
        });
        let qa_fail = serde_json::json!({
            "passed": false, "tests_run": 1, "tests_passed": 0, "tests_failed": 1,
            "bug_tickets": [{"target":"engineer","ticket_type":"functional_defect",
                "input":{},"expected":"ok","actual":"fail","remediation_directive":"fix it"}]
        });
        let security_ok = serde_json::json!({
            "passed": true, "exploits_attempted": 0, "exploits_succeeded": 0, "bug_tickets": []
        });
        // 3 iterations: build + (qa_fail + security_ok + fix) × 3
        let client = StubLlmClient::new(vec![
            engineer_resp.to_string(),
            qa_fail.to_string(), security_ok.to_string(), engineer_resp.to_string(),
            qa_fail.to_string(), security_ok.to_string(), engineer_resp.to_string(),
            qa_fail.to_string(), security_ok.to_string(),
        ]);
        let orchestrator = Orchestrator::new(&client)
            .with_circuit_breaker_policy(crate::config::CircuitBreakerPolicy::Proceed);
        let spec = make_refined_spec();

        let outcome = orchestrator.run_from_spec(&spec).await;
        match outcome {
            PipelineOutcome::Built(artifact) => {
                assert!(artifact.escalated);
                assert!(!artifact.escalated_tickets.is_empty());
            }
            other => panic!("Expected Built (escalated), got {:?}", other),
        }
    }

    #[tokio::test]
    async fn run_from_spec_with_recommend_extend() {
        let spec = RefinedSpec {
            action: SpecAction::RecommendExtend,
            spec: CapabilitySpec {
                name: "test".into(),
                description: "test".into(),
                inputs: serde_json::Value::Null,
                outputs: serde_json::Value::Null,
                constraints: CapabilityConstraints::default(),
            },
            design_notes: "extend instead".into(),
            extend_target: Some("existing".into()),
            extend_features: Some(vec!["feat_a".into()]),
            complexity_hint: None,
        };

        let client = StubLlmClient::constant("unused");
        let orchestrator = Orchestrator::new(&client);

        let outcome = orchestrator.run_from_spec(&spec).await;
        match outcome {
            PipelineOutcome::RecommendExtend { target, features } => {
                assert_eq!(target, "existing");
                assert_eq!(features, vec!["feat_a"]);
            }
            other => panic!("Expected RecommendExtend, got {:?}", other),
        }
    }
}
