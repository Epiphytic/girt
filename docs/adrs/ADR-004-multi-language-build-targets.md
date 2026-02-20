# ADR-004: Multi-Language Build Targets

**Status:** Accepted
**Date:** 2026-02-20
**Context:** How the Engineer agent selects the implementation language for WASM tools

---

## Decision

The Engineer agent selects the implementation language using two mechanisms, in priority order:

1. **User configuration** — If `girt.toml` specifies a `default_language`, the Engineer uses it unless there's a strong reason not to. Users can also specify a language per-request in the capability spec.
2. **Automatic selection** — If no language is specified, the Engineer selects automatically based on the capability type and its own assessment of which language produces the most reliable, performant result.

### Supported Languages

| Language | Status | Best For |
|---|---|---|
| **Rust** | Default, preferred | Performance-critical tools, complex logic, strong type safety |
| **Go** | Supported | Network-heavy tools, simpler logic, higher first-pass compilation success from LLM |
| **AssemblyScript** | Supported (Phase 5) | TypeScript-familiar users, quick prototyping |
| **C** | Supported (Phase 5) | Low-level operations, porting existing C libraries to WASM |

### Automatic Selection Heuristics

When the Engineer selects automatically, it considers:

- **Complexity of the logic** — Complex state machines, error handling, or type-rich domains favor Rust.
- **Network-heavy tools** — Tools that primarily make HTTP calls and transform JSON may succeed faster in Go due to simpler error handling patterns.
- **User ecosystem** — If the `girt-tools` repository already has similar tools in a specific language, prefer that language for consistency.
- **Build success history** — If the Engineer has failed to compile a similar tool in one language (tracked via the Pipeline Lead's learnings), try another.

### Configuration

```toml
# girt.toml
[build]
default_language = "rust"        # "rust", "go", "assemblyscript", "c"
# Per-request override via capability spec:
# { "constraints": { "language": "go" } }
```

## Context

LLM-generated Rust has a lower first-pass compilation rate (~30-50%) compared to Go (~60-70%) due to Rust's strict type system, borrow checker, and verbose error handling. However, Rust produces more correct and secure code when it does compile. Supporting multiple languages lets the pipeline optimize for reliability vs. speed depending on the tool's requirements.

## Rationale

- **User preference is respected.** If a team standardizes on Rust for auditability, they shouldn't get Go tools.
- **Automatic selection improves pipeline success rate.** Rather than always fighting the borrow checker for a simple HTTP-to-JSON tool, the Engineer can pick Go and succeed on the first try.
- **The WIT standard abstracts over language.** All languages compile to the same WASM Component with the same WIT interface. Consumers don't need to know or care what language a tool was written in.

## Consequences

- The Engineer's system prompt includes guidance for all supported languages, not just Rust.
- The `girt-tools` repository may contain tools in multiple languages. Build tooling must handle all supported languages.
- The GitHub Actions build pipeline needs language-specific compilation steps (cargo-component for Rust, tinygo for Go, asc for AssemblyScript).
- Phase 5 of the implementation plan covers adding Go and AssemblyScript support.
