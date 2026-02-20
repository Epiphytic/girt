# Security Review: E2E Pipeline Implementation

**Branch:** `test/design-end-to-end-pipeline-test`
**Date:** 2026-02-20
**Reviewer:** Claude Opus 4.6
**Verdict:** REQUEST_CHANGES (1 HIGH, 2 MEDIUM, 2 LOW)

---

## Threat Model Context

GIRT is a tool factory that generates, compiles, and publishes WASM tools. The primary attack surface is:

1. **LLM outputs** -- all source code, WIT definitions, and policy YAML come from LLM responses
2. **External tool execution** -- `cargo-component` and `oras` are invoked as subprocesses
3. **OCI registry interaction** -- pushing artifacts with credentials
4. **Configuration** -- API keys and registry tokens in config files

The WASM sandbox (Wassette/Wasmtime) provides the primary isolation boundary. This review focuses on the pipeline components that sit *outside* that sandbox.

---

## Findings

### HIGH

#### H-1: QA and Red Team silently pass on LLM response parse failure
**File:** `crates/girt-pipeline/src/agent/qa.rs:67-85`, `crates/girt-pipeline/src/agent/red_team.rs:68-85`
**Confidence:** 95
**CVSS:** N/A (design flaw, not exploitable vulnerability)

**Description:** When the LLM returns a response that cannot be parsed as JSON, both the QA agent and Red Team agent default to `passed: true`. This means:

- If an LLM consistently fails to produce structured JSON, *every tool it generates will pass all validation*.
- A prompt injection attack that causes the LLM to output non-JSON will bypass both QA and security review.
- The `tests_run: 0, tests_passed: 0, passed: true` default is internally contradictory.

**Attack vector:** An adversary who can influence the tool specification (e.g., via a prompt injection in the tool description field) could craft input that causes the QA/Red Team LLM calls to produce non-JSON output, thus bypassing all validation gates.

**Impact:** Malicious or buggy tool code could be published without QA or security review.

**Remediation:**
- Default to `passed: false` on parse failure, or
- Retry the LLM call once before defaulting, or
- Treat parse failure as a distinct "inconclusive" result that the orchestrator handles (e.g., counts as a failed iteration toward the circuit breaker)

---

### MEDIUM

#### M-1: Tool name not sanitized for path traversal in compiler
**File:** `crates/girt-pipeline/src/compiler.rs:44`
**Confidence:** 85

**Description:** The `tool_name` from `CompileInput` is used directly as a path component:

```rust
let project_dir = base_dir.join(&input.tool_name);
```

The `tool_name` originates from the Architect LLM's refined spec. If the LLM produces a name containing path traversal sequences (e.g., `../../../tmp/evil`), files could be written outside the temp directory.

**Attack vector:** Prompt injection in the tool spec's `name` field, or a compromised/misbehaving LLM producing a name with `../` sequences.

**Impact:** File write outside intended directory. Limited by OS permissions and the fact that the base_dir is typically in a temp directory, but could still overwrite files in `/tmp` or adjacent temp directories.

**Remediation:** Sanitize tool_name before path construction:
```rust
fn sanitize_tool_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .collect()
}
```

#### M-2: OCI push passes file paths as command-line arguments
**File:** `crates/girt-pipeline/src/publish.rs:105-126`
**Confidence:** 80

**Description:** The `push_oci` method constructs `oras push` arguments using `format!` with `wasm_path.display()` and other paths. While `tokio::process::Command` does handle argument escaping, the paths originate from tool names that flow through the Architect LLM.

If a tool name contains shell-special characters, the `format!("{path}:media/type")` argument could be misinterpreted by `oras`. The `display()` method does not escape for shell context.

**Attack vector:** A tool name containing characters like spaces, quotes, or colons could cause `oras` to misparse the `file:mediatype` argument format. The `:` character in the media type suffix makes this format particularly sensitive.

**Impact:** OCI push failure or misdirected push (low severity given the current single-colon parsing).

**Remediation:** Validate that wasm_path and other file paths do not contain colons (which conflict with the `file:mediatype` oras format). Or use oras's `--config` flag for structured input.

---

### LOW

#### L-1: No HTTP request timeout on LLM client
**File:** `crates/girt-pipeline/src/llm.rs:48`
**Confidence:** 80

**Description:** The `reqwest::Client::new()` in `OpenAiCompatibleClient::new()` has no configured timeout. A hanging LLM server will cause the pipeline to block indefinitely.

**Impact:** Denial of service (resource exhaustion). The pipeline goroutine/task holds a queue item in `in_progress` state indefinitely.

**Remediation:** Set a reasonable timeout:
```rust
http: reqwest::Client::builder()
    .timeout(std::time::Duration::from_secs(120))
    .build()
    .unwrap_or_default(),
```

#### L-2: `girt.toml` committed to git can accumulate secrets
**File:** `girt.toml`, `.gitignore`
**Confidence:** 75

**Description:** `girt.toml` supports `api_key` and `token` fields but is tracked in git. The committed version correctly has no secrets. However, the file is not in `.gitignore`, so a contributor who adds their API key locally could accidentally commit it.

**Impact:** API key exposure in version control history.

**Remediation:** Add `girt.toml` to `.gitignore` and ship `girt.toml.example` as the template, or add a pre-commit hook that rejects `girt.toml` changes containing `api_key` values.

---

## Positive Security Observations

1. **API key from env var takes precedence** (`config.rs:92`): `GIRT_LLM_API_KEY` env var overrides the config file, which is the correct pattern for secrets.

2. **Bearer auth only sent when key is present** (`llm.rs:86-88`): The conditional `if let Some(key) = &self.api_key` prevents sending an empty auth header.

3. **Error responses do not leak API keys**: The error messages in `llm.rs:91-95` include the HTTP status and response body but not the request headers or API key.

4. **WASM sandbox isolation**: The tool runs inside Wassette/Wasmtime, which provides the critical security boundary. The pipeline vulnerabilities identified here affect what *gets into* the sandbox, not what can escape it.

5. **Temp directory isolation**: The compiler uses `tempfile::tempdir()` which creates directories with restricted permissions.

6. **OCI push verifies file existence** (`publish.rs:96-103`): Pre-flight check prevents pushing nonexistent files.

7. **Compiler error output is bounded**: `String::from_utf8_lossy` is used instead of `.unwrap()` for subprocess output.

---

## Recommendations Summary

| ID | Severity | Component | Action |
|----|----------|-----------|--------|
| H-1 | HIGH | QA/Red Team agents | Change default-on-parse-failure from `passed: true` to `passed: false` |
| M-1 | MEDIUM | Compiler | Sanitize tool_name before path construction |
| M-2 | MEDIUM | OCI Publisher | Validate paths for oras argument format safety |
| L-1 | LOW | LLM Client | Add HTTP request timeout |
| L-2 | LOW | Config | Protect girt.toml from accidental secret commits |
