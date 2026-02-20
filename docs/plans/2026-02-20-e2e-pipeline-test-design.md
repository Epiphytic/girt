# E2E Pipeline Test Design

**Date:** 2026-02-20
**Status:** Approved
**Scope:** Full-stack end-to-end test of the GIRT build pipeline

---

## Goal

Validate the entire GIRT pipeline end-to-end with real components: a real LLM (GLM-4.7-Flash via vLLM), real WASM compilation (cargo-component), real Wassette execution, and real OCI registry push (ghcr.io). No stubs in the critical path.

## Current State & Gaps

The codebase has 90+ unit tests covering individual components with `StubLlmClient` and mocked responses. Nothing tests the full flow: queue file → orchestrator → LLM calls → WASM compile → Wassette load → OCI publish.

### Gap 1: Plugin path mismatch

`plugin.json` references `"agents": "./agents/"` relative to `.claude-plugin/`, but agent files live at `agents/` (repo root). The plugin cannot discover its components.

**Fix:** Move all plugin component files under `.claude-plugin/`.

### Gap 2: No OpenAI-compatible LLM client

The `LlmClient` trait has only `StubLlmClient`. Need `OpenAiCompatibleClient` that calls any `/v1/chat/completions` endpoint.

**Fix:** Implement `OpenAiCompatibleClient` in `crates/girt-pipeline/src/llm.rs`, configured via `girt.toml`.

### Gap 3: No WASM compilation

The Engineer generates source code strings but nothing compiles them into WASM Components.

**Fix:** New `WasmCompiler` in `crates/girt-pipeline/src/compiler.rs` wrapping `cargo-component build`.

### Gap 4: No OCI registry push

The Publisher writes to local cache only.

**Fix:** Extend `Publisher` with `oras push` to ghcr.io.

### Gap 5: No queue-to-publish wiring

Queue, orchestrator, and publisher exist as separate units. Nothing connects them.

**Fix:** New `QueueConsumer` that claims → orchestrates → compiles → publishes → moves to completed/failed.

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                    E2E Test Harness                       │
│                                                           │
│  Setup:                                                   │
│    1. Create temp dirs for queue + tool cache             │
│    2. Initialize OpenAiCompatibleClient → localhost:8000  │
│    3. Write CapabilityRequest JSON to queue/pending/      │
│                                                           │
│  Execution:                                               │
│    4. QueueConsumer claims the request                    │
│    5. Orchestrator runs full pipeline:                    │
│       Architect → Engineer → compile → QA → Red Team     │
│    6. On pass: publish to local cache + OCI registry     │
│    7. On fail: circuit breaker after 3 iterations        │
│                                                           │
│  Assertions:                                              │
│    8. Tool dir exists at cache/<tool_name>/               │
│    9. manifest.json has qa_result.passed == true          │
│   10. tool.wasm is a valid WASM Component                │
│   11. policy.yaml matches spec constraints               │
│   12. OCI artifact exists at ghcr.io                     │
│   13. wassette component load file://<tool.wasm> works   │
│   14. Queue request moved to completed/                  │
│                                                           │
│  Cleanup:                                                 │
│   15. Remove temp dirs                                   │
│   16. Delete test OCI tag from registry                  │
│                                                           │
│  ┌───────────────────────────────────────────────────┐   │
│  │           OpenAiCompatibleClient                   │   │
│  │  endpoint: http://localhost:8000/v1                │   │
│  │  model: zai-org/GLM-4.7-Flash                     │   │
│  │  ▲                                                 │   │
│  │  │ LlmClient::chat()                              │   │
│  │  │                                                 │   │
│  │  ├── ArchitectAgent::refine()                      │   │
│  │  ├── EngineerAgent::build()                        │   │
│  │  ├── QaAgent::test()                               │   │
│  │  ├── RedTeamAgent::audit()                         │   │
│  │  └── EngineerAgent::fix() (if bug tickets)         │   │
│  └───────────────────────────────────────────────────┘   │
│                                                           │
│  ┌───────────┐  ┌────────────┐  ┌─────────────────┐     │
│  │ FileQueue  │→│Orchestrator│→│ WasmCompiler     │     │
│  │ (real fs)  │  │ (real)     │  │ (cargo-component)│     │
│  └───────────┘  └────────────┘  └────────┬────────┘     │
│                                           │               │
│                       ┌───────────────────┤               │
│                       ▼                   ▼               │
│               ┌──────────────┐   ┌──────────────┐        │
│               │ LocalCache   │   │ OciPublisher  │        │
│               │ (~/.girt/)   │   │ (ghcr.io)    │        │
│               └──────────────┘   └──────────────┘        │
└─────────────────────────────────────────────────────────┘
```

### Component reality table

| Component | Status | Notes |
|---|---|---|
| File queue | Real | Temp dir for isolation |
| Orchestrator | Real | Full circuit breaker logic |
| LLM calls | Real | GLM-4.7-Flash via vLLM on localhost:8000 |
| Architect | Real | LLM refines spec |
| Engineer | Real | LLM generates compilable Rust code |
| WASM compilation | Real | `cargo-component build --release` |
| QA | Real | LLM generates tests, runs via Wassette |
| Red Team | Real | LLM crafts exploits, runs via Wassette |
| Publisher | Real | Local cache + OCI push to ghcr.io |
| Wassette | Real | Loads compiled component, runs tool invocations |
| OCI registry | Real | Push to ghcr.io/epiphytic/girt-tools |
| MCP proxy | Not tested | Separate integration test, has smoke tests |
| Decision engine | Not tested | Has 38 unit tests, orthogonal to build pipeline |

## New Code

### 1. `OpenAiCompatibleClient` — `crates/girt-pipeline/src/llm.rs`

Implements `LlmClient` trait using `reqwest` to call `/v1/chat/completions`.

```rust
pub struct OpenAiCompatibleClient {
    http: reqwest::Client,
    base_url: String,       // "http://localhost:8000/v1"
    model: String,          // "zai-org/GLM-4.7-Flash"
    api_key: Option<String>,
}
```

Mapping:
- `LlmRequest.system_prompt` → `{"role": "system", "content": "..."}`
- `LlmRequest.messages` → `[{"role": "user"/"assistant", "content": "..."}]`
- `LlmRequest.max_tokens` → `max_tokens`
- Response: extract `choices[0].message.content`

### 2. `WasmCompiler` — `crates/girt-pipeline/src/compiler.rs`

Wraps `cargo-component build`.

```rust
pub struct WasmCompiler {
    cargo_component_bin: String,  // defaults to "cargo-component"
}

pub struct CompileInput {
    pub source_code: String,
    pub wit_definition: String,
    pub tool_name: String,
    pub tool_version: String,
}

pub struct CompileOutput {
    pub wasm_path: PathBuf,
    pub build_dir: PathBuf,
}
```

Flow:
1. Create temp dir with scaffolded Cargo project:
   ```
   /tmp/girt-build-<uuid>/
     Cargo.toml          # template with wit-bindgen, wasi deps
     src/lib.rs           # Engineer's source code
     wit/world.wit        # Engineer's WIT + girt-world imports
   ```
2. Run `cargo component build --release --target wasm32-wasip1`
3. Return path to compiled `.wasm` or `CompileError`

### 3. `OciPublisher` — extends `crates/girt-pipeline/src/publish.rs`

Uses `oras push` to publish to ghcr.io.

Artifact layers:
- `tool.wasm` → `application/vnd.wasm.component.layer.v0+wasm`
- `policy.yaml` → `application/vnd.girt.policy.v1+yaml`
- `manifest.json` → `application/vnd.girt.manifest.v1+json`

Auth: `GIRT_REGISTRY_TOKEN` env var, or `gh auth token`.

For E2E tests: push to `ghcr.io/epiphytic/girt-tools/<name>:e2e-test`, clean up after.

### 4. `QueueConsumer` — extends `crates/girt-pipeline/src/queue.rs`

Connects queue → orchestrator → compiler → publisher:

```rust
pub struct QueueConsumer {
    queue: FileQueue,
    llm: Arc<dyn LlmClient>,
    compiler: WasmCompiler,
    publisher: Publisher,
    metrics: Arc<PipelineMetrics>,
}

impl QueueConsumer {
    pub async fn process_next(&self) -> Result<Option<ProcessResult>, PipelineError> {
        // claim → orchestrate → compile → publish → move to completed/failed
    }
}
```

### 5. `GirtConfig` — `crates/girt-pipeline/src/config.rs`

Reads `girt.toml`:

```toml
[llm]
provider = "openai-compatible"   # or "stub"
base_url = "http://localhost:8000/v1"
model = "zai-org/GLM-4.7-Flash"
api_key = ""                     # or env: GIRT_LLM_API_KEY
max_tokens = 4096

[registry]
url = "ghcr.io/epiphytic/girt-tools"
# token from env: GIRT_REGISTRY_TOKEN

[build]
default_language = "rust"
default_tier = "standard"
```

### 6. Agent prompt updates

**Engineer**: Must produce code compatible with `girt-world@0.1.0` WIT using `wit_bindgen::generate!`. Current prompt generates freestanding Rust.

**QA**: Generate test cases as JSON, run each via `wassette tool invoke` against the loaded component, compare actual vs expected.

**Red Team**: Generate exploit payloads, run via `wassette tool invoke`, verify policy enforcement blocks disallowed access.

### 7. Plugin path fix

Move all plugin component files under `.claude-plugin/`:

```
.claude-plugin/
  plugin.json
  agents/pipeline-lead.md, architect.md, engineer.md, qa.md, red-team.md
  skills/request-capability.md, list-tools.md, promote-tool.md
  commands/girt-status.md, girt-build.md, girt-registry.md
  hooks/capability-intercept.sh, tool-call-gate.sh
```

## Test Scenarios

### Scenario 1: Happy path — simple stateless tool

Request a "base64 encoder/decoder" with no network, no secrets. The LLM produces compilable Rust. QA and Red Team pass. Assert: compiled .wasm loads in Wassette, artifact published to cache and registry.

### Scenario 2: Network-capable tool

Request an "HTTP status checker" needing network access to a declared host. Assert: policy.yaml constrains to declared hosts, Red Team validates no SSRF.

### Scenario 3: Circuit breaker

Request a tool with contradictory or impossible requirements. Assert: 3 iterations, `PipelineError::CircuitBreaker`, request moves to `failed/`.

### Scenario 4: Recommend extend

Pre-populate cache with a stdlib tool, then request something overlapping. Assert: Architect recommends extending, no build triggered.

## Test Infrastructure

### Test gating

Tests require external dependencies (vLLM, cargo-component, Wassette, OCI auth). Gated with `#[ignore]` by default:

```bash
# Run E2E tests
cargo test --test e2e_pipeline -- --ignored

# Or via helper script that checks prereqs
./scripts/run-e2e.sh
```

### Test isolation

Each test creates isolated temp dirs for queue and cache. OCI pushes use a `e2e-test` tag and are cleaned up after.

### Prerequisites

```bash
cargo install cargo-component
cargo install oras           # or install from release
rustup target add wasm32-wasip1

# Verify
cargo-component --version
oras version
wassette --version
curl -s http://localhost:8000/v1/models  # vLLM with GLM-4.7-Flash
gh auth status                            # for OCI push
```

## Dependencies

### New crate dependencies

| Crate | Purpose | Used by |
|---|---|---|
| `reqwest` + `json` | HTTP client for OpenAI API | `OpenAiCompatibleClient` |
| `toml` | Parse girt.toml | `GirtConfig` |
| `tempfile` | Isolated build/test dirs | `WasmCompiler`, E2E tests |

### CLI tools (prerequisites)

| Tool | Purpose |
|---|---|
| `cargo-component` | Compile Rust → WASM Component |
| `oras` | Push OCI artifacts to ghcr.io |
| `wassette` v0.4.0+ | Load + run WASM Components |
| vLLM + GLM-4.7-Flash | LLM inference on localhost:8000 |

## Non-goals

- Testing the MCP proxy layer (separate integration test)
- Testing the decision engine cascade (38 unit tests exist)
- Testing the agent team orchestration via Claude Code plugin (depends on plugin path fix; separate test)
- SLSA provenance attestation generation (CI-only concern)
- Multi-language compilation (Go, AssemblyScript — Rust only for E2E v1)
