/// End-to-end integration tests for girt-runtime.
///
/// These tests compile real WASM components with cargo-component and execute
/// them through the LifecycleManager. They require:
///   - `cargo-component` installed (`cargo install cargo-component`)
///   - `wasm32-wasip1` target installed (`rustup target add wasm32-wasip1`)
///
/// Run with: `cargo test -p girt-runtime --test integration_test -- --include-ignored`

use girt_runtime::{ComponentMeta, LifecycleManager};
use girt_pipeline::compiler::{CompileInput, WasmCompiler};
use std::time::{SystemTime, UNIX_EPOCH};

/// Celsius-to-Fahrenheit converter — the canonical GIRT smoke test tool.
const CELSIUS_TO_FAHRENHEIT_SRC: &str = r#"
#[allow(warnings)]
mod bindings;

use bindings::Guest;

struct Component;

impl Guest for Component {
    fn run(input: String) -> Result<String, String> {
        let args: serde_json::Value = serde_json::from_str(&input)
            .map_err(|e| format!("invalid JSON: {e}"))?;

        let celsius = args["celsius"]
            .as_f64()
            .ok_or_else(|| "missing or non-numeric 'celsius' field".to_string())?;

        let fahrenheit = celsius * 9.0 / 5.0 + 32.0;

        Ok(serde_json::json!({ "fahrenheit": fahrenheit }).to_string())
    }
}

bindings::export!(Component with_types_in bindings);
"#;

#[tokio::test]
#[ignore = "requires cargo-component and wasm32-wasip1 target"]
async fn smoke_test_celsius_to_fahrenheit() {
    // 1. Compile the tool
    let compiler = WasmCompiler::new();
    let input = CompileInput {
        source_code: CELSIUS_TO_FAHRENHEIT_SRC.into(),
        wit_definition: String::new(), // uses default girt-tool world
        tool_name: "celsius_to_fahrenheit".into(),
        tool_version: "0.1.0".into(),
    };

    let compiled = compiler.compile(&input).await
        .expect("WasmCompiler::compile failed — is cargo-component installed?");

    assert!(compiled.wasm_path.exists(), "wasm file should exist after compilation");
    println!("Compiled: {}", compiled.wasm_path.display());

    // 2. Load into LifecycleManager
    let tmp = tempfile::tempdir().unwrap();
    let manager = LifecycleManager::new(Some(tmp.path().to_path_buf()))
        .expect("LifecycleManager::new failed");

    let meta = ComponentMeta {
        component_id: "celsius_to_fahrenheit@0.1.0".into(),
        tool_name: "celsius_to_fahrenheit".into(),
        description: "Convert Celsius to Fahrenheit".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "celsius": { "type": "number", "description": "Temperature in Celsius" }
            },
            "required": ["celsius"]
        }),
        wasm_hash: String::new(),
        built_at: SystemTime::now()
            .duration_since(UNIX_EPOCH).unwrap().as_millis() as u64,
    };

    manager.load_component(&compiled.wasm_path, meta).await
        .expect("load_component failed");

    // 3. Verify it appears in list_tools
    let tools = manager.list_tools().await;
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].tool_name, "celsius_to_fahrenheit");
    println!("list_tools: {} tool(s)", tools.len());

    // 4. Call the tool
    let args = serde_json::json!({ "celsius": 100.0 });
    let result = manager.call_tool("celsius_to_fahrenheit", &args).await
        .expect("call_tool failed");

    println!("Result: {result}");

    let fahrenheit = result["fahrenheit"].as_f64()
        .expect("result should have 'fahrenheit' field");

    assert!(
        (fahrenheit - 212.0).abs() < 0.001,
        "100°C should be 212°F, got {fahrenheit}"
    );

    // 5. Verify error handling
    let bad_args = serde_json::json!({ "wrong_key": 42 });
    let err = manager.call_tool("celsius_to_fahrenheit", &bad_args).await;
    assert!(err.is_err(), "call with bad args should return Err");
    println!("Error handling: {:?}", err.unwrap_err());

    // 6. Verify persistence — reload from disk
    let manager2 = LifecycleManager::new(Some(tmp.path().to_path_buf()))
        .expect("second manager failed");
    manager2.load_persisted().await;

    let tools2 = manager2.list_tools().await;
    assert_eq!(tools2.len(), 1, "persisted tool should be restored");
    println!("Persistence: {} tool(s) restored", tools2.len());

    let result2 = manager2.call_tool("celsius_to_fahrenheit", &serde_json::json!({"celsius": 0.0})).await
        .expect("call after persist failed");
    let f2 = result2["fahrenheit"].as_f64().unwrap();
    assert!((f2 - 32.0).abs() < 0.001, "0°C should be 32°F, got {f2}");

    println!("All smoke test assertions passed.");
}

#[tokio::test]
#[ignore = "requires cargo-component and wasm32-wasip1 target"]
async fn smoke_test_tool_not_found_returns_error() {
    let tmp = tempfile::tempdir().unwrap();
    let manager = LifecycleManager::new(Some(tmp.path().to_path_buf())).unwrap();

    let err = manager.call_tool("nonexistent_tool", &serde_json::json!({})).await;
    assert!(matches!(err, Err(girt_runtime::RuntimeError::ToolNotFound(_))));
}
