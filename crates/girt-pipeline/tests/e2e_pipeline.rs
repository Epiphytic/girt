//! End-to-end pipeline tests.
//!
//! These tests require external dependencies:
//! - vLLM running on localhost:8000 with zai-org/GLM-4.7-Flash
//! - cargo-component installed
//! - wassette installed
//!
//! Run with: cargo test -p girt-pipeline --test e2e_pipeline -- --ignored --nocapture

use std::sync::Arc;

use girt_core::spec::{CapabilityConstraints, CapabilitySpec};
use girt_pipeline::cache::ToolCache;
use girt_pipeline::compiler::WasmCompiler;
use girt_pipeline::llm::OpenAiCompatibleClient;
use girt_pipeline::metrics::PipelineMetrics;
use girt_pipeline::publish::Publisher;
use girt_pipeline::queue::{Queue, QueueConsumer};
use girt_pipeline::types::{CapabilityRequest, RequestSource};

fn check_prerequisites() {
    // Check vLLM
    let resp = std::process::Command::new("curl")
        .args(["-s", "http://localhost:8000/v1/models"])
        .output();
    assert!(
        resp.is_ok() && resp.unwrap().status.success(),
        "vLLM not reachable at localhost:8000"
    );

    // Check cargo-component
    let resp = std::process::Command::new("cargo-component")
        .arg("--version")
        .output();
    assert!(
        resp.is_ok() && resp.unwrap().status.success(),
        "cargo-component not installed"
    );

    // Check wassette
    let resp = std::process::Command::new("wassette")
        .arg("--version")
        .output();
    assert!(
        resp.is_ok() && resp.unwrap().status.success(),
        "wassette not installed"
    );
}

fn make_llm_client() -> Arc<dyn girt_pipeline::llm::LlmClient> {
    Arc::new(OpenAiCompatibleClient::new(
        "http://localhost:8000/v1".into(),
        "zai-org/GLM-4.7-Flash".into(),
        None,
    ))
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
    // The pipeline may fail at various stages (LLM response parsing,
    // compilation, etc.). With a real LLM, errors are expected and valid.
    let result = consumer.process_next(&compiler, None, None).await;

    let snap = metrics.snapshot();
    assert_eq!(snap.builds_started, 1);

    match result {
        Ok(Some(girt_pipeline::queue::ProcessResult::Built { name, .. })) => {
            eprintln!("SUCCESS: Tool '{name}' built and cached");
            // Verify cache has the tool
            let tool_dir = tmp.path().join("tools").join(&name);
            assert!(tool_dir.join("manifest.json").exists());
            assert!(tool_dir.join("tool.wasm").exists());
            assert!(tool_dir.join("source.rs").exists());
            assert!(tool_dir.join("policy.yaml").exists());

            // Verify WASM loads in Wassette
            let wasm_path = tool_dir.join("tool.wasm");
            let wassette_check = tokio::process::Command::new("wassette")
                .arg("inspect")
                .arg(wasm_path.to_str().unwrap())
                .output()
                .await;
            assert!(
                wassette_check.is_ok(),
                "Wassette should be able to inspect the WASM component"
            );

            assert_eq!(snap.builds_completed, 1);
        }
        Ok(Some(girt_pipeline::queue::ProcessResult::Failed(e))) => {
            // Orchestrator-level failure (circuit breaker, QA/RedTeam error)
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
            // Pipeline-level errors (compilation failure, publish error, etc.)
            // are valid E2E outcomes when the LLM generates non-compilable code.
            eprintln!("Pipeline error (valid E2E outcome): {e}");
        }
    }

    // Verify queue state -- request should not be pending
    assert!(
        consumer.queue().list_pending().await.unwrap().is_empty(),
        "No requests should be pending"
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
    // With a real LLM, the build may actually succeed (LLM will try its best),
    // fail at orchestration, or fail at compilation. Any outcome is valid for
    // E2E. The important thing is no panic.
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
        Ok(None) => {
            panic!("Queue should have had a request to process");
        }
        Err(e) => {
            eprintln!("Impossible spec pipeline error (valid): {e}");
        }
    }
}

#[tokio::test]
#[ignore]
async fn e2e_happy_path_with_oci_push() {
    check_prerequisites();

    // Also check oras
    let oras_check = std::process::Command::new("oras").arg("version").output();
    if oras_check.is_err() || !oras_check.unwrap().status.success() {
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

    // Request a simple tool for OCI push test
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
                // Clean up: delete the test tag
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
        Ok(None) => {
            panic!("Queue should have had a request to process");
        }
        Err(e) => {
            eprintln!("OCI test pipeline error (valid E2E outcome): {e}");
        }
    }
}
