use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::compiler::WasmCompiler;
use crate::error::PipelineError;
use crate::llm::LlmClient;
use crate::metrics::PipelineMetrics;
use crate::orchestrator::{Orchestrator, PipelineOutcome};
use crate::publish::Publisher;
use crate::types::{CapabilityRequest, RequestStatus};

/// File-based queue for capability requests.
///
/// Stores requests as JSON files in a directory structure:
/// ```text
/// base_dir/
///   pending/     -- new requests waiting to be processed
///   in_progress/ -- requests currently being built
///   completed/   -- successfully built requests
///   failed/      -- requests that failed after max retries
/// ```
///
/// Atomic file moves (rename) between directories prevent race conditions.
pub struct Queue {
    base_dir: PathBuf,
}

impl Queue {
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    /// Default queue location: ~/.girt/queue/
    pub fn default_path() -> PathBuf {
        dirs_path().join("queue")
    }

    fn pending_dir(&self) -> PathBuf {
        self.base_dir.join("pending")
    }

    fn in_progress_dir(&self) -> PathBuf {
        self.base_dir.join("in_progress")
    }

    fn completed_dir(&self) -> PathBuf {
        self.base_dir.join("completed")
    }

    fn failed_dir(&self) -> PathBuf {
        self.base_dir.join("failed")
    }

    /// Initialize the queue directory structure.
    pub async fn init(&self) -> Result<(), PipelineError> {
        tokio::fs::create_dir_all(self.pending_dir()).await?;
        tokio::fs::create_dir_all(self.in_progress_dir()).await?;
        tokio::fs::create_dir_all(self.completed_dir()).await?;
        tokio::fs::create_dir_all(self.failed_dir()).await?;
        Ok(())
    }

    /// Enqueue a new capability request.
    pub async fn enqueue(&self, request: &CapabilityRequest) -> Result<(), PipelineError> {
        let filename = format!("{}.json", request.id);
        let path = self.pending_dir().join(&filename);
        let json = serde_json::to_string_pretty(request)?;
        tokio::fs::write(&path, json).await?;
        tracing::info!(id = %request.id, path = %path.display(), "Request enqueued");
        Ok(())
    }

    /// Claim the next pending request by atomically moving it to in_progress.
    pub async fn claim_next(&self) -> Result<Option<CapabilityRequest>, PipelineError> {
        let mut entries = tokio::fs::read_dir(self.pending_dir()).await?;

        // Find the first .json file (sorted by name for deterministic ordering)
        let mut files: Vec<PathBuf> = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "json") {
                files.push(path);
            }
        }
        files.sort();

        let Some(source_path) = files.first() else {
            return Ok(None);
        };

        // Read the request
        let content = tokio::fs::read_to_string(source_path).await?;
        let mut request: CapabilityRequest = serde_json::from_str(&content)?;
        request.status = RequestStatus::InProgress;

        // Atomic move to in_progress
        let filename = source_path
            .file_name()
            .ok_or_else(|| PipelineError::QueueError("Invalid filename".into()))?;
        let dest_path = self.in_progress_dir().join(filename);
        tokio::fs::rename(source_path, &dest_path).await?;

        // Update the file with new status
        let json = serde_json::to_string_pretty(&request)?;
        tokio::fs::write(&dest_path, json).await?;

        tracing::info!(id = %request.id, "Request claimed");
        Ok(Some(request))
    }

    /// Mark a request as completed.
    pub async fn complete(&self, request: &CapabilityRequest) -> Result<(), PipelineError> {
        self.move_request(request, &self.in_progress_dir(), &self.completed_dir())
            .await
    }

    /// Mark a request as failed.
    pub async fn fail(&self, request: &CapabilityRequest) -> Result<(), PipelineError> {
        self.move_request(request, &self.in_progress_dir(), &self.failed_dir())
            .await
    }

    /// List pending request IDs.
    pub async fn list_pending(&self) -> Result<Vec<String>, PipelineError> {
        self.list_dir(&self.pending_dir()).await
    }

    /// List in-progress request IDs.
    pub async fn list_in_progress(&self) -> Result<Vec<String>, PipelineError> {
        self.list_dir(&self.in_progress_dir()).await
    }

    async fn move_request(
        &self,
        request: &CapabilityRequest,
        from_dir: &Path,
        to_dir: &Path,
    ) -> Result<(), PipelineError> {
        let filename = format!("{}.json", request.id);
        let source = from_dir.join(&filename);
        let dest = to_dir.join(&filename);
        tokio::fs::rename(&source, &dest).await?;
        tracing::info!(
            id = %request.id,
            to = %to_dir.display(),
            "Request moved"
        );
        Ok(())
    }

    async fn list_dir(&self, dir: &Path) -> Result<Vec<String>, PipelineError> {
        let mut entries = tokio::fs::read_dir(dir).await?;
        let mut ids = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "json")
                && let Some(stem) = path.file_stem()
            {
                ids.push(stem.to_string_lossy().into_owned());
            }
        }
        ids.sort();
        Ok(ids)
    }
}

#[derive(Debug)]
pub enum ProcessResult {
    Built {
        name: String,
        oci_reference: Option<String>,
    },
    Extended {
        target: String,
        features: Vec<String>,
    },
    Failed(PipelineError),
}

pub struct QueueConsumer {
    queue: Queue,
    llm: Arc<dyn LlmClient>,
    publisher: Publisher,
    metrics: Arc<PipelineMetrics>,
}

impl QueueConsumer {
    pub fn new(
        queue: Queue,
        llm: Arc<dyn LlmClient>,
        publisher: Publisher,
        metrics: Arc<PipelineMetrics>,
    ) -> Self {
        Self {
            queue,
            llm,
            publisher,
            metrics,
        }
    }

    pub fn queue(&self) -> &Queue {
        &self.queue
    }

    pub async fn process_next(
        &self,
        compiler: &WasmCompiler,
        registry_url: Option<&str>,
        tag: Option<&str>,
    ) -> Result<Option<ProcessResult>, PipelineError> {
        let request = match self.queue.claim_next().await? {
            Some(r) => r,
            None => return Ok(None),
        };

        self.metrics.record_build_started();
        tracing::info!(id = %request.id, name = %request.spec.name, "Processing request");

        let orchestrator = Orchestrator::new(self.llm.as_ref());
        let outcome = orchestrator.run(&request).await;

        match outcome {
            PipelineOutcome::Built(artifact) => {
                let compile_input = crate::compiler::CompileInput {
                    source_code: artifact.build_output.source_code.clone(),
                    wit_definition: artifact.build_output.wit_definition.clone(),
                    tool_name: artifact.spec.name.clone(),
                    tool_version: "0.1.0".into(),
                };

                let compile_output = compiler.compile(&compile_input).await?;

                self.publisher
                    .publish_with_wasm(&artifact, &compile_output.wasm_path)
                    .await?;

                let oci_reference = if let (Some(url), Some(t)) = (registry_url, tag) {
                    Some(
                        self.publisher
                            .push_oci(&artifact, &compile_output.wasm_path, url, t)
                            .await?,
                    )
                } else {
                    None
                };

                self.queue.complete(&request).await?;
                self.metrics
                    .record_build_completed(artifact.build_iterations);

                Ok(Some(ProcessResult::Built {
                    name: artifact.spec.name.clone(),
                    oci_reference,
                }))
            }
            PipelineOutcome::RecommendExtend { target, features } => {
                self.queue.complete(&request).await?;
                self.metrics.record_recommend_extend();
                Ok(Some(ProcessResult::Extended { target, features }))
            }
            PipelineOutcome::Failed(e) => {
                self.queue.fail(&request).await?;
                self.metrics.record_build_failed();
                Ok(Some(ProcessResult::Failed(e)))
            }
        }
    }

    pub async fn process_next_no_compile(
        &self,
    ) -> Result<Option<ProcessResult>, PipelineError> {
        let request = match self.queue.claim_next().await? {
            Some(r) => r,
            None => return Ok(None),
        };

        self.metrics.record_build_started();

        let orchestrator = Orchestrator::new(self.llm.as_ref());
        let outcome = orchestrator.run(&request).await;

        match outcome {
            PipelineOutcome::Built(artifact) => {
                self.publisher.publish(&artifact).await?;
                self.queue.complete(&request).await?;
                self.metrics
                    .record_build_completed(artifact.build_iterations);
                Ok(Some(ProcessResult::Built {
                    name: artifact.spec.name.clone(),
                    oci_reference: None,
                }))
            }
            PipelineOutcome::RecommendExtend { target, features } => {
                self.queue.complete(&request).await?;
                self.metrics.record_recommend_extend();
                Ok(Some(ProcessResult::Extended { target, features }))
            }
            PipelineOutcome::Failed(e) => {
                self.queue.fail(&request).await?;
                self.metrics.record_build_failed();
                Ok(Some(ProcessResult::Failed(e)))
            }
        }
    }
}

fn dirs_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".girt")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CapabilityRequest, RequestSource};
    use girt_core::spec::{CapabilityConstraints, CapabilitySpec};
    use tempfile::TempDir;

    fn make_request(name: &str) -> CapabilityRequest {
        CapabilityRequest::new(
            CapabilitySpec {
                name: name.into(),
                description: format!("Test tool: {name}"),
                inputs: serde_json::Value::Null,
                outputs: serde_json::Value::Null,
                constraints: CapabilityConstraints::default(),
            },
            RequestSource::Operator,
        )
    }

    #[tokio::test]
    async fn enqueue_and_claim() {
        let tmp = TempDir::new().unwrap();
        let queue = Queue::new(tmp.path().to_path_buf());
        queue.init().await.unwrap();

        let request = make_request("test_tool");
        let id = request.id.clone();

        queue.enqueue(&request).await.unwrap();

        let pending = queue.list_pending().await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0], id);

        let claimed = queue.claim_next().await.unwrap();
        assert!(claimed.is_some());
        let claimed = claimed.unwrap();
        assert_eq!(claimed.id, id);
        assert_eq!(claimed.status, RequestStatus::InProgress);

        // Pending should be empty, in_progress should have one
        assert!(queue.list_pending().await.unwrap().is_empty());
        assert_eq!(queue.list_in_progress().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn complete_moves_to_completed() {
        let tmp = TempDir::new().unwrap();
        let queue = Queue::new(tmp.path().to_path_buf());
        queue.init().await.unwrap();

        let request = make_request("test_tool");
        queue.enqueue(&request).await.unwrap();

        let claimed = queue.claim_next().await.unwrap().unwrap();
        queue.complete(&claimed).await.unwrap();

        assert!(queue.list_in_progress().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn fail_moves_to_failed() {
        let tmp = TempDir::new().unwrap();
        let queue = Queue::new(tmp.path().to_path_buf());
        queue.init().await.unwrap();

        let request = make_request("bad_tool");
        queue.enqueue(&request).await.unwrap();

        let claimed = queue.claim_next().await.unwrap().unwrap();
        queue.fail(&claimed).await.unwrap();

        assert!(queue.list_in_progress().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn claim_returns_none_when_empty() {
        let tmp = TempDir::new().unwrap();
        let queue = Queue::new(tmp.path().to_path_buf());
        queue.init().await.unwrap();

        let result = queue.claim_next().await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn multiple_requests_processed_in_order() {
        let tmp = TempDir::new().unwrap();
        let queue = Queue::new(tmp.path().to_path_buf());
        queue.init().await.unwrap();

        let r1 = make_request("alpha");
        let r2 = make_request("beta");
        let r3 = make_request("gamma");

        queue.enqueue(&r1).await.unwrap();
        queue.enqueue(&r2).await.unwrap();
        queue.enqueue(&r3).await.unwrap();

        assert_eq!(queue.list_pending().await.unwrap().len(), 3);

        // Claims should process in sorted filename order
        let c1 = queue.claim_next().await.unwrap().unwrap();
        queue.complete(&c1).await.unwrap();

        let c2 = queue.claim_next().await.unwrap().unwrap();
        queue.complete(&c2).await.unwrap();

        let c3 = queue.claim_next().await.unwrap().unwrap();
        queue.complete(&c3).await.unwrap();

        assert!(queue.list_pending().await.unwrap().is_empty());
        assert!(queue.list_in_progress().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn queue_consumer_processes_happy_path() {
        use crate::cache::ToolCache;
        use crate::llm::StubLlmClient;
        use crate::metrics::PipelineMetrics;
        use crate::publish::Publisher;
        use std::sync::Arc;

        let tmp = TempDir::new().unwrap();
        let queue = Queue::new(tmp.path().join("queue"));
        queue.init().await.unwrap();

        let cache = ToolCache::new(tmp.path().join("tools"));
        let publisher = Publisher::new(cache);
        publisher.init().await.unwrap();

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
            "passed": true, "tests_run": 1, "tests_passed": 1,
            "tests_failed": 0, "bug_tickets": []
        });
        let security_resp = serde_json::json!({
            "passed": true, "exploits_attempted": 1,
            "exploits_succeeded": 0, "bug_tickets": []
        });

        let llm = Arc::new(StubLlmClient::new(vec![
            architect_resp.to_string(),
            engineer_resp.to_string(),
            qa_resp.to_string(),
            security_resp.to_string(),
        ]));

        let metrics = Arc::new(PipelineMetrics::new());
        let consumer = QueueConsumer::new(queue, llm, publisher, metrics.clone());

        let request = make_request("test_tool");
        consumer.queue().enqueue(&request).await.unwrap();

        let result = consumer.process_next_no_compile().await.unwrap();
        assert!(result.is_some());

        let snap = metrics.snapshot();
        assert_eq!(snap.builds_started, 1);
        assert_eq!(snap.builds_completed, 1);
    }
}
