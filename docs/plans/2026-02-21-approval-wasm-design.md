# Approval WASM — Technical Design

**Date:** 2026-02-21  
**ADR:** ADR-011  
**Status:** Design / Implementation planning

---

## Overview

This doc covers the concrete implementation plan for the pluggable approval
system. The Discord approval WASM is built first (partly via the GIRT pipeline
itself as a technology showcase), then wired into the proxy.

---

## Phase 1: Infrastructure (Rust changes)

### 1a. New WIT world file

`crates/girt-runtime/wit/approval-world.wit`:

```wit
package girt:approval;

world approval-provider {
    use wasi:http/outgoing-handler@0.2.0;
    use wasi:clocks/wall-clock@0.2.0;

    export request-approval: func(
        question: string,
        context: string,
    ) -> result<approval-result, string>;
}

record approval-result {
    approved: bool,
    user-message: option<string>,
    authorized-by: string,
    evidence-url: string,
}
```

### 1b. `ApprovalConfig` — new `girt.toml` section

```rust
#[derive(Debug, Default, Deserialize)]
pub struct ApprovalConfig {
    pub wasm_path: Option<String>,
    pub timeout_secs: Option<u64>,
    pub authorized_users: Vec<String>,
}
```

Added to `GirtConfig` alongside `pipeline`.

### 1c. `ApprovalManager` — new struct in `girt-runtime`

Wraps `LifecycleManager` to load and call an approval provider WASM with:
- Distinct timeout budget (from `ApprovalConfig::timeout_secs`)
- `ApprovalContext` JSON assembly
- Result parsing from the WASM return value

```rust
pub struct ApprovalManager {
    lifecycle: Arc<LifecycleManager>,
    config: ApprovalConfig,
}

impl ApprovalManager {
    pub async fn request_approval(
        &self,
        question: &str,
        context: ApprovalContext,
    ) -> Result<ApprovalResult, ApprovalError>;
}
```

### 1d. Proxy integration

`GirtProxy` gets an `Option<Arc<ApprovalManager>>`. In
`handle_request_capability`, when the Creation Gate returns ASK:

```rust
Decision::Ask { reason } => {
    match &self.approval_manager {
        Some(manager) => {
            let result = manager.request_approval(&reason, context).await?;
            if result.approved {
                self.trigger_build(spec).await  // proceed
            } else {
                // return denial with evidence
            }
        }
        None => Ok(make_tool_result(/* return ASK to caller */))
    }
}
```

---

## Phase 2: Discord Approval WASM

This is the interesting part — **built using the GIRT pipeline itself**.

### Spec submitted to `request_capability`

```json
{
  "name": "discord_approval",
  "description": "Send a human-approval request to a Discord channel and wait for a response. Accepts JSON input with: question (string), context (string), channel_id (string), bot_token (string), authorized_users (array of strings, empty = any user), timeout_secs (number). Posts an embed to the channel, reacts with 👍 and 👎, polls for the first reaction or reply from an authorized user. Returns JSON: { approved: bool, user_message: string|null, authorized_by: string, evidence_url: string }. Uses WASI HTTP for all Discord API calls. Polls every 10 seconds. Returns { approved: false, authorized_by: 'timeout', evidence_url: '' } on timeout.",
  "inputs": {
    "question": "string — the question to put to the human",
    "context": "string — JSON context for display in the embed",
    "channel_id": "string — Discord channel snowflake ID",
    "bot_token": "string — Discord bot token (Bearer)",
    "authorized_users": "array<string> — usernames that may respond (empty = any)",
    "timeout_secs": "number — seconds to wait before auto-deny"
  },
  "outputs": {
    "approved": "bool",
    "user_message": "string | null",
    "authorized_by": "string",
    "evidence_url": "string"
  },
  "constraints": {
    "network": ["discord.com", "discordapp.com"],
    "storage": [],
    "secrets": []
  }
}
```

> **Note:** `bot_token` is in the input schema here because the pipeline builds
> it as a general-purpose tool. When the proxy calls it as an approval provider,
> the host injects the token via `host_auth_proxy` rather than passing it raw.
> The WASM handles both: if `bot_token` is in input, use it; if empty, call
> `host_auth_proxy("discord-bot")` (not yet implemented — Phase 3).

### Key implementation details for the Engineer

The pipeline prompt (via `coding_standards_path`) now includes CLAUDE.md, so
the Engineer gets: fail-fast, no hardcoded secrets, idempotency, modular code.

Additional constraints injected via the spec description:
- Must use `wasi:http/outgoing-handler` for all Discord API calls (no std HTTP)
- Polling loop with `wasi:clocks` sleep between iterations
- Parse Discord API JSON responses for reactions and message replies
- Return early on first valid response — don't wait for timeout if answered

### Discord API calls the WASM makes

```
POST   /api/v10/channels/{channel_id}/messages          → send embed
POST   /api/v10/channels/{channel_id}/messages/{id}/reactions/👍/@me  → react
POST   /api/v10/channels/{channel_id}/messages/{id}/reactions/👎/@me  → react
GET    /api/v10/channels/{channel_id}/messages/{id}/reactions/👍       → poll
GET    /api/v10/channels/{channel_id}/messages/{id}/reactions/👎       → poll
GET    /api/v10/channels/{channel_id}/messages?after={id}&limit=5      → poll replies
```

### Expected pipeline outcome

The pipeline will likely need 2-3 iterations (WASI HTTP boilerplate is
non-trivial). Realistic outcomes:
- **Best case:** builds in iteration 1-2, QA/RedTeam passes → `.cwasm` ready
- **Likely:** iteration 2-3 with QA catching WASI HTTP mistakes → fixed
- **Circuit breaker:** if Discord API response parsing is too complex → manual
  fallback to hand-written version

---

## Phase 3: `host_auth_proxy` for Discord

Once the WASM exists, the proxy injects the bot token via the host rather than
via the input JSON. This removes the token from the call site entirely.

The `EnvSecretStore` already maps `"discord"` → `DISCORD_BOT_TOKEN`. The
`host_auth_proxy` host function needs to be exposed in the WASM linker.

---

## Phase 4: Registry + Bootstrap

- Commit the compiled `discord-approval.cwasm` to the repo as a bootstrap binary
- Add `scripts/install.sh` step to copy it to `~/.config/girt/approval/`
- Document in user guide: first run bootstraps approval, subsequent runs are gated

---

## Metadata recorded per approval

Every `CapabilityRequest` gets an `approval` field added to its metadata:

```json
{
  "approval": {
    "approved": true,
    "authorized_by": "liamhelmer",
    "evidence_url": "https://discord.com/channels/1473159530316566551/.../...",
    "timestamp": "2026-02-21T04:00:00Z",
    "provider": "discord"
  }
}
```

This is included in the final `request_capability` tool result and the
request's JSON file in the queue.

---

## GIRT Tool Convention: Continue Signal

WASM components have a natural execution time limit (~60 seconds). Operations
that may take longer — waiting for human input, polling an external queue,
streaming a large result — must not block indefinitely. Instead, they use the
**continue-signal pattern**:

1. The tool runs for up to its execution budget, does useful work, and returns
2. If the operation is not complete, it returns `status: "pending"` plus a
   **resume token** (an opaque identifier the next invocation uses to pick up
   where it left off — e.g. a Discord `message_id`, a job ID, a cursor)
3. The caller (MCP client, orchestrator, Claude Code) re-invokes with the
   resume token; the tool skips setup and goes straight to checking status
4. The loop continues until the tool returns a terminal status (`approved`,
   `denied`, `complete`, `failed`, etc.) or the caller's overall deadline
   expires

**Responsibilities:**
- **WASM tool**: do work, return terminal or pending. Never loops indefinitely.
  Per-invocation timeout: ≤60s. Poll interval: reasonable (5–15s).
- **Caller**: owns the re-invocation loop, overall deadline, escalation,
  cleanup. Interprets `pending` as "call me again with this token."

**This is standard practice** (pagination cursors, chunked responses, async
job polling). GIRT tools that can exceed 60s must document their resume token
in the output schema and indicate in their description that `pending` means
re-invoke.

The discord_approval WASM is the first GIRT tool implementing this pattern:
- First call: post message, poll for ≤60s, return `{status, message_id}`
- Re-invocation: pass `message_id`, skip posting, poll for ≤60s, return again
- Caller loops until `approved`/`denied` or overall timeout

---

## Implementation Order

1. [ ] ADR-011 merged (this branch)
2. [ ] Phase 1a: `approval-world.wit`
3. [ ] Phase 1b: `ApprovalConfig` in `config.rs`
4. [ ] Phase 1c: `ApprovalManager` in `girt-runtime`
5. [ ] Phase 1d: Proxy integration
6. [ ] **Phase 2: Build Discord WASM via pipeline** ← dogfood moment
7. [ ] Phase 3: `host_auth_proxy` for Discord token
8. [ ] Phase 4: Registry + bootstrap binary + install script update
