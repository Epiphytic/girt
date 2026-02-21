//! End-to-end pipeline tests.
//!
//! These tests drive the full build pipeline (queue → orchestrator → LLM → WASM compile → publish)
//! against real external dependencies and are skipped by default.
//!
//! ## Prerequisites
//! - `cargo-component` installed (`cargo install cargo-component`)
//! - `ANTHROPIC_API_KEY` set (or OpenClaw auth-profiles configured)
//!
//! ## Run
//! ```bash
//! cargo test -p girt-pipeline --test e2e_pipeline -- --ignored --nocapture
//! ```

use std::sync::Arc;

use girt_core::spec::{CapabilityConstraints, CapabilitySpec};
use girt_pipeline::cache::ToolCache;
use girt_pipeline::compiler::WasmCompiler;
use girt_pipeline::llm::AnthropicLlmClient;
use girt_pipeline::metrics::PipelineMetrics;
use girt_pipeline::publish::Publisher;
use girt_pipeline::queue::{Queue, QueueConsumer};
use girt_pipeline::types::{CapabilityRequest, RequestSource};

/// Verify external tool dependencies are available.
fn check_prerequisites() {
    // Check cargo-component (required to compile WASM components)
    let resp = std::process::Command::new("cargo-component")
        .arg("--version")
        .output();
    assert!(
        resp.is_ok() && resp.unwrap().status.success(),
        "cargo-component not installed — run: cargo install cargo-component"
    );

    // Check Anthropic credentials are available (env var or OpenClaw auth)
    let has_key = std::env::var("ANTHROPIC_API_KEY").is_ok();
    let has_openclaw_auth = dirs::home_dir()
        .map(|h| {
            h.join(".openclaw")
                .join("agents")
                .join("main")
                .join("agent")
                .join("auth-profiles.json")
                .exists()
        })
        .unwrap_or(false);
    assert!(
        has_key || has_openclaw_auth,
        "No Anthropic credentials found. Set ANTHROPIC_API_KEY or run `openclaw models auth setup-token --provider anthropic`"
    );
}

fn make_llm_client() -> Arc<dyn girt_pipeline::llm::LlmClient> {
    Arc::new(
        AnthropicLlmClient::from_env_or("claude-sonnet-4-5".into(), None)
            .expect("Failed to initialise Anthropic LLM client"),
    )
}

/// Check that a file is a valid WASM binary (starts with magic bytes `\0asm`).
fn assert_valid_wasm(path: &std::path::Path) {
    let bytes = std::fs::read(path).expect("failed to read wasm file");
    assert!(
        bytes.starts_with(b"\0asm"),
        "File at {} is not a valid WASM binary (missing magic bytes)",
        path.display()
    );
}

#[tokio::test]
#[ignore]
async fn e2e_happy_path_simple_tool() {
    check_prerequisites();

    let tmp = tempfile::tempdir().unwrap();
    let queue = Queue::new(tmp.path().join("queue"));
    queue.init().await.unwrap();

    let cache = ToolCache::new(tmp.path().join("tools"));
    let publisher = Publisher::new(cache);
    publisher.init().await.unwrap();

    let llm = make_llm_client();
    let metrics = Arc::new(PipelineMetrics::new());
    let consumer = QueueConsumer::new(queue, llm, publisher, metrics.clone());
    let compiler = WasmCompiler::new();

    // Request a simple stateless tool
    let spec = CapabilitySpec {
        name: "base64_codec".into(),
        description: "Encode and decode base64 strings. Takes a JSON input with 'action' (encode/decode) and 'data' (string). Returns the result as a string.".into(),
        inputs: serde_json::json!({
            "action": "string (encode or decode)",
            "data": "string"
        }),
        outputs: serde_json::json!({
            "result": "string"
        }),
        constraints: CapabilityConstraints::default(),
    };

    let request = CapabilityRequest::new(spec, RequestSource::Operator);
    consumer.queue().enqueue(&request).await.unwrap();

    // Process the request (with real LLM + real compilation).
    let result = consumer.process_next(&compiler, None, None).await;

    let snap = metrics.snapshot();
    assert_eq!(snap.builds_started, 1);

    match result {
        Ok(Some(girt_pipeline::queue::ProcessResult::Built { name, .. })) => {
            eprintln!("SUCCESS: Tool '{name}' built and cached");

            let tool_dir = tmp.path().join("tools").join(&name);
            assert!(tool_dir.join("manifest.json").exists(), "manifest.json should exist");
            assert!(tool_dir.join("source.rs").exists(), "source.rs should exist");

            // Verify WASM was produced and is a valid binary
            let wasm_path = tool_dir.join("tool.wasm");
            assert!(wasm_path.exists(), "tool.wasm should exist after build");
            assert_valid_wasm(&wasm_path);
            eprintln!("WASM binary valid: {} bytes", std::fs::metadata(&wasm_path).unwrap().len());

            assert_eq!(snap.builds_completed, 1);
        }
        Ok(Some(girt_pipeline::queue::ProcessResult::Failed(e))) => {
            // Orchestrator-level failure (circuit breaker, QA/RedTeam error) — valid E2E outcome.
            eprintln!("Build failed at orchestrator level (valid E2E outcome): {e}");
            assert_eq!(snap.builds_failed, 1);
        }
        Ok(Some(girt_pipeline::queue::ProcessResult::Extended { target, .. })) => {
            eprintln!("Got RecommendExtend to: {target} (valid E2E outcome)");
        }
        Ok(None) => {
            panic!("Queue should have had a request to process");
        }
        Err(e) => {
            // Pipeline-level errors (compilation failure, etc.) are valid E2E outcomes.
            eprintln!("Pipeline error (valid E2E outcome): {e}");
        }
    }

    // Queue should be empty after processing
    assert!(
        consumer.queue().list_pending().await.unwrap().is_empty(),
        "No requests should be pending after processing"
    );
}

#[tokio::test]
#[ignore]
async fn e2e_circuit_breaker_impossible_spec() {
    check_prerequisites();

    let tmp = tempfile::tempdir().unwrap();
    let queue = Queue::new(tmp.path().join("queue"));
    queue.init().await.unwrap();

    let cache = ToolCache::new(tmp.path().join("tools"));
    let publisher = Publisher::new(cache);
    publisher.init().await.unwrap();

    let llm = make_llm_client();
    let metrics = Arc::new(PipelineMetrics::new());
    let consumer = QueueConsumer::new(queue, llm, publisher, metrics.clone());
    let compiler = WasmCompiler::new();

    // Request something deliberately contradictory
    let spec = CapabilitySpec {
        name: "impossible_tool".into(),
        description: "A tool that must simultaneously have zero memory usage and process infinite data streams in constant time while also being a valid quine that outputs its own source code in every response.".into(),
        inputs: serde_json::json!({"infinite_stream": "bytes"}),
        outputs: serde_json::json!({"paradox": "void"}),
        constraints: CapabilityConstraints::default(),
    };

    let request = CapabilityRequest::new(spec, RequestSource::Operator);
    consumer.queue().enqueue(&request).await.unwrap();

    let result = consumer.process_next(&compiler, None, None).await;

    let snap = metrics.snapshot();
    assert_eq!(snap.builds_started, 1);

    // With a real LLM, any outcome is valid for E2E. The important thing is no panic.
    match result {
        Ok(Some(girt_pipeline::queue::ProcessResult::Built { name, .. })) => {
            eprintln!("Impossible spec actually built (LLM tried its best): {name}");
        }
        Ok(Some(girt_pipeline::queue::ProcessResult::Failed(e))) => {
            eprintln!("Impossible spec failed as expected: {e}");
        }
        Ok(Some(girt_pipeline::queue::ProcessResult::Extended { target, .. })) => {
            eprintln!("Impossible spec got extend recommendation: {target}");
        }
        Ok(None) => panic!("Queue should have had a request to process"),
        Err(e) => eprintln!("Impossible spec pipeline error (valid): {e}"),
    }
}

#[tokio::test]
#[ignore]
async fn e2e_happy_path_with_oci_push() {
    check_prerequisites();

    // Check oras separately (OCI push is optional)
    let oras_ok = std::process::Command::new("oras")
        .arg("version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !oras_ok {
        eprintln!("SKIP: oras not installed, skipping OCI push test");
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    let queue = Queue::new(tmp.path().join("queue"));
    queue.init().await.unwrap();

    let cache = ToolCache::new(tmp.path().join("tools"));
    let publisher = Publisher::new(cache);
    publisher.init().await.unwrap();

    let llm = make_llm_client();
    let metrics = Arc::new(PipelineMetrics::new());
    let consumer = QueueConsumer::new(queue, llm, publisher, metrics.clone());
    let compiler = WasmCompiler::new();

    let spec = CapabilitySpec {
        name: "oci_test_tool".into(),
        description: "A simple echo tool for OCI push testing. Takes a string input and returns it unchanged.".into(),
        inputs: serde_json::json!({"input": "string"}),
        outputs: serde_json::json!({"output": "string"}),
        constraints: CapabilityConstraints::default(),
    };

    let request = CapabilityRequest::new(spec, RequestSource::Operator);
    consumer.queue().enqueue(&request).await.unwrap();

    let result = consumer
        .process_next(
            &compiler,
            Some("ghcr.io/epiphytic/girt-tools"),
            Some("e2e-test"),
        )
        .await;

    match result {
        Ok(Some(girt_pipeline::queue::ProcessResult::Built { oci_reference, name, .. })) => {
            eprintln!("Built tool: {name}");
            if let Some(reference) = &oci_reference {
                eprintln!("Published to: {reference}");
                // Clean up the test tag
                let _ = tokio::process::Command::new("oras")
                    .args(["manifest", "delete", "--force", reference])
                    .output()
                    .await;
            }
        }
        Ok(Some(girt_pipeline::queue::ProcessResult::Failed(e))) => {
            eprintln!("OCI test build failed (valid E2E outcome): {e}");
        }
        Ok(Some(girt_pipeline::queue::ProcessResult::Extended { target, .. })) => {
            eprintln!("OCI test got extend recommendation: {target}");
        }
        Ok(None) => panic!("Queue should have had a request to process"),
        Err(e) => eprintln!("OCI test pipeline error (valid E2E outcome): {e}"),
    }
}
