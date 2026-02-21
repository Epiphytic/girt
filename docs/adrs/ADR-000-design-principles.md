# ADR-000: GIRT Design Principles

**Status:** Active  
**Date:** 2026-02-21  
**Scope:** All of GIRT — this document grounds every subsequent ADR

---

## Primary Purpose

**GIRT exists to reduce context utilization for the calling agent.**

When an AI agent needs a tool that doesn't exist yet, the naive approach is to write the code inline — using the agent's context window to design, implement, debug, and verify the tool. This is expensive, brittle, and fundamentally wrong: the agent's context is a finite resource that should be spent on the task at hand, not on software engineering.

GIRT inverts this. The calling agent says what it needs in plain terms. GIRT's pipeline handles everything else: design, security review, implementation, quality assurance, and deployment. The calling agent gets back a working, sandboxed tool. It never has to think about how that tool was built.

> **The calling agent should be woefully oblivious to implementation details.**  
> It describes intent. GIRT produces capability.

---

## Consequences of the Primary Purpose

Every design decision in GIRT should be evaluated against this question:  
*Does this make calling agents do less work, or more work?*

This drives several concrete constraints:

### 1. The caller describes WHAT, not HOW

The `request_capability` interface accepts:
- A name
- A plain-language description of what the tool should do
- Input/output types (optional — the Architect can infer these)

The caller must **never** need to specify:
- Which external APIs or protocols to use
- Security requirements (input validation, injection prevention, rate limits)
- Implementation patterns (polling strategy, retry logic, timeout handling)
- WIT interface definitions
- Build toolchain choices

If a calling agent finds itself thinking about WASI HTTP or Discord API endpoints in order to request a capability, GIRT has failed.

### 2. The pipeline owns complexity

Complexity that would otherwise consume caller context belongs in the pipeline:

| Caller's job | Pipeline's job |
|---|---|
| "I need a Discord approval bot" | Choose the Discord REST API, WASI HTTP, auth pattern |
| "Approve risky actions" | Add input validation, injection prevention, resource limits |
| "Wait for a human response" | Design the polling algorithm, timeout handling, backoff |
| "Record who approved" | Define the audit trail format, evidence URL structure |

The Architect agent is responsible for expanding a minimal request into a complete, secure specification. It does not pass through what it receives — it makes decisions.

### 3. Pipeline artifacts are the unit of reuse

A GIRT tool is a compiled WASM component stored in the tool cache and optionally published to an OCI registry. Once built, it costs the caller nothing to invoke — no context, no generation, just a function call. This is the payoff: **one pipeline run, infinite cheap invocations**.

Tools should be designed for reuse across sessions and agents. The Architect's SCOPE principle (don't over-engineer) and COMPLETE SPEC principle (don't under-specify) both serve this goal.

### 4. The approval mechanism is itself a tool

The human-in-the-loop approval flow (ADR-011) exemplifies the design: rather than returning a question to the caller and consuming its context waiting for an answer, GIRT handles the entire human interaction internally via a WASM approval component. The caller fires `request_capability` and eventually gets a result — it doesn't manage the Discord polling, the authorization check, or the timeout.

This pattern generalises: any complexity that would otherwise fragment the caller's attention belongs inside a GIRT component.

---

## Secondary Principles

These matter but are subordinate to the primary purpose:

**Security by default.** WASM sandboxing, policy gates, Red Team review. Tools that pass the pipeline can be trusted. The calling agent should not have to audit tool code.

**Pipeline quality gates, not caller vigilance.** QA and Red Team agents catch bugs and vulnerabilities so the calling agent doesn't have to reason about whether the tool is safe.

**Composability over monoliths.** Small, focused tools composed at call time are better than large tools with many modes. Fewer failure modes, easier reuse.

**Dogfooding.** GIRT's own infrastructure components (approval WASM, future monitoring tools) are built through the GIRT pipeline. If GIRT can't build its own tools, it hasn't met the bar.

---

## What This Document Is Not

This is not a technical specification. It is a statement of purpose that should be consulted when ADRs conflict or when a design choice is unclear. When in doubt, ask: *does this make the calling agent do more work or less?*
