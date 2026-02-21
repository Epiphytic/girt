# ADR-010: Embedded WASM Runtime (girt-runtime)

**Status:** Accepted
**Date:** 2026-02-20
**Supersedes:** ADR-001 (Wassette Fork Strategy)
**Context:** How GIRT executes WASM components

---

## Decision

GIRT will **not** use Wassette as an external subprocess dependency. Instead, GIRT will embed a `girt-runtime` crate — derived from Wassette's core WASM execution code — directly inside the `girt-proxy` binary.

`girt-runtime` is a port of the following Wassette internals (MIT licensed):

| Wassette source | girt-runtime equivalent | Purpose |
|---|---|---|
| `crates/wassette/src/runtime_context.rs` | `girt-runtime/src/runtime_context.rs` | Wasmtime engine + linker setup (WASI p2, WASI HTTP, WASI config) |
| `crates/wassette/src/lib.rs` (`LifecycleManager`) | `girt-runtime/src/lifecycle.rs` | Component load/unload/invoke, in-memory registry |
| `crates/wassette/src/component_storage.rs` | `girt-runtime/src/storage.rs` | Filesystem layout, `.cwasm` precompile cache, validation stamps |
| `crates/wassette/src/wasistate.rs` | `girt-runtime/src/wasistate.rs` | Per-invocation WASM state, resource limiter, secrets |
| `crates/component2json/` | `girt-runtime/src/schema.rs` | WIT-to-MCP tool schema derivation |
| `crates/policy/` | `girt-runtime/src/policy.rs` | `policy.yaml` parsing, resource limit + permission enforcement |

**What is NOT ported:**
- Wassette's MCP server wrapper (GIRT has its own in `girt-proxy`)
- Wassette's built-in `load-component` / `unload-component` MCP tools (replaced by GIRT's `request_capability` + pipeline)
- Wassette's OCI search and component-registry.json tooling (registry integration handled separately by GIRT)

## Context

### Why Not Wassette as a Subprocess?

The original architecture spawned Wassette as a child process and spoke MCP over stdio. This approach has critical gaps:

1. **Dynamic component loading is opaque.** There is no documented Wassette API for loading a newly built component into a running instance. After GIRT's build pipeline produces a `.wasm`, there is no clean way to make it callable.
2. **Wassette is not production-ready.** Microsoft explicitly labels it early-stage. It changes significantly between releases. Tracking it as a binary dependency adds fragile compatibility concerns.
3. **Two execution paths never connect.** The proxy path and the queue consumer path both call `Orchestrator::run()` but only the queue consumer path reaches `WasmCompiler::compile()`. There is no clean splice point in the subprocess model.
4. **Subprocess model adds operational complexity.** Two processes to manage, stdio plumbing, child process lifecycle, no shared memory.

### Why Not a Clean-Room Implementation?

Wassette has solved hard problems well:
- The Wasmtime engine + linker configuration for WASI p2 with component model support
- The `LifecycleManager` pattern for dynamic component loading/unloading
- The `component2json` WIT introspection for deriving MCP tool schemas
- The `policy.yaml` → Wasmtime resource limits mapping

Reimplementing these from scratch would take significant time for no architectural benefit. The MIT license makes porting both legal and practical.

### Why Not Just Fork Wassette?

ADR-001's fork criteria were "bloated, easier to rewrite, or upstream hostile." None of these exactly apply. The issue is architectural: GIRT needs the runtime *embedded*, not as a peer process. Forking implies maintaining Wassette as a whole project; we only need its execution core.

## Architecture Consequence

`girt-proxy` no longer spawns Wassette as a child process. It instead holds a `LifecycleManager` in-process:

```
Before:
  Agent → [girt-proxy via MCP stdio] → [Wassette subprocess via MCP stdio]

After:
  Agent → [girt-proxy via MCP stdio, girt-runtime embedded]
```

After the build pipeline produces a `.wasm` artifact:

```rust
// Before (broken — no API for this):
// ???

// After (girt-runtime):
lifecycle_manager.load_component("file:///path/to/tool.wasm").await?;
peer.notify_tool_list_changed().await?;
```

The "load-component gap" identified in the gap analysis disappears entirely: `LifecycleManager::load_component()` is the API.

## Consequences

- `crates/girt-runtime` is added to the workspace. It is the only crate that depends on `wasmtime`, `wasmtime-wasi`, `wasmtime-wasi-http`, and `wasmtime-wasi-config`.
- `crates/girt-proxy` replaces its `Peer<RoleClient>` (Wassette MCP client) with a `LifecycleManager`. The `GirtProxy` struct changes accordingly.
- `crates/girt-proxy/src/main.rs` initializes `LifecycleManager` instead of spawning a Wassette subprocess.
- The `--wassette-bin` and `--wassette-args` CLI flags are removed.
- All references to Wassette in system prompts, agent definitions, and documentation are updated to refer to `girt-runtime` or "the GIRT WASM runtime."
- ADR-001 is superseded by this ADR.
- License attribution for ported Wassette code is maintained in `NOTICE` and per-file copyright headers as required by the MIT license.

## Attribution

Ported code originates from [microsoft/wassette](https://github.com/microsoft/wassette), MIT License.
Copyright (c) Microsoft Corporation.
