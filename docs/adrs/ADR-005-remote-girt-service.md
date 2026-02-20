# ADR-005: Remote GIRT Service

**Status:** Accepted
**Date:** 2026-02-20
**Context:** Whether the build pipeline runs locally or as a cloud service

---

## Decision

### Phase 1 (POC): Local Claude Code Agent Team

The initial implementation runs the entire GIRT pipeline locally as a Claude Code agent team (see [ADR-007](./ADR-007-claude-agent-team-orchestration.md)). The Pipeline Lead, Architect, Engineer, QA, and Red Team all run on the user's machine within Claude Code sessions.

### Phase 2: Configurable Cloud Build Service

The second version adds the ability to offload the build pipeline to a remote service. Users configure this in `girt.toml`:

```toml
# girt.toml
[build]
mode = "local"          # "local" (Claude Code agent team) or "remote"

[build.remote]
endpoint = "https://girt.epiphytic.dev/api/v1/build"
auth = "github"         # Authentication method
timeout_seconds = 300   # Max wait for remote build
```

### Remote Service Architecture

```
Local (User's Machine)                Remote (Cloud)
──────────────────────                ────────────────

GIRT MCP Proxy                        GIRT Build Service
  │                                     │
  │ POST /build {spec}                  ├── Architect Agent
  │────────────────────────────────────►├── Engineer Agent
  │                                     ├── QA Agent (Wassette sandbox)
  │◄─── 202 Accepted {build_id}        ├── Red Team Agent (Wassette sandbox)
  │                                     │
  │ GET /build/{id}/status              │
  │────────────────────────────────────►│
  │◄─── {status: "building", ...}       │
  │                                     │
  │ GET /build/{id}/status              │
  │────────────────────────────────────►│
  │◄─── {status: "complete",            │
  │      artifact: "oci://..."}         │
  │                                     │
  │ load-component(oci://...)           │
  │──► Wassette                         │
```

### When Remote Makes Sense

- **Build speed.** The remote service can maintain warm compilation caches, pre-loaded Wassette instances, and dedicated GPU for embedding similarity checks.
- **Cost optimization.** Remote builds can use cheaper LLM tiers or batched API calls.
- **Shared learnings.** The remote service accumulates build success/failure patterns across all users, improving the Engineer's effectiveness.
- **CI/CD integration.** Build pipelines that don't have Claude Code available can use the remote API directly.

## Context

Running 4-5 LLM agents locally for every tool build is expensive in both time and API cost. A cloud service amortizes infrastructure costs and provides caching benefits that individual users can't achieve alone.

However, a cloud dependency introduces availability risk, latency, and trust concerns (capability specs may contain sensitive information about what users are building).

## Rationale

- **Local-first is the right default.** Users should be able to use GIRT without any cloud dependency. The POC proves the architecture works entirely locally.
- **Remote is an optimization, not a requirement.** The same pipeline runs in both modes — the difference is where the agents execute.
- **Phased rollout reduces risk.** Building local-first means the remote service is additive, not foundational. If the service goes down, users fall back to local builds.

## Consequences

- The GIRT MCP proxy must abstract over local vs. remote build execution. A `BuildBackend` trait/interface with `local` and `remote` implementations.
- The remote API must accept the same capability spec format and return the same artifact format.
- Capability specs sent to the remote service should be treated as potentially sensitive. TLS is mandatory. Consider end-to-end encryption for spec contents.
- The remote service is out of scope for the initial implementation. Phase 2 timing is TBD based on adoption and demand.
