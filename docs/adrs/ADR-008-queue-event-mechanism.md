# ADR-008: Queue Polling vs. Filesystem Watching

**Status:** Accepted
**Date:** 2026-02-20
**Context:** How the Pipeline Lead detects new capability requests in the queue

---

## Decision

Use **filesystem watching** (inotify on Linux, kqueue on macOS) as the primary queue notification mechanism. Fall back to polling only on platforms where filesystem watching is unavailable.

### Implementation

```
~/.girt/queue/pending/          ◄── inotify/kqueue watches this directory
    │
    │ IN_CREATE / NOTE_WRITE event
    │
    ▼
Pipeline Lead wakes up
    │
    ▼
Process new request file
```

### Platform Support

| Platform | Mechanism | Crate |
|---|---|---|
| Linux | inotify | `notify` (Rust crate, cross-platform) |
| macOS | kqueue / FSEvents | `notify` |
| Windows | ReadDirectoryChangesW | `notify` |
| Fallback | Polling (5-second interval) | `notify` with `PollWatcher` |

The Rust `notify` crate provides a unified API across all platforms, making this straightforward.

### Future: External Event System

In future versions, the queue mechanism will be abstracted behind an `EventSource` trait to support:

- **Message queues** (NATS, Redis Pub/Sub) for distributed/multi-node setups
- **Webhooks** for remote GIRT service integration (ADR-005)
- **Claude Code hooks** for direct integration without filesystem intermediary

```rust
trait EventSource {
    async fn next_request(&mut self) -> Result<CapabilityRequest>;
}

// Implementations:
// - FsWatchEventSource (inotify/kqueue — current)
// - PollEventSource (fallback)
// - NatsEventSource (future)
// - WebhookEventSource (future)
```

## Context

The Pipeline Lead agent needs to detect when new capability requests arrive in `~/.girt/queue/pending/`. There are two approaches:

**Polling:** Check the directory on a timer (e.g., every 5 seconds). Simple but wastes CPU cycles when idle and adds latency when requests arrive.

**Filesystem watching:** The OS notifies the process when a file is created in the watched directory. Zero CPU when idle, near-instant detection of new requests.

## Rationale

- **inotify/kqueue is the right default.** Both mechanisms are mature, kernel-level, and essentially free in terms of resource consumption. They provide sub-millisecond notification latency.
- **The `notify` crate handles cross-platform differences.** We don't need to write platform-specific code.
- **Polling is the fallback, not the default.** Some environments (network filesystems, certain containers) don't support filesystem watching. The `notify` crate's `PollWatcher` handles this transparently.
- **The abstraction enables future flexibility.** When the remote build service (ADR-005) or distributed deployments need a different event mechanism, the `EventSource` trait makes the swap clean.

## Consequences

- The `notify` crate is added as a dependency.
- The Pipeline Lead's queue consumer uses `notify::RecommendedWatcher` by default.
- The `EventSource` trait is defined from the start, even though only `FsWatchEventSource` is implemented initially. This prevents a refactor when adding new backends.
- The fallback poll interval (5 seconds) is configurable in `girt.toml`.
