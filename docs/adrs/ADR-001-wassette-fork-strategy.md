# ADR-001: Wassette Fork Strategy

**Status:** Superseded by [ADR-010](./ADR-010-embedded-wasm-runtime.md)
**Date:** 2026-02-20
**Superseded:** 2026-02-20
**Context:** When and why to fork Microsoft's Wassette runtime

> **Note:** This ADR assumed Wassette would be used as an external subprocess dependency.
> That approach was abandoned in favour of an embedded `girt-runtime` crate that ports
> Wassette's execution core directly. See ADR-010 for the current decision.

---

## Decision

GIRT adopts [Wassette](https://github.com/microsoft/wassette) as its WASM execution runtime. We will contribute upstream and consume releases as a dependency. We fork **only** under one of these conditions:

1. **Wassette becomes bloated or unusable.** If the project grows in scope beyond what GIRT needs and the bloat impacts performance, binary size, or maintainability, we fork to a minimal subset.
2. **It becomes easier to write our own.** If GIRT's requirements diverge far enough from Wassette's direction that maintaining compatibility costs more than a clean-room implementation, we build our own.
3. **Upstream is hostile to contributions.** If the Wassette maintainers are consistently reluctant to accept PRs or feature requests that GIRT needs, and we accumulate a significant patch set that can't land upstream, we fork to avoid carrying an unbounded maintenance burden.

## Context

Wassette (v0.4.0, MIT licensed) provides the core capabilities GIRT needs: Wasmtime sandboxing, MCP server integration, policy-based permissions, OCI component loading, and WIT introspection. Building these from scratch would cost months of effort for equivalent functionality.

However, Wassette is early-stage, Microsoft-maintained, and its roadmap may diverge from GIRT's needs. We need a clear threshold for when to stop tracking upstream and take ownership.

## Rationale

- **Fork is a last resort, not a goal.** Consuming upstream means we get security patches, new WASI features, and community contributions for free.
- **The facade pattern protects us.** GIRT's MCP proxy layer abstracts over Wassette. If we fork or replace it, only the proxy internals change — the pipeline, plugin, and agent definitions are unaffected.
- **MIT license means forking is always an option.** There's no legal barrier. The question is purely about maintenance cost.

## Consequences

- We track Wassette releases and test compatibility as part of CI.
- We contribute features we need upstream before building workarounds.
- We maintain a list of patches/workarounds we carry locally. If this list exceeds 5 significant items, we re-evaluate.
- The GIRT MCP proxy is designed so that swapping Wassette for an alternative runtime requires changes only in the proxy layer.
