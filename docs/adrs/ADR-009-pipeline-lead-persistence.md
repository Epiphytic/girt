# ADR-009: Pipeline Lead Persistence Model

**Status:** Accepted
**Date:** 2026-02-20
**Context:** How the Pipeline Lead agent manages context across multiple capability requests

---

## Decision

The Pipeline Lead is a **long-running agent** that persists across requests. After completing each request, it:

1. **Records learnings** — Updates `MANIFEST.md` and `docs/` in the `girt-tools` repository with information about the newly built tool, build patterns that worked or failed, and any insights from the pipeline.
2. **Clears its context** — Compacts or resets its conversation context to prevent bloat, retaining only the persistent learnings written to files.

### Lifecycle Per Request

```
Pipeline Lead (long-running)
    │
    │ ◄── inotify: new request in queue
    │
    ▼
[Read request from queue]
    │
    ▼
[Run Creation Gate]
    │
    ▼
[Spawn Architect → Engineer → QA → Red Team]
    │
    ▼
[On success: publish artifact]
    │
    ▼
[Record learnings]
    ├── Update MANIFEST.md in girt-tools (new tool entry)
    ├── Update docs/ with build notes if notable
    ├── Log build success/failure patterns
    └── Record any new policy rules learned
    │
    ▼
[Clear context]
    ├── Teammates are shut down
    ├── Task list is cleaned up
    └── Pipeline Lead's context is compacted
    │
    ▼
[Wait for next request...]
```

### What Gets Recorded

After each request, the Pipeline Lead writes:

**To `girt-tools/MANIFEST.md`:**
- Tool name, version, description
- Source language
- Build date and iteration count
- Registry location

**To `girt-tools/docs/build-log.md`** (append-only):
- Request summary (what was asked for)
- Build outcome (success, failure, circuit breaker)
- Notable patterns (e.g., "Rust borrow checker issues with async HTTP — switched to Go")
- Time taken

**To `~/.girt/hookwise-rules.jsonl`** (local):
- New Creation Gate decisions to cache
- New policy rules discovered during the build (e.g., "tools requesting access to 169.254.169.254 should be auto-denied")

### Context Clearing Strategy

The Pipeline Lead uses Claude Code's context compaction after each request:

1. All spawned teammates (Architect, Engineer, QA, Red Team) are shut down via `shutdown_request`
2. All tasks in the task list are verified as completed or cleaned up
3. The Pipeline Lead's conversation history is compacted, preserving only:
   - The system prompt and agent definition
   - References to persistent files (MANIFEST.md, build-log.md, hookwise-rules.jsonl)
4. The agent continues running, waiting for the next filesystem event

This gives the Pipeline Lead a "clean desk" for each new request while maintaining institutional knowledge in files.

### Why Not Fresh-Per-Request

A fresh agent per request would lose:
- **Warm filesystem watcher** — no re-initialization delay between requests
- **Loaded registry state** — the Pipeline Lead maintains an in-memory view of available tools
- **Session continuity** — if multiple requests arrive quickly, a persistent agent can batch or prioritize them

### Why Not Fully Persistent Context

Keeping the full conversation history across requests would:
- **Bloat context rapidly** — each build pipeline generates substantial back-and-forth
- **Degrade response quality** — irrelevant prior context pollutes the agent's reasoning
- **Risk context limit** — after 5-10 builds, the context window fills up

The hybrid approach (persistent agent, cleared context, file-based learnings) gets the benefits of both.

## Context

The Pipeline Lead is the orchestrator for the entire GIRT build pipeline. It needs to balance responsiveness (staying ready for new requests) with context hygiene (not accumulating irrelevant history from prior builds).

## Rationale

- **Files are the memory, not the context window.** MANIFEST.md and build-log.md are durable, inspectable, and available to all agents — not just the Pipeline Lead.
- **Context compaction is a natural checkpoint.** It aligns with Claude Code's existing context management. The Pipeline Lead doesn't fight the system; it works with it.
- **Learnings compound over time.** Each build improves the Pipeline Lead's decision-making via cached rules and documented patterns, without requiring the full history in context.

## Consequences

- The Pipeline Lead's agent definition must include instructions to record learnings and clear context after each request.
- `girt-tools/MANIFEST.md` and `girt-tools/docs/build-log.md` are updated by the Pipeline Lead and must be committed (manually or via automated PR).
- The `~/.girt/hookwise-rules.jsonl` file grows over time as new decisions are cached. A periodic cleanup or deduplication process may be needed.
- The Pipeline Lead should handle interrupted builds gracefully — if it's shut down mid-build, it should be able to resume or clean up on restart by checking the `in-progress/` queue directory.
