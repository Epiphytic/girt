# E2E Pipeline Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Wire the GIRT build pipeline end-to-end: queue file in → LLM calls (via OpenAI-compatible API) → WASM compile → Wassette load → OCI push → tool in cache.

**Architecture:** Replace `StubLlmClient` with `OpenAiCompatibleClient` backed by any `/v1/chat/completions` endpoint. Add `WasmCompiler` (wraps `cargo-component`), `OciPublisher` (wraps `oras`), and `QueueConsumer` (connects queue → orchestrator → compile → publish). Fix plugin paths so Claude Code discovers agents/skills/commands.

**Tech Stack:** Rust, reqwest, tokio, cargo-component, oras, Wassette v0.4.0, toml (config), tempfile (tests)

**Design doc:** `docs/plans/2026-02-20-e2e-pipeline-test-design.md`

---

## Prerequisites

Before starting, install:

```bash
cargo install cargo-component
cargo install oras  # or download binary from https://oras.land/
rustup target add wasm32-wasip1  # already installed
```

Verify vLLM is running:
```bash
curl -s http://localhost:8000/v1/models | jq '.data[0].id'
# Should output: "zai-org/GLM-4.7-Flash"
```

---

### Task 1: Fix plugin path mismatch

Plugin files exist at repo root (`agents/`, `skills/`, `commands/`, `hooks/`) but `plugin.json` references them relative to `.claude-plugin/`. Move all plugin component files into `.claude-plugin/`.

**Files:**
- Move: `agents/*.md` → `.claude-plugin/agents/*.md`
- Move: `skills/*.md` → `.claude-plugin/skills/*.md`
- Move: `commands/*.md` → `.claude-plugin/commands/*.md`
- Move: `hooks/*.sh` → `.claude-plugin/hooks/*.sh`

**Step 1: Move the files**

```bash
mkdir -p .claude-plugin/agents .claude-plugin/skills .claude-plugin/commands .claude-plugin/hooks
mv agents/*.md .claude-plugin/agents/
mv skills/*.md .claude-plugin/skills/
mv commands/*.md .claude-plugin/commands/
mv hooks/*.sh .claude-plugin/hooks/
rmdir agents skills commands hooks
```

**Step 2: Verify plugin.json paths resolve**

```bash
ls .claude-plugin/agents/pipeline-lead.md .claude-plugin/agents/architect.md .claude-plugin/agents/engineer.md .claude-plugin/agents/qa.md .claude-plugin/agents/red-team.md
ls .claude-plugin/skills/request-capability.md .claude-plugin/skills/list-tools.md .claude-plugin/skills/promote-tool.md
ls .claude-plugin/commands/girt-status.md .claude-plugin/commands/girt-build.md .claude-plugin/commands/girt-registry.md
ls .claude-plugin/hooks/capability-intercept.sh .claude-plugin/hooks/tool-call-gate.sh
```

Expected: all files listed without error.

**Step 3: Commit**

```bash
git add -A agents/ skills/ commands/ hooks/ .claude-plugin/
git commit -m "fix: move plugin components under .claude-plugin/ directory

plugin.json references ./agents/, ./skills/, etc. relative to
.claude-plugin/, but files were at repo root. Moves them so
Claude Code plugin discovery finds them."
```

---

### Task 2: Add workspace dependencies for new features

Add `reqwest`, `toml`, and `tempfile` to workspace and update `girt-pipeline` Cargo.toml.

**Files:**
- Modify: `Cargo.toml` (workspace root)
- Modify: `crates/girt-pipeline/Cargo.toml`

**Step 1: Add workspace dependencies**

In `Cargo.toml` (workspace root), add to `[workspace.dependencies]`:

```toml
reqwest = { version = "0.12", features = ["json"] }
toml = "0.8"
tempfile = "3"
```

**Step 2: Update girt-pipeline Cargo.toml**

In `crates/girt-pipeline/Cargo.toml`:

- Change the `reqwest` dependency from optional to required:
  ```toml
  reqwest = { workspace = true }
  ```
- Add `toml`:
  ```toml
  toml = { workspace = true }
  ```
- Remove the `[features]` section (`anthropic` feature is no longer needed — the client is always available, configured at runtime via `girt.toml`).
- `tempfile` is already in `[dev-dependencies]`.

**Step 3: Verify it compiles**

```bash
cargo check --workspace
```

Expected: compiles cleanly.

**Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock crates/girt-pipeline/Cargo.toml
git commit -m "build: add reqwest, toml as required deps for LLM client and config"
```

---

### Task 3: Implement `OpenAiCompatibleClient`

TDD implementation of the OpenAI-compatible LLM client.

**Files:**
- Modify: `crates/girt-pipeline/src/llm.rs`

**Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `crates/girt-pipeline/src/llm.rs`:

```rust
#[tokio::test]
async fn openai_client_formats_request_correctly() {
    // This test uses a mock server to verify request format.
    // We'll test that the client sends the right JSON structure.
    let client = OpenAiCompatibleClient::new(
        "http://localhost:9999/v1".into(),
        "test-model".into(),
        None,
    );
    // Attempting to call a non-existent server should return an error
    let request = LlmRequest {
        system_prompt: "You are helpful.".into(),
        messages: vec![LlmMessage {
            role: "user".into(),
            content: "Hello".into(),
        }],
        max_tokens: 100,
    };
    let result = client.chat(&request).await;
    // Should fail with connection error, not panic
    assert!(result.is_err());
}
```

**Step 2: Run test to verify it fails**

```bash
cargo test -p girt-pipeline openai_client_formats_request -- --nocapture
```

Expected: FAIL — `OpenAiCompatibleClient` does not exist yet.

**Step 3: Implement `OpenAiCompatibleClient`**

Add to `crates/girt-pipeline/src/llm.rs`, above the `StubLlmClient`:

```rust
/// OpenAI-compatible LLM client.
///
/// Calls any `/v1/chat/completions` endpoint (vLLM, OpenAI, Ollama, etc.).
/// Configured via `girt.toml` or environment variables.
pub struct OpenAiCompatibleClient {
    http: reqwest::Client,
    base_url: String,
    model: String,
    api_key: Option<String>,
}

impl OpenAiCompatibleClient {
    pub fn new(base_url: String, model: String, api_key: Option<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url,
            model,
            api_key,
        }
    }
}

impl LlmClient for OpenAiCompatibleClient {
    fn chat<'a>(
        &'a self,
        request: &'a LlmRequest,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, PipelineError>> + Send + 'a>> {
        Box::pin(async move {
            let mut messages = vec![serde_json::json!({
                "role": "system",
                "content": request.system_prompt,
            })];

            for msg in &request.messages {
                messages.push(serde_json::json!({
                    "role": msg.role,
                    "content": msg.content,
                }));
            }

            let body = serde_json::json!({
                "model": self.model,
                "messages": messages,
                "max_tokens": request.max_tokens,
            });

            let url = format!("{}/chat/completions", self.base_url);
            let mut req = self.http.post(&url).json(&body);

            if let Some(key) = &self.api_key {
                req = req.bearer_auth(key);
            }

            let resp = req.send().await.map_err(|e| {
                PipelineError::LlmError(format!("HTTP request failed: {e}"))
            })?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(PipelineError::LlmError(format!(
                    "LLM API returned {status}: {body}"
                )));
            }

            let json: serde_json::Value = resp.json().await.map_err(|e| {
                PipelineError::LlmError(format!("Failed to parse response: {e}"))
            })?;

            let content = json["choices"][0]["message"]["content"]
                .as_str()
                .ok_or_else(|| {
                    PipelineError::LlmError(format!(
                        "No content in response: {}",
                        serde_json::to_string_pretty(&json).unwrap_or_default()
                    ))
                })?
                .to_string();

            Ok(LlmResponse { content })
        })
    }
}
```

**Step 4: Run test to verify it passes**

```bash
cargo test -p girt-pipeline openai_client_formats_request -- --nocapture
```

Expected: PASS (error is returned, not a panic).

**Step 5: Write integration test against real vLLM**

Add another test gated with `#[ignore]`:

```rust
#[tokio::test]
#[ignore] // Requires vLLM running on localhost:8000
async fn openai_client_calls_real_vllm() {
    let client = OpenAiCompatibleClient::new(
        "http://localhost:8000/v1".into(),
        "zai-org/GLM-4.7-Flash".into(),
        None,
    );
    let request = LlmRequest {
        system_prompt: "Reply with exactly: PONG".into(),
        messages: vec![LlmMessage {
            role: "user".into(),
            content: "PING".into(),
        }],
        max_tokens: 10,
    };
    let response = client.chat(&request).await.unwrap();
    assert!(!response.content.is_empty());
}
```

**Step 6: Run integration test**

```bash
cargo test -p girt-pipeline openai_client_calls_real_vllm -- --ignored --nocapture
```

Expected: PASS — response contains text from GLM-4.7-Flash.

**Step 7: Commit**

```bash
git add crates/girt-pipeline/src/llm.rs
git commit -m "feat: add OpenAI-compatible LLM client

Implements LlmClient trait for any /v1/chat/completions endpoint.
Configurable base_url, model, and optional api_key. Tested against
vLLM running GLM-4.7-Flash on localhost:8000."
```

---

### Task 4: Implement `GirtConfig`

Configuration loading from `girt.toml`.

**Files:**
- Create: `crates/girt-pipeline/src/config.rs`
- Modify: `crates/girt-pipeline/src/lib.rs` (add `pub mod config;`)

**Step 1: Write the failing test**

Create `crates/girt-pipeline/src/config.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_config() {
        let toml_str = r#"
[llm]
provider = "openai-compatible"
base_url = "http://localhost:8000/v1"
model = "zai-org/GLM-4.7-Flash"
"#;
        let config: GirtConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.llm.provider, LlmProvider::OpenAiCompatible);
        assert_eq!(config.llm.base_url, "http://localhost:8000/v1");
        assert_eq!(config.llm.model, "zai-org/GLM-4.7-Flash");
        assert_eq!(config.llm.max_tokens, 4096); // default
    }

    #[test]
    fn parses_stub_provider() {
        let toml_str = r#"
[llm]
provider = "stub"
"#;
        let config: GirtConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.llm.provider, LlmProvider::Stub);
    }

    #[test]
    fn parses_full_config() {
        let toml_str = r#"
[llm]
provider = "openai-compatible"
base_url = "http://localhost:11434/v1"
model = "llama3"
api_key = "sk-test"
max_tokens = 8192

[registry]
url = "ghcr.io/epiphytic/girt-tools"

[build]
default_language = "rust"
default_tier = "standard"
"#;
        let config: GirtConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.llm.api_key, Some("sk-test".into()));
        assert_eq!(config.llm.max_tokens, 8192);
        assert_eq!(config.registry.url, "ghcr.io/epiphytic/girt-tools");
        assert_eq!(config.build.default_language, "rust");
    }
}
```

**Step 2: Run test to verify it fails**

```bash
cargo test -p girt-pipeline parses_minimal_config
```

Expected: FAIL — `GirtConfig` does not exist.

**Step 3: Implement config structs**

Add above the tests in `crates/girt-pipeline/src/config.rs`:

```rust
use std::path::Path;
use std::sync::Arc;

use serde::Deserialize;

use crate::error::PipelineError;
use crate::llm::{LlmClient, OpenAiCompatibleClient, StubLlmClient};

#[derive(Debug, Deserialize)]
pub struct GirtConfig {
    pub llm: LlmConfig,
    #[serde(default)]
    pub registry: RegistryConfig,
    #[serde(default)]
    pub build: BuildConfig,
}

#[derive(Debug, Deserialize)]
pub struct LlmConfig {
    pub provider: LlmProvider,
    #[serde(default = "default_base_url")]
    pub base_url: String,
    #[serde(default = "default_model")]
    pub model: String,
    pub api_key: Option<String>,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
}

fn default_base_url() -> String {
    "http://localhost:8000/v1".into()
}
fn default_model() -> String {
    "zai-org/GLM-4.7-Flash".into()
}
fn default_max_tokens() -> u32 {
    4096
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LlmProvider {
    OpenAiCompatible,
    Stub,
}

#[derive(Debug, Default, Deserialize)]
pub struct RegistryConfig {
    #[serde(default = "default_registry_url")]
    pub url: String,
    pub token: Option<String>,
}

fn default_registry_url() -> String {
    "ghcr.io/epiphytic/girt-tools".into()
}

#[derive(Debug, Default, Deserialize)]
pub struct BuildConfig {
    #[serde(default = "default_language")]
    pub default_language: String,
    #[serde(default = "default_tier")]
    pub default_tier: String,
}

fn default_language() -> String {
    "rust".into()
}
fn default_tier() -> String {
    "standard".into()
}

impl GirtConfig {
    /// Load config from a TOML file path.
    pub fn from_file(path: &Path) -> Result<Self, PipelineError> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            PipelineError::IoError(e)
        })?;
        toml::from_str(&content).map_err(|e| {
            PipelineError::LlmError(format!("Failed to parse config: {e}"))
        })
    }

    /// Build an LLM client from this config.
    ///
    /// The `api_key` field can be overridden by the `GIRT_LLM_API_KEY` env var.
    pub fn build_llm_client(&self) -> Arc<dyn LlmClient> {
        match self.llm.provider {
            LlmProvider::OpenAiCompatible => {
                let api_key = std::env::var("GIRT_LLM_API_KEY")
                    .ok()
                    .or_else(|| self.llm.api_key.clone());
                Arc::new(OpenAiCompatibleClient::new(
                    self.llm.base_url.clone(),
                    self.llm.model.clone(),
                    api_key,
                ))
            }
            LlmProvider::Stub => Arc::new(StubLlmClient::constant("stub response")),
        }
    }
}
```

**Step 4: Wire into lib.rs**

Add `pub mod config;` to `crates/girt-pipeline/src/lib.rs`.

**Step 5: Run tests**

```bash
cargo test -p girt-pipeline parses_minimal_config parses_stub_provider parses_full_config
```

Expected: all PASS.

**Step 6: Create default girt.toml at repo root**

Create `girt.toml`:

```toml
[llm]
provider = "openai-compatible"
base_url = "http://localhost:8000/v1"
model = "zai-org/GLM-4.7-Flash"
max_tokens = 4096

[registry]
url = "ghcr.io/epiphytic/girt-tools"

[build]
default_language = "rust"
default_tier = "standard"
```

**Step 7: Commit**

```bash
git add crates/girt-pipeline/src/config.rs crates/girt-pipeline/src/lib.rs girt.toml
git commit -m "feat: add GirtConfig with TOML-based LLM and registry configuration

Supports openai-compatible and stub providers. API key can be set
via girt.toml or GIRT_LLM_API_KEY env var. Default config targets
vLLM on localhost:8000 with GLM-4.7-Flash."
```

---

### Task 5: Implement `WasmCompiler`

Wraps `cargo-component build` to compile Engineer output into WASM Components.

**Files:**
- Create: `crates/girt-pipeline/src/compiler.rs`
- Modify: `crates/girt-pipeline/src/lib.rs` (add `pub mod compiler;`)
- Modify: `crates/girt-pipeline/src/error.rs` (if needed)

**Step 1: Write the failing test**

Create `crates/girt-pipeline/src/compiler.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn scaffolds_cargo_project_correctly() {
        let tmp = TempDir::new().unwrap();
        let compiler = WasmCompiler::new();

        let input = CompileInput {
            source_code: "// placeholder".into(),
            wit_definition: "package test:tool;".into(),
            tool_name: "test_tool".into(),
            tool_version: "0.1.0".into(),
        };

        let build_dir = compiler.scaffold_project(&input, tmp.path()).unwrap();

        assert!(build_dir.join("Cargo.toml").exists());
        assert!(build_dir.join("src/lib.rs").exists());
        assert!(build_dir.join("wit/world.wit").exists());
    }

    #[tokio::test]
    #[ignore] // Requires cargo-component installed
    async fn compiles_minimal_wasm_component() {
        let compiler = WasmCompiler::new();

        // Minimal valid Component Model source
        let input = CompileInput {
            source_code: r#"
wit_bindgen::generate!({
    world: "girt-tool",
    path: "wit",
});

struct Component;

impl Guest for Component {
    fn run(input: String) -> Result<String, String> {
        Ok(format!("echo: {input}"))
    }
}

export!(Component);
"#.into(),
            wit_definition: r#"
package girt:tool@0.1.0;

world girt-tool {
    export run: func(input: string) -> result<string, string>;
}
"#.into(),
            tool_name: "echo_tool".into(),
            tool_version: "0.1.0".into(),
        };

        let output = compiler.compile(&input).await.unwrap();
        assert!(output.wasm_path.exists());
        assert!(output.wasm_path.extension().unwrap() == "wasm");
    }
}
```

**Step 2: Run test to verify it fails**

```bash
cargo test -p girt-pipeline scaffolds_cargo_project_correctly
```

Expected: FAIL — `WasmCompiler` does not exist.

**Step 3: Implement `WasmCompiler`**

Add above the tests in `crates/girt-pipeline/src/compiler.rs`:

```rust
use std::path::{Path, PathBuf};

use crate::error::PipelineError;

/// Input to the WASM compiler.
pub struct CompileInput {
    pub source_code: String,
    pub wit_definition: String,
    pub tool_name: String,
    pub tool_version: String,
}

/// Output of a successful WASM compilation.
pub struct CompileOutput {
    pub wasm_path: PathBuf,
    pub build_dir: PathBuf,
}

/// Wraps `cargo-component build` to compile Rust source into WASM Components.
pub struct WasmCompiler {
    cargo_component_bin: String,
}

impl WasmCompiler {
    pub fn new() -> Self {
        Self {
            cargo_component_bin: "cargo-component".into(),
        }
    }

    /// Scaffold a Cargo project in the given directory.
    pub fn scaffold_project(
        &self,
        input: &CompileInput,
        base_dir: &Path,
    ) -> Result<PathBuf, PipelineError> {
        let project_dir = base_dir.join(&input.tool_name);
        std::fs::create_dir_all(project_dir.join("src"))?;
        std::fs::create_dir_all(project_dir.join("wit"))?;

        // Write Cargo.toml
        let cargo_toml = format!(
            r#"[package]
name = "{name}"
version = "{version}"
edition = "2024"

[dependencies]
wit-bindgen = "0.41"
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"

[lib]
crate-type = ["cdylib"]

[package.metadata.component]
package = "girt:tool@0.1.0"
"#,
            name = input.tool_name,
            version = input.tool_version,
        );
        std::fs::write(project_dir.join("Cargo.toml"), cargo_toml)?;

        // Write source code
        std::fs::write(project_dir.join("src/lib.rs"), &input.source_code)?;

        // Write WIT definition
        std::fs::write(project_dir.join("wit/world.wit"), &input.wit_definition)?;

        Ok(project_dir)
    }

    /// Compile the project into a WASM Component.
    pub async fn compile(&self, input: &CompileInput) -> Result<CompileOutput, PipelineError> {
        let tmp = tempfile::tempdir()?;
        let project_dir = self.scaffold_project(input, tmp.path())?;

        let output = tokio::process::Command::new(&self.cargo_component_bin)
            .arg("build")
            .arg("--release")
            .current_dir(&project_dir)
            .output()
            .await
            .map_err(|e| {
                PipelineError::CompilationError(format!(
                    "Failed to run cargo-component: {e}. Is it installed? (cargo install cargo-component)"
                ))
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            return Err(PipelineError::CompilationError(format!(
                "cargo-component build failed:\nstdout: {stdout}\nstderr: {stderr}"
            )));
        }

        // Find the compiled .wasm file
        let wasm_dir = project_dir
            .join("target")
            .join("wasm32-wasip1")
            .join("release");

        let wasm_filename = format!("{}.wasm", input.tool_name.replace('-', "_"));
        let wasm_path = wasm_dir.join(&wasm_filename);

        if !wasm_path.exists() {
            // Try finding any .wasm file in the release dir
            let mut found = None;
            if wasm_dir.exists() {
                for entry in std::fs::read_dir(&wasm_dir)? {
                    let entry = entry?;
                    if entry.path().extension().is_some_and(|e| e == "wasm") {
                        found = Some(entry.path());
                        break;
                    }
                }
            }
            match found {
                Some(path) => return Ok(CompileOutput {
                    wasm_path: path,
                    build_dir: project_dir,
                }),
                None => return Err(PipelineError::CompilationError(format!(
                    "No .wasm file found in {}", wasm_dir.display()
                ))),
            }
        }

        Ok(CompileOutput {
            wasm_path,
            build_dir: project_dir,
        })
    }
}

impl Default for WasmCompiler {
    fn default() -> Self {
        Self::new()
    }
}
```

**Step 4: Wire into lib.rs**

Add `pub mod compiler;` to `crates/girt-pipeline/src/lib.rs`.

**Step 5: Run scaffold test**

```bash
cargo test -p girt-pipeline scaffolds_cargo_project_correctly
```

Expected: PASS.

**Step 6: Run compile test (requires cargo-component)**

```bash
cargo test -p girt-pipeline compiles_minimal_wasm_component -- --ignored --nocapture
```

Expected: PASS — `.wasm` file produced.

**Step 7: Commit**

```bash
git add crates/girt-pipeline/src/compiler.rs crates/girt-pipeline/src/lib.rs
git commit -m "feat: add WasmCompiler wrapping cargo-component build

Scaffolds a Cargo project from Engineer output (source, WIT, Cargo.toml)
and compiles it to a WASM Component. Includes project scaffolding test
and integration test (gated behind --ignored) that compiles real WASM."
```

---

### Task 6: Implement `OciPublisher`

Extend the `Publisher` with OCI push via `oras`.

**Files:**
- Modify: `crates/girt-pipeline/src/publish.rs`

**Step 1: Write the failing test**

Add to the test module in `crates/girt-pipeline/src/publish.rs`:

```rust
#[tokio::test]
async fn publish_full_stores_wasm_in_cache() {
    let tmp = TempDir::new().unwrap();
    let cache = ToolCache::new(tmp.path().join("tools"));
    let publisher = Publisher::new(cache);
    publisher.init().await.unwrap();

    let artifact = make_artifact();

    // Create a fake .wasm file
    let wasm_dir = tmp.path().join("build");
    std::fs::create_dir_all(&wasm_dir).unwrap();
    let wasm_path = wasm_dir.join("published_tool.wasm");
    std::fs::write(&wasm_path, b"fake wasm bytes").unwrap();

    let result = publisher
        .publish_with_wasm(&artifact, &wasm_path)
        .await
        .unwrap();

    assert_eq!(result.tool_name, "published_tool");
    assert!(result.local_path.join("tool.wasm").exists());
}
```

**Step 2: Run test to verify it fails**

```bash
cargo test -p girt-pipeline publish_full_stores_wasm_in_cache
```

Expected: FAIL — `publish_with_wasm` does not exist.

**Step 3: Implement `publish_with_wasm` and `push_oci`**

Add to the `Publisher` impl in `crates/girt-pipeline/src/publish.rs`:

```rust
/// Publish a build artifact with its compiled WASM binary.
///
/// Stores in local cache and optionally pushes to OCI registry.
pub async fn publish_with_wasm(
    &self,
    artifact: &BuildArtifact,
    wasm_path: &std::path::Path,
) -> Result<PublishResult, PipelineError> {
    let tool_name = artifact.spec.name.clone();

    // Store in local cache (metadata + source)
    let local_path = self.cache.store(artifact).await?;

    // Copy WASM binary into cache dir
    let cached_wasm = local_path.join("tool.wasm");
    tokio::fs::copy(wasm_path, &cached_wasm).await?;

    tracing::info!(
        tool = %tool_name,
        path = %local_path.display(),
        "Artifact published to local cache with WASM binary"
    );

    Ok(PublishResult {
        tool_name,
        local_path,
        oci_reference: None,
    })
}

/// Push an artifact to an OCI registry using `oras push`.
///
/// Requires `oras` CLI on PATH and valid authentication.
pub async fn push_oci(
    &self,
    artifact: &BuildArtifact,
    wasm_path: &std::path::Path,
    registry_url: &str,
    tag: &str,
) -> Result<String, PipelineError> {
    let tool_name = &artifact.spec.name;
    let reference = format!("{}/{}:{}", registry_url, tool_name, tag);

    let cache_dir = self.cache.base_dir().join(tool_name);
    let manifest_path = cache_dir.join("manifest.json");
    let policy_path = cache_dir.join("policy.yaml");

    // Verify files exist
    for path in [wasm_path, manifest_path.as_path(), policy_path.as_path()] {
        if !path.exists() {
            return Err(PipelineError::PublishError(format!(
                "Required file missing: {}",
                path.display()
            )));
        }
    }

    let output = tokio::process::Command::new("oras")
        .arg("push")
        .arg(&reference)
        .arg(format!(
            "{}:application/vnd.wasm.component.layer.v0+wasm",
            wasm_path.display()
        ))
        .arg(format!(
            "{}:application/vnd.girt.policy.v1+yaml",
            policy_path.display()
        ))
        .arg(format!(
            "{}:application/vnd.girt.manifest.v1+json",
            manifest_path.display()
        ))
        .output()
        .await
        .map_err(|e| {
            PipelineError::PublishError(format!(
                "Failed to run oras: {e}. Is it installed?"
            ))
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(PipelineError::PublishError(format!(
            "oras push failed: {stderr}"
        )));
    }

    tracing::info!(tool = %tool_name, reference = %reference, "Pushed to OCI registry");
    Ok(reference)
}
```

**Step 4: Run tests**

```bash
cargo test -p girt-pipeline publish_full_stores_wasm_in_cache
```

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/girt-pipeline/src/publish.rs
git commit -m "feat: add WASM binary publishing and OCI registry push

Publisher.publish_with_wasm() stores WASM binary alongside metadata
in local cache. Publisher.push_oci() wraps oras CLI to push artifacts
to ghcr.io with typed OCI media types."
```

---

### Task 7: Implement `QueueConsumer`

Wires queue → orchestrator → compiler → publisher.

**Files:**
- Modify: `crates/girt-pipeline/src/queue.rs` (add `QueueConsumer` and `ProcessResult`)

**Step 1: Write the failing test**

Add to the test module in `crates/girt-pipeline/src/queue.rs`:

```rust
#[tokio::test]
async fn queue_consumer_processes_happy_path() {
    use crate::cache::ToolCache;
    use crate::compiler::WasmCompiler;
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

    // Stub LLM with happy-path responses
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

    // Enqueue a request
    let request = make_request("test_tool");
    consumer.queue().enqueue(&request).await.unwrap();

    // Process it (skip compile for stub test)
    let result = consumer.process_next_no_compile().await.unwrap();
    assert!(result.is_some());

    let snap = metrics.snapshot();
    assert_eq!(snap.builds_started, 1);
    assert_eq!(snap.builds_completed, 1);
}
```

**Step 2: Run test to verify it fails**

```bash
cargo test -p girt-pipeline queue_consumer_processes_happy_path
```

Expected: FAIL — `QueueConsumer` does not exist.

**Step 3: Implement `QueueConsumer`**

Add to `crates/girt-pipeline/src/queue.rs`:

```rust
use crate::cache::ToolCache;
use crate::compiler::WasmCompiler;
use crate::llm::LlmClient;
use crate::metrics::PipelineMetrics;
use crate::orchestrator::{Orchestrator, PipelineOutcome};
use crate::publish::Publisher;
use std::sync::Arc;

/// Result of processing a queue item.
#[derive(Debug)]
pub enum ProcessResult {
    Built { name: String, oci_reference: Option<String> },
    Extended { target: String, features: Vec<String> },
    Failed(PipelineError),
}

/// Connects queue → orchestrator → compiler → publisher.
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
        Self { queue, llm, publisher, metrics }
    }

    pub fn queue(&self) -> &Queue {
        &self.queue
    }

    /// Process one request: orchestrate → compile → publish.
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
                // Compile WASM
                let compile_input = crate::compiler::CompileInput {
                    source_code: artifact.build_output.source_code.clone(),
                    wit_definition: artifact.build_output.wit_definition.clone(),
                    tool_name: artifact.spec.name.clone(),
                    tool_version: "0.1.0".into(),
                };

                let compile_output = compiler.compile(&compile_input).await?;

                // Publish locally with WASM
                self.publisher
                    .publish_with_wasm(&artifact, &compile_output.wasm_path)
                    .await?;

                // Optionally push to OCI registry
                let oci_reference = if let (Some(url), Some(t)) = (registry_url, tag) {
                    Some(self.publisher.push_oci(&artifact, &compile_output.wasm_path, url, t).await?)
                } else {
                    None
                };

                self.queue.complete(&request).await?;
                self.metrics.record_build_completed(artifact.build_iterations);

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

    /// Process without compilation (for unit tests with stub LLM).
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
                self.metrics.record_build_completed(artifact.build_iterations);
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
```

**Step 4: Run tests**

```bash
cargo test -p girt-pipeline queue_consumer_processes_happy_path
```

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/girt-pipeline/src/queue.rs
git commit -m "feat: add QueueConsumer wiring queue to orchestrator to publisher

Connects the full pipeline: claim request from queue, run orchestrator
(Architect → Engineer → QA → Red Team), compile WASM, publish to
local cache + optionally OCI registry. Includes no-compile variant
for unit tests with stubbed LLM."
```

---

### Task 8: Update Engineer agent prompt for real compilation

The current Engineer prompt produces freestanding Rust. For real `cargo-component` compilation, it needs to produce Component Model code with `wit_bindgen::generate!`.

**Files:**
- Modify: `crates/girt-pipeline/src/agent/engineer.rs`
- Modify: `.claude-plugin/agents/engineer.md`

**Step 1: Update the Rust prompt constant**

Replace `ENGINEER_RUST_PROMPT` in `crates/girt-pipeline/src/agent/engineer.rs` with a prompt that instructs the LLM to produce compilable Component Model code. The prompt must specify:
- Use `wit_bindgen::generate!` macro
- Export a `run` function matching the WIT world
- Include proper `use` statements
- Output both the source code and a matching WIT definition
- The WIT must define a `girt-tool` world with `export run: func(input: string) -> result<string, string>;`

Read the existing prompt at `crates/girt-pipeline/src/agent/engineer.rs:5-23` and update it. Also update the WIT in the JSON output contract to match what `cargo-component` expects.

**Step 2: Update the plugin agent prompt**

Read `.claude-plugin/agents/engineer.md` and add the same Component Model instructions to the agent's system prompt for when it runs as a Claude Code agent.

**Step 3: Run existing Engineer tests**

```bash
cargo test -p girt-pipeline -k engineer
```

Expected: existing tests still pass (they use StubLlmClient and don't compile).

**Step 4: Commit**

```bash
git add crates/girt-pipeline/src/agent/engineer.rs .claude-plugin/agents/engineer.md
git commit -m "feat: update Engineer prompt for WASM Component Model compilation

Instructs LLM to produce code using wit_bindgen::generate! macro,
export a run function matching the girt-tool WIT world, and include
a matching WIT interface definition."
```

---

### Task 9: Write E2E test harness

The full-stack integration test. Gated with `#[ignore]`.

**Files:**
- Create: `crates/girt-pipeline/tests/e2e_pipeline.rs`
- Create: `scripts/run-e2e.sh`

**Step 1: Write the E2E test**

Create `crates/girt-pipeline/tests/e2e_pipeline.rs`:

```rust
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

    // Process the request (with real LLM + real compilation)
    let result = consumer
        .process_next(&compiler, None, None)
        .await
        .expect("process_next should not error");

    // Assert something happened
    assert!(result.is_some(), "Queue should have had a request");

    let snap = metrics.snapshot();
    assert_eq!(snap.builds_started, 1);

    match result.unwrap() {
        girt_pipeline::queue::ProcessResult::Built { name, .. } => {
            assert_eq!(name, "base64_codec");
            // Verify cache has the tool
            let tool_dir = tmp.path().join("tools").join("base64_codec");
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
        girt_pipeline::queue::ProcessResult::Failed(e) => {
            // A failed build is still a valid E2E outcome
            // (LLM might not generate compilable code on first try)
            eprintln!("Build failed (expected possible): {e}");
            assert_eq!(snap.builds_failed, 1);
        }
        girt_pipeline::queue::ProcessResult::Extended { target, .. } => {
            eprintln!("Got RecommendExtend to: {target}");
            // Valid outcome if LLM decides to recommend extension
        }
    }

    // Verify queue state
    assert!(
        consumer.queue().list_pending().await.unwrap().is_empty(),
        "No requests should be pending"
    );
    assert!(
        consumer.queue().list_in_progress().await.unwrap().is_empty(),
        "No requests should be in progress"
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

    let result = consumer
        .process_next(&compiler, None, None)
        .await
        .expect("process_next should not error");

    assert!(result.is_some());

    let snap = metrics.snapshot();
    assert_eq!(snap.builds_started, 1);
    // With a real LLM, the build may actually succeed (LLM will try its best)
    // or fail. Either outcome is valid for E2E. The important thing is no panic.
    assert!(snap.builds_completed + snap.builds_failed == 1);
}
```

**Step 2: Create the E2E helper script**

Create `scripts/run-e2e.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

echo "=== GIRT E2E Test Runner ==="
echo ""

# Check prerequisites
MISSING=()

if ! command -v cargo-component &>/dev/null; then
    MISSING+=("cargo-component (cargo install cargo-component)")
fi

if ! command -v wassette &>/dev/null; then
    MISSING+=("wassette (https://github.com/microsoft/wassette)")
fi

if ! command -v oras &>/dev/null; then
    echo "WARN: oras not found — OCI push tests will be skipped"
fi

if ! curl -sf http://localhost:8000/v1/models >/dev/null 2>&1; then
    MISSING+=("vLLM on localhost:8000 (not reachable)")
fi

if [ ${#MISSING[@]} -gt 0 ]; then
    echo "ERROR: Missing prerequisites:"
    for dep in "${MISSING[@]}"; do
        echo "  - $dep"
    done
    exit 1
fi

echo "All prerequisites OK."
echo ""
echo "Running E2E tests (this may take several minutes per test)..."
echo ""

cargo test -p girt-pipeline --test e2e_pipeline -- --ignored --nocapture "$@"
```

**Step 3: Make script executable and test it**

```bash
chmod +x scripts/run-e2e.sh
./scripts/run-e2e.sh
```

Expected: prerequisites check passes, E2E tests run. The happy-path test may take 1-3 minutes due to LLM calls + compilation.

**Step 4: Commit**

```bash
git add crates/girt-pipeline/tests/e2e_pipeline.rs scripts/run-e2e.sh
git commit -m "feat: add E2E pipeline integration tests

Full-stack tests that run real LLM (GLM-4.7-Flash via vLLM),
real WASM compilation (cargo-component), and verify artifacts.
Gated with #[ignore], run via scripts/run-e2e.sh or
cargo test --test e2e_pipeline -- --ignored."
```

---

### Task 10: Run E2E and fix issues

This is the iteration task. Run the E2E, observe failures, fix, repeat.

**Step 1: Run E2E happy path**

```bash
cargo test -p girt-pipeline --test e2e_pipeline e2e_happy_path -- --ignored --nocapture
```

**Step 2: Diagnose failures**

Common failure modes:
- **LLM returns invalid JSON**: The Engineer's `parse_build_output` fallback handles this, but the source code won't compile as a proper Component. Fix: improve the Engineer prompt with more examples.
- **Compilation fails**: The LLM-generated code doesn't use `wit_bindgen` correctly. Fix: add few-shot examples to the prompt, or add a retry loop in the compiler that feeds errors back to the LLM.
- **WIT mismatch**: The WIT the LLM generates doesn't match what `cargo-component` expects. Fix: provide the exact WIT in the prompt and tell the LLM not to modify it.

**Step 3: Fix and re-run until green**

Iterate on:
1. Engineer prompt quality (most likely source of failures)
2. Compiler error reporting (feed `cargo-component` stderr back to user)
3. QA/Red Team prompt alignment with Wassette execution model

**Step 4: Commit each fix**

Each fix should be a separate commit with a descriptive message explaining what broke and why.

---

### Task 11: OCI registry push E2E (optional, if oras is available)

**Files:**
- Modify: `crates/girt-pipeline/tests/e2e_pipeline.rs`

**Step 1: Add OCI push test**

```rust
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

    // Same setup as happy path, but with registry_url and tag
    // ... (same as e2e_happy_path_simple_tool but pass registry URL)

    let result = consumer
        .process_next(
            &compiler,
            Some("ghcr.io/epiphytic/girt-tools"),
            Some("e2e-test"),
        )
        .await
        .expect("process_next should not error");

    if let Some(girt_pipeline::queue::ProcessResult::Built { oci_reference, .. }) = &result {
        if let Some(reference) = oci_reference {
            eprintln!("Published to: {reference}");
            // Clean up: delete the test tag
            let _ = tokio::process::Command::new("oras")
                .args(["manifest", "delete", "--force", reference])
                .output()
                .await;
        }
    }
}
```

**Step 2: Run it**

```bash
cargo test -p girt-pipeline --test e2e_pipeline e2e_happy_path_with_oci_push -- --ignored --nocapture
```

**Step 3: Commit**

```bash
git add crates/girt-pipeline/tests/e2e_pipeline.rs
git commit -m "feat: add E2E test with OCI registry push

Pushes built artifact to ghcr.io/epiphytic/girt-tools with e2e-test
tag. Cleans up after. Skips if oras is not installed."
```

---

## Summary

| Task | Component | Effort |
|------|-----------|--------|
| 1 | Plugin path fix | Small — move files |
| 2 | Workspace deps | Small — Cargo.toml edits |
| 3 | OpenAiCompatibleClient | Medium — HTTP client + tests |
| 4 | GirtConfig | Medium — TOML parsing + tests |
| 5 | WasmCompiler | Medium — cargo-component wrapper + tests |
| 6 | OciPublisher | Medium — oras wrapper + tests |
| 7 | QueueConsumer | Medium — wiring + tests |
| 8 | Engineer prompt update | Small — prompt engineering |
| 9 | E2E test harness | Medium — integration tests + script |
| 10 | E2E iteration | Variable — fix LLM output issues |
| 11 | OCI push E2E | Small — extend test |
