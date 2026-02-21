# ADR-012: Planner Agent for Complex Capability Specs

**Status:** Proposed  
**Date:** 2026-02-21  
**Context:** Build pipeline — between Architect and Engineer

---

## Problem

The current pipeline (Architect → Engineer → QA/RedTeam loop) is reactive about security. The Engineer writes code from a spec, the Red Team finds issues, the Engineer fixes those specific issues — but each fix introduces new surface area. For complex components, the circuit breaker triggers before the code converges.

Observed with `discord_approval`:
- Iteration 1: 6 security findings
- Iteration 2: 3 security findings (different ones)
- Iteration 3: 3 more security findings (again, different)
- Never converges in 3–5 iterations

Root cause: the Engineer is solving a complex security problem reactively, one ticket at a time, without a complete threat model up front.

---

## Decision

Add a **Planner agent** that runs between the Architect and Engineer for sufficiently complex specs. The Planner thinks through the full implementation before any code is written: security threats, validation strategy, API call sequence, error handling, edge cases. The Engineer receives both the spec and the Planner's output as its primary reference.

The Planner is **not** an additional gate — it does not approve or reject. It is a planning document generator.

---

## Pipeline With Planner

```
Caller intent
    │
    ▼
Architect          ← refines vague request into complete spec
    │
    ▼
Complexity Check   ← is planning warranted? (see triggers below)
    │
   yes
    │
    ▼
Planner            ← threat-models and sequences the implementation
    │
    ▼
Engineer           ← codes from spec + plan (not from spec alone)
    │
    ▼
QA + Red Team      ← validate; loop back to Engineer if issues found
    │
    ▼
WasmCompiler → girt-runtime → tools/list_changed
```

---

## Planner Responsibilities

The Planner receives the Architect's refined spec and produces an `ImplementationPlan`:

```json
{
  "validation_layer": "All input validation before any API calls: ...",
  "security_notes": "Threat model and mitigations: ...",
  "api_sequence": "Step-by-step API call order with error handling: ...",
  "edge_cases": "Identified edge cases and how to handle each: ...",
  "implementation_guidance": "Language-specific patterns, crate recommendations: ..."
}
```

The Planner does NOT write code. It writes a structured implementation brief that answers every question the Engineer might have before the Engineer starts writing.

---

## Complexity Triggers

The Planner runs when the Architect's refined spec meets any of these criteria:

| Trigger | Rationale |
|---|---|
| `constraints.network` non-empty | External API calls introduce auth, injection, error handling complexity |
| `constraints.secrets` non-empty | Credential handling requires explicit security strategy |
| Description mentions async/polling | Timeout edge cases, loop termination, clock semantics |
| Multiple input fields with user-provided strings | Each is a potential injection surface |
| `complexity_hint: "high"` from Architect | Architect can explicitly flag complex specs |

The Architect output format gains an optional `complexity_hint` field:
```json
{
  "action": "build",
  "spec": { ... },
  "complexity_hint": "high",
  "design_notes": "..."
}
```

---

## Planner System Prompt (sketch)

```
You are a Senior Security Architect and Implementation Planner for sandboxed 
WebAssembly components. You do not write code. You produce implementation plans.

You receive a tool spec that has already been refined by an Architect. Your job 
is to think through the full implementation before any code is written:

1. THREAT MODEL: For each input field, what can an attacker do? CRLF injection, 
   path traversal, resource exhaustion, identity spoofing?
2. VALIDATION STRATEGY: What must be validated, in what order, before any 
   external calls?
3. API SEQUENCE: What external calls are needed, in what order? What does each 
   response look like? What errors can occur?
4. EDGE CASES: Empty lists, zero timeouts, extremely long strings, concurrent 
   callers, API rate limits?
5. IMPLEMENTATION GUIDANCE: Which patterns work well in WASM+WASI? What to avoid?

Output a structured plan the Engineer can follow directly. Be specific.
```

---

## What the Engineer Receives

Without Planner (current):
```
System: [ENGINEER_RUST_PROMPT]
User: Implement this tool spec: {spec_json}
```

With Planner:
```
System: [ENGINEER_RUST_PROMPT]
User: Implement this tool spec according to the implementation plan.

Spec:
{spec_json}

Implementation Plan:
{plan_json}

Follow the plan. Do not deviate from the validation strategy or API sequence 
described unless you have a compelling technical reason, which you must note 
in a code comment.
```

---

## Expected Impact

The discord_approval Red Team findings map directly to what a Planner would catch:
- "bot_token CRLF injection" → Planner: validation_layer
- "channel_id path traversal" → Planner: threat_model  
- "username mutability" → Planner: security_notes (use IDs, not names)
- "text reply parsing injection" → Planner: api_sequence (reactions only, no text parsing)
- "empty authorized_users semantics" → Planner: edge_cases (document the design decision)

Prediction: complex components that currently take 5+ iterations and still fail should converge in 1–2 iterations with a Planner.

---

## Consequences

### Positive
- Fewer Engineer iterations for complex components
- Security issues caught before code is written, not after
- The plan is an artifact — it can be logged and audited
- Consistent security posture across all complex components

### Negative
- One additional LLM call per complex build (Planner is a separate agent invocation)
- Planner quality depends on the LLM's security knowledge — not a substitute for human review of high-stakes tools
- Adds latency to the build for complex specs (~15–30s for the planning call)

### Neutral
- Simple tools (pure computation, no I/O) skip the Planner — no overhead for the common case
- The MAX_ITERATIONS circuit breaker is unchanged; the Planner reduces how many are needed, not the limit

---

## Implementation Steps

1. Add `complexity_hint` field to `RefinedSpec` / `SpecAction`
2. Add `ImplementationPlan` type to `types.rs`
3. Implement `PlannerAgent` in `crates/girt-pipeline/src/agent/planner.rs`
4. Add complexity check to `Orchestrator::build_loop()`
5. Update `EngineerAgent::build()` to accept `Option<ImplementationPlan>`
6. Update Architect prompt to include `complexity_hint` in output format
