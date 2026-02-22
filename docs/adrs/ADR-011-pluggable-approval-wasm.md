# ADR-011: Pluggable Human-in-the-Loop Approval via WASM Components

**Status:** Proposed  
**Date:** 2026-02-21  
**Context:** GIRT pipeline — human approval integration

---

## Decision

Human-in-the-loop approval requests (Creation Gate ASK outcomes, Architect ambiguity, circuit breaker escalations) are fulfilled by a **WASM component implementing the `girt:approval` WIT world**. The component is loaded by girt-runtime and called in-process, exactly like any other GIRT tool. The first implementation targets Discord.

---

## Context

The Creation Gate can return three decisions: ALLOW, DENY, and ASK. Currently ASK is returned as a raw tool result back to the calling agent (Claude Code), which works only when Claude Code's `AskUserQuestion` is in the loop. For:

- **Standalone `girt` daemon** (background service, no Claude Code session)
- **Architect ambiguity** (spec too vague to build safely without clarification)
- **Circuit breaker escalation** (three failed iterations — needs human intervention)

…there is currently no mechanism to surface the question to a human and wait for a response. The operator simply receives an error or an empty ASK result.

---

## Options Considered

**A. Hardcode Discord integration in the proxy**  
Simple to ship first. Tight coupling, not portable, requires recompile to change.

**B. Webhook callback (HTTP POST + wait)**  
Generic but requires the operator to run a receiver. High friction for new users.

**C. WASM approval component (chosen)**  
The approval mechanism is itself a GIRT component — same sandbox, same registry, same build pipeline. Operators swap providers by swapping a `.cwasm` file.

---

## Architecture

### WIT World — `girt:approval`

```wit
package girt:approval;

/// The interface an approval provider WASM must export.
world approval-provider {
    export request-approval: func(
        question: string,
        context: string,    // JSON-encoded ApprovalContext
    ) -> result<approval-result, string>;
}

record approval-result {
    /// true = approved, false = denied
    approved: bool,
    /// Text reply from the approver (if they wrote one, vs just reacting)
    user-message: option<string>,
    /// Human-readable identity of who approved/denied (e.g. "liamhelmer")
    authorized-by: string,
    /// Permalink to the approval message/thread for audit trail
    evidence-url: string,
}
```

`ApprovalContext` (JSON, passed as `context` string):
```json
{
  "request_id": "req_abc123",
  "spec_name": "add_two_numbers",
  "spec_description": "...",
  "gate": "creation",
  "reason": "Architect needs clarification on overflow behaviour"
}
```

### Configuration (`girt.toml`)

```toml
[approval]
# Path to the compiled approval provider WASM.
# Supports ~ expansion. If absent, ASK falls back to returning the question
# as a tool result (Claude Code / interactive mode).
wasm_path = "~/.config/girt/approval/discord-approval.cwasm"

# How long to wait for a response before auto-denying.
timeout_secs = 1800   # 30 minutes

# Which Discord users may approve requests.
# Empty list = any member of the configured channel may respond.
authorized_users = ["liamhelmer"]
```

Provider-specific config (Discord example) lives in a `[approval.provider]` table
and is serialised to JSON and passed to the WASM via the `context` argument's
`provider_config` field, injected by the proxy before calling the component.

### Secret Handling

The Discord bot token is stored via girt-secrets and injected by the host
using the existing `host_auth_proxy` mechanism. The WASM never sees the raw
token; the host signs/proxies Discord API requests on its behalf.

```
girt-secrets lookup("discord-bot") → bearer token
  → injected as Authorization header on outbound WASI HTTP calls
```

This follows the same zero-knowledge pattern used by all GIRT tool WASMs.

### Proxy Integration Points

Two call sites in `girt-proxy`:

1. **Creation Gate ASK** (`proxy.rs::handle_request_capability`)  
   Current: returns ASK result as tool content.  
   New: if approval WASM configured → call WASM → proceed or deny based on result.

2. **Circuit breaker escalation** (`orchestrator.rs`, future)  
   After 3 failed iterations → call approval WASM with escalation context →
   operator decides whether to retry, abort, or supply clarification.

Approval metadata (`authorized_by`, `evidence_url`) is appended to the
`CapabilityRequest` metadata and included in the final tool result JSON so
the calling agent has a full audit trail.

### Discord WASM — Behaviour

1. Use WASI HTTP to POST a Discord embed message to the configured channel:
   - Title: "🔔 GIRT Approval Required"
   - Fields: spec name, gate, reason
   - Footer: request ID + timeout
2. React to the message with 👍 and 👎 using Discord API
3. Poll `GET /channels/{id}/messages/{id}/reactions` every 10 seconds
4. Accept the **first reaction or reply** from an authorized user:
   - 👍 reaction → `approved: true`, `user-message: null`
   - 👎 reaction → `approved: false`, `user-message: null`
   - Text reply → `approved: true`, `user-message: <reply text>`
5. Return `approval-result` with the responder's username and message permalink
6. On timeout → return `approved: false`, `authorized-by: "timeout"`

### Runtime Considerations

Approval WASMs run much longer than tool WASMs (minutes, not milliseconds).
Two changes needed in girt-runtime:

- **Per-component timeout config**: `approval-provider` components get a
  separate timeout budget (default: `approval.timeout_secs + 30s` grace).
- **Cooperative polling**: the approval WASM yields between polls; no busy-spin.
  WASI HTTP calls are async-compatible via wasmtime-wasi-http.

### Bootstrap Problem

The Discord approval WASM must exist *before* it can be used to approve
capability requests. The initial WASM is built through the pipeline with
the Creation Gate in ALLOW mode (no approval needed), then registered as
the approval provider. Subsequent requests are then covered.

This is documented and expected — the first approval WASM is a bootstrap artifact.

---

## Consequences

### Positive
- **Dogfooding** — GIRT's approval mechanism is itself a GIRT component,
  demonstrating the full pipeline end-to-end on a non-trivial real use case
- **Pluggable** — Slack, email, web UI, CLI, SMS: any approval mechanism
  that fits the WIT interface works without changing the proxy
- **Auditable** — every approval carries `authorized_by` + `evidence_url`
  baked into request metadata and the final tool result
- **Registry-distributable** — approval WASMs can be published to the GIRT
  registry and shared across deployments
- **Showcase** — concrete demonstration of GIRT building production-useful
  components using its own pipeline

### Negative
- **Bootstrap friction** — first approval WASM must be built before approval
  flow is active (mitigated by providing a pre-built binary in the repo)
- **Longer-running components** — timeout handling in girt-runtime needs extension
- **Second WIT world** — `girt:approval` is a distinct interface from `girt:tool`;
  the compiler and registry need to handle both

### Neutral
- The `girt:tool` pipeline is unchanged; approval WASMs are loaded and called
  via the same `LifecycleManager` with a different component registration path

---

## Open Questions

1. Should the pre-built Discord approval WASM be committed to the repo as a
   bootstrap binary, or always built on first use?
2. Should `authorized_users` be part of `girt.toml` (static) or part of the
   WASM's own config (dynamic, swappable per-provider)?
3. Should circuit breaker escalation block (wait for approval) or just
   notify and fail-fast? Blocking is safer; async notify is more resilient
   to long human response times.
4. Multi-approver: require N of M approvals for higher-risk operations?
