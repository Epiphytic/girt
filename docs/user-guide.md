# GIRT User Guide

## What is GIRT?

GIRT (Generative Isolated Runtime for Tools) is a multi-agent tool factory that dynamically generates, tests, and publishes sandboxed WebAssembly tools. It sits as an MCP proxy in front of [Wassette](https://github.com/microsoft/wassette), intercepting capability requests and tool calls through a decision cascade.

When an AI agent needs a tool that doesn't exist, GIRT can build it on demand: an Architect refines the spec, an Engineer generates the code, QA tests it, and a Red Team audits it — all automatically.

## Prerequisites

- Rust toolchain (stable, edition 2024)
- [Wassette](https://github.com/microsoft/wassette) installed and on PATH
- Claude Code (for plugin usage)

## Quick Start

### Build from source

```bash
git clone https://github.com/Epiphytic/girt.git
cd girt
cargo build --workspace
```

### Run the proxy

```bash
# Start GIRT with default Wassette binary
cargo run --bin girt -- --wassette-bin wassette

# Or with custom Wassette path and args
cargo run --bin girt -- --wassette-bin /path/to/wassette --wassette-args --policy-dir /etc/wassette/policies
```

GIRT listens on stdio (MCP transport) and spawns Wassette as a child process.

### Verify it works

```bash
# Run the smoke tests
cargo test --workspace

# Run the E2E verification script (requires Wassette on PATH)
./scripts/verify-proxy.sh
```

## How It Works

### Decision Gates

Every tool interaction passes through decision gates:

**Execution Gate** (on every `call_tool`):
1. Policy rules — pattern matching for known-good/known-bad
2. Decision cache — previous decisions are remembered
3. LLM evaluation — AI assesses the request
4. HITL — human-in-the-loop for uncertain cases

**Creation Gate** (on `request_capability`):
1. Policy rules
2. Decision cache
3. Registry lookup — check if the tool already exists in OCI registries
4. CLI check — defer to native utilities (jq, curl, ripgrep, etc.)
5. Similarity check — embedding-based matching against existing tools
6. LLM evaluation
7. HITL

### Build Pipeline

When the Creation Gate approves a request:

1. **Architect** refines the capability spec into a generic, reusable tool design
2. **Engineer** generates code (Rust, Go, or AssemblyScript) targeting WASM Components
3. **QA** runs functional correctness tests
4. **Red Team** performs adversarial security auditing
5. If issues found, bug tickets route back to Engineer (max 3 iterations)
6. Passing tools are published to the local cache and OCI registries

### Default Policy Rules

**Auto-denied** (security threats):
- Shell execution, credential extraction
- Filesystem root access, cloud metadata SSRF (169.254.169.254)
- Wildcard network access

**Auto-allowed** (safe operations):
- Math/calculation operations
- String/text manipulation

## Configuration

GIRT reads configuration from `girt.toml`:

```toml
[registries]
# Default registries (Epiphytic public is included automatically)
[[registries.additional]]
name = "my-org"
url = "ghcr.io/my-org/girt-tools"

[build]
# Default language for new tools
default_language = "rust"  # rust | go | assemblyscript

# Resource tier for generated policy.yaml
default_tier = "standard"  # minimal | standard | extended

[logging]
# Set via GIRT_LOG env var
# GIRT_LOG=debug cargo run --bin girt
level = "info"
```

## Resource Tiers

Generated tools get resource limits in their Wassette policy:

| Tier | Memory | Timeout | Max Response |
|------|--------|---------|-------------|
| Minimal | 64 MB | 5s | 1 MB |
| Standard | 128 MB | 15s | 5 MB |
| Extended | 512 MB | 60s | 20 MB |

## Environment Variables

| Variable | Purpose | Default |
|----------|---------|---------|
| `GIRT_LOG` | Log level filter | `info` |
| `GIRT_QUEUE_DIR` | Queue directory | `~/.girt/queue/` |
| `GIRT_TOOLS_DIR` | Tool cache directory | `~/.girt/tools/` |

## Crate Structure

| Crate | Purpose |
|-------|---------|
| `girt-proxy` | MCP proxy binary, CLI entry point |
| `girt-core` | Decision engine, hookwise cascade layers |
| `girt-pipeline` | Build pipeline agents, queue, cache, metrics |
| `girt-secrets` | Secret store facade for WASM tools |
