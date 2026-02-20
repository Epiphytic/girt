# GIRT - Generative Isolated Runtime for Tools

## Overview

GIRT is a multi-agent tool factory that dynamically generates, tests, and publishes sandboxed WebAssembly tools on demand. It sits as an MCP proxy in front of Wassette (Microsoft's WASM runtime for MCP).

## Architecture

```
Agent -> GIRT MCP Proxy -> Decision Engine -> Build Pipeline -> Wassette
```

### Decision Engine (girt-core)

6-layer Hookwise cascade for both Creation and Execution gates:
1. Policy Rules (regex pattern matching)
2. Decision Cache (spec hash lookup)
3. Registry Lookup (OCI registry query)
4. CLI Check (known CLI utilities)
5. LLM Evaluation (Anthropic API)
6. HITL (human-in-the-loop escalation)

### Build Pipeline (girt-pipeline)

Architect -> Engineer -> QA + Red Team with circuit breaker (max 3 iterations):
- Architect: refines capability request into generic tool spec
- Engineer: generates Rust WASM Component source code
- QA: functional correctness testing
- Red Team: adversarial security auditing
- Bug tickets route back to Engineer for fixes
- Circuit breaker halts after 3 failed iterations

### Local Paths

- Queue: `~/.girt/queue/{pending,in_progress,completed,failed}/`
- Tool cache: `~/.girt/tools/<tool_name>/manifest.json`
- Audit log: `~/.girt/audit.log`

## Development

```bash
# Build
cargo build --workspace

# Test
cargo test --workspace

# Clippy
cargo clippy --workspace -- -D warnings

# Run (requires Wassette installed)
cargo run --bin girt -- --wassette-bin wassette
```

## Plugin Commands

- `/girt-status` -- Show pipeline, queue, and tool cache status
- `/girt-build [name_or_spec]` -- Manually trigger a capability build
- `/girt-registry [list|add|remove]` -- Manage OCI registry config
