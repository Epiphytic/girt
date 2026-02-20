# ADR-007: Claude Agent Team Orchestration

**Status:** Proposed
**Date:** 2026-02-20
**Context:** GIRT build pipeline orchestration model

---

## Decision

The GIRT build pipeline (Creation Gate → Architect → Engineer → QA → Red Team → Publish) will be orchestrable as a **Claude Code agent team** via a custom Claude Code plugin. The team runs as a persistent loop, consuming capability requests from a queue and executing the full pipeline for each.

## Context

The GIRT design document describes five LLM personas that collaborate through structured JSON artifacts. There are two ways to orchestrate this:

**Option A: Custom Rust orchestrator.** GIRT's MCP proxy process manages the pipeline directly — spawning LLM calls for each persona, routing artifacts between them, and managing the bug-ticket loop. The orchestrator is bespoke code.

**Option B: Claude agent team.** Each persona is a Claude Code agent (defined in the plugin). A team lead agent pulls requests from a queue, creates tasks, spawns teammates, and coordinates the pipeline using Claude Code's native team infrastructure (TaskCreate, TaskUpdate, SendMessage, etc.). The orchestration logic lives in agent system prompts and the plugin's skill definitions, not in custom Rust code.

**We choose Option B** as the primary orchestration mode, with Option A as a future optimization path for high-throughput scenarios.

## Rationale

1. **Leverage existing infrastructure.** Claude Code already provides agent teams, task lists, inter-agent messaging, and HITL via AskUserQuestion. Building a custom orchestrator duplicates this.

2. **Plugin ecosystem alignment.** The Epiphytic org already builds Claude Code plugins (hookwise, agent-fork-join). Making GIRT a plugin keeps the toolchain consistent and lets it compose with other plugins.

3. **Faster iteration.** Agent system prompts and skill definitions are easier to update than compiled Rust orchestration logic. The pipeline behavior can be tuned without recompilation.

4. **Natural HITL integration.** Claude Code's permission system and AskUserQuestion provide HITL gates natively. The Creation Gate's ASK outcome and the circuit breaker's escalation can use these directly instead of building custom HITL plumbing.

5. **Observability.** Claude Code's task list gives the user real-time visibility into pipeline state — which persona is active, what's queued, what's blocked, what passed/failed.

## Architecture

### Plugin Structure

```
girt-plugin/
├── plugin.json                    # Plugin manifest
├── .mcp.json                      # MCP server config (GIRT proxy with embedded runtime)
├── agents/
│   ├── pipeline-lead.md           # Team lead: queue consumer + orchestrator
│   ├── architect.md               # Spec refinement agent
│   ├── engineer.md                # Code generation agent
│   ├── qa.md                      # Functional testing agent
│   └── red-team.md                # Security auditing agent
├── skills/
│   ├── request-capability.md      # Operator-facing skill to submit requests
│   ├── list-tools.md              # Browse available tools in registries
│   └── promote-tool.md            # Promote tool from private → public registry
├── hooks/
│   ├── capability-intercept.md    # Intercepts request_capability calls
│   └── tool-call-gate.md          # Execution Gate hook on tool invocations
├── commands/
│   ├── girt-status.md             # /girt-status — pipeline + queue status
│   ├── girt-build.md              # /girt-build — manually trigger a build
│   └── girt-registry.md           # /girt-registry — manage registry config
└── CLAUDE.md                      # Plugin-level instructions
```

### Queue Mechanism

Capability requests enter the pipeline via a **file-based queue** stored locally:

```
~/.girt/queue/
├── pending/
│   ├── 1708444800-fetch-github-issues.json
│   └── 1708444860-parse-csv.json
├── in-progress/
│   └── 1708444750-http-client.json
├── completed/
│   └── 1708444600-json-transform.json
└── failed/
│   └── 1708444500-shell-exec.json
```

Each request file contains the Operator's capability spec plus metadata:

```json
{
  "id": "req_abc123",
  "timestamp": "2026-02-20T12:00:00Z",
  "source": "operator",
  "spec": {
    "name": "fetch_github_issues",
    "description": "...",
    "inputs": { ... },
    "outputs": { ... },
    "constraints": { ... }
  },
  "status": "pending",
  "priority": "normal",
  "attempts": 0
}
```

**Why file-based, not a database or message broker:**
- Zero external dependencies (no Redis, no SQLite)
- Atomic file operations via rename (move between directories)
- Human-inspectable (JSON files in a directory)
- Git-ignorable (not committed, but easy to debug)
- Sufficient for the expected throughput (tools are built in minutes, not milliseconds)

### Agent Team Workflow

```
┌────────────────────────────────────────────────────────────┐
│                  CLAUDE AGENT TEAM                          │
│                                                            │
│  ┌──────────────┐                                          │
│  │ Pipeline Lead │ ◄── polls ~/.girt/queue/pending/         │
│  │ (Team Lead)   │                                          │
│  └──────┬───────┘                                          │
│         │                                                  │
│         │ 1. Picks up request                              │
│         │ 2. Runs Creation Gate (hookwise decision)        │
│         │ 3. If ALLOW → creates task pipeline:             │
│         │                                                  │
│         ├──► TaskCreate: "Architect: refine spec"          │
│         │       │                                          │
│         │       ▼                                          │
│         │    ┌───────────┐                                 │
│         │    │ Architect │ Refines spec, outputs JSON      │
│         │    │ (Agent)   │ Updates task with refined spec   │
│         │    └─────┬─────┘                                 │
│         │          │                                       │
│         ├──► TaskCreate: "Engineer: implement spec"        │
│         │       │                                          │
│         │       ▼                                          │
│         │    ┌───────────┐                                 │
│         │    │ Engineer  │ Writes code, compiles .wasm     │
│         │    │ (Agent)   │ Loads into girt-runtime         │
│         │    └─────┬─────┘                                 │
│         │          │                                       │
│         ├──► TaskCreate: "QA: test component"              │
│         │    TaskCreate: "Red Team: audit component"       │
│         │       │              │                           │
│         │       ▼              ▼                           │
│         │    ┌─────────┐  ┌───────────┐                   │
│         │    │   QA    │  │ Red Team  │  (parallel)       │
│         │    │ (Agent) │  │ (Agent)   │                    │
│         │    └────┬────┘  └─────┬─────┘                   │
│         │         │             │                          │
│         │         ▼             ▼                          │
│         │    [Bug Tickets?] ──yes──► Loop to Engineer      │
│         │         │                   (max 3 iterations)   │
│         │         no                                       │
│         │         │                                        │
│         │         ▼                                        │
│         │    [Publish to OCI Registry]                     │
│         │    [Move request to completed/]                  │
│         │    [Notify Operator via MCP tools/list_changed]  │
│         │                                                  │
│         │ 4. If DENY → move to failed/, notify Operator    │
│         │ 5. If ASK  → AskUserQuestion, then proceed       │
│         │ 6. If DEFER → load existing tool, notify Operator│
│         │                                                  │
│         │ 7. Poll for next request...                      │
│         │                                                  │
└────────────────────────────────────────────────────────────┘
```

### Pipeline Lead Agent

The Pipeline Lead is the team's orchestrator. It:

1. **Polls the queue** — watches `~/.girt/queue/pending/` for new requests
2. **Runs the Creation Gate** — invokes the hookwise decision cascade
3. **Spawns teammates** — creates the Architect, Engineer, QA, and Red Team agents as needed
4. **Manages the bug-ticket loop** — if QA or Red Team fails, routes the ticket back to the Engineer and tracks iteration count
5. **Enforces the circuit breaker** — after 3 failed iterations, halts and escalates to the user
6. **Handles publishing** — on success, pushes the artifact to the configured OCI registry
7. **Notifies the Operator** — sends MCP notification that the tool is available

The Pipeline Lead does NOT participate in spec refinement, code generation, testing, or security auditing. It is purely an orchestrator.

### Concurrency Model

- **One request at a time** by default. The pipeline is sequential per-request because the Engineer, QA, and Red Team operate on the same artifact.
- **QA and Red Team run in parallel** against the compiled artifact. Both must pass.
- **Multiple Pipeline Leads** could run concurrently in the future (one per queue partition), but this is out of scope for v1.
- The queue prevents race conditions: a request file is atomically moved from `pending/` to `in-progress/` when claimed.

### Integration Points

**How a request enters the queue:**

1. **From the Operator agent** — via the `request_capability` tool, which writes the spec to `~/.girt/queue/pending/` and returns immediately. The Operator yields and waits for the MCP notification.

2. **From a CLI command** — `/girt-build <spec.json>` manually enqueues a build request. Useful for pre-building tools or testing the pipeline.

3. **From a hook** — the `capability-intercept` hook can detect when any agent attempts to use a tool that doesn't exist and auto-enqueue a capability request.

**How the Operator gets notified:**

The Pipeline Lead triggers `girt-runtime`'s `LifecycleManager::load_component()` to register the new `.wasm` artifact in-process, then the GIRT MCP proxy sends `tools/list_changed` to the Operator's MCP session. The Operator's tool list updates without restart.

## Consequences

### Positive

- Pipeline orchestration is defined in agent prompts and skills, not compiled code — faster to iterate
- Full visibility into pipeline state via Claude Code's task list UI
- HITL gates use Claude Code's native permission system
- Composes with other Epiphytic plugins (hookwise hooks, agent-fork-join patterns)
- Users can inspect, debug, and manually intervene in the pipeline via Claude Code

### Negative

- **Latency overhead.** Claude Code agent spawning adds latency vs. direct API calls from Rust. Each agent spawn includes context loading and model initialization.
- **Cost.** Each agent consumes a full Claude Code conversation turn. The 5-agent pipeline is more expensive through Claude Code than through direct API calls with minimal prompts.
- **Reliability dependency.** Pipeline availability depends on Claude Code being running and responsive. A standalone Rust orchestrator would be more resilient.
- **Concurrency limits.** Claude Code's team infrastructure has practical limits on concurrent agents. High-throughput scenarios may hit these.

### Migration Path to Option A

If throughput or cost becomes a concern, the pipeline can be migrated to a Rust orchestrator that makes direct API calls to Claude (via the Anthropic SDK) while preserving:
- The same agent system prompts (used as API system messages)
- The same queue format (file-based, same JSON schema)
- The same girt-runtime integration (direct `LifecycleManager` calls)
- The same hookwise decision engine (Rust library)

The plugin layer would then become a thin CLI/UI wrapper over the Rust orchestrator, rather than the orchestrator itself. The agent definitions serve as the specification regardless of which runtime executes them.

## Amendment — 2026-02-20: OAuth Credential Handling

When GIRT makes direct Anthropic API calls (both current `AnthropicLlmClient` and the future Rust orchestrator path described above), credentials are resolved from:

1. `ANTHROPIC_API_KEY` env var
2. OpenClaw `auth-profiles.json` (reads the token GIRT's host agent is already using)
3. GIRT's own OAuth token store (`~/.config/girt/auth.json`)
4. `api_key` in `girt.toml`

For case 3, GIRT uses the [`anthropic-auth`](https://docs.rs/anthropic-auth/latest) crate (v0.1, MIT) which implements the full Anthropic OAuth 2.0 PKCE flow (Max subscription or Console API-key-creation mode). This powers a `girt auth login` CLI command and provides automatic token refresh. Token storage is handled by GIRT via `AnthropicOAuthStore` in `girt-secrets`; `anthropic-auth` deliberately does not persist tokens itself.

---

## Open Questions

1. **Queue polling interval.** How frequently should the Pipeline Lead check for new requests? Too fast wastes API turns, too slow adds user-perceived latency. A filesystem watcher (inotify/kqueue) would be ideal but may not be available in all Claude Code environments.

2. **Agent persistence.** Should the Pipeline Lead be a long-running agent that persists across requests, or should it be spawned fresh per-request? Long-running preserves context (useful for caching) but risks context bloat. Fresh per-request is simpler but loses inter-request learning.

3. **Multi-pipeline teams.** Should different types of tools (security-sensitive vs. data-transform) have different pipeline configurations (e.g., skip Red Team for pure data transforms)? This would require pipeline profiles.

4. **Offline mode.** If the user is working without Claude Code (e.g., CI/CD), can the GIRT pipeline fall back to the Rust orchestrator (Option A) automatically?
