# G.I.R.T. Technical Design Document (Refined)
## Generative Isolated Runtime for Tools

**Status:** Draft
**Date:** 2026-02-20
**Origin:** Gemini-generated design, refined with architectural review

---

## 1. System Philosophy

GIRT is a **multi-agent tool factory** that dynamically generates, tests, and publishes sandboxed WebAssembly tools on demand. It applies the Hookwise tri-state decision pipeline to both **tool creation** and **tool execution**, ensuring that every capability an LLM agent acquires is policy-checked, functionally verified, and security-audited before use.

GIRT does **not** build its own runtime or sandbox. It sits as an **MCP proxy** in front of [Wassette](https://github.com/microsoft/wassette) (Microsoft's security-oriented WASM runtime for MCP), delegating all sandboxed execution to Wassette's Wasmtime engine and policy system. GIRT owns the build pipeline and decision logic; Wassette owns the execution boundary.

### Core Principles

1. **Build, don't bundle.** The Operator agent has no static tools. When it needs a capability, it requests one. The pipeline either finds an existing match or builds it.
2. **Decide like Hookwise.** Every tool creation and execution passes through a cached decision cascade with tri-state outcomes (Allow, Deny, Ask) and HITL escalation.
3. **Design for reuse.** The Architect refines every capability request into a generic, composable tool spec before any code is written. Tools are built for the ecosystem, not just the immediate task.
4. **Defense in depth.** Wassette provides runtime sandboxing. GIRT provides pre-deployment assurance (functional testing + adversarial security audit). Both layers are active simultaneously.
5. **Publish and reuse.** Passing tools are pushed to OCI registries. The Epiphytic org maintains a curated public registry and a private internal registry. Users can configure additional registries.

---

## 2. Architecture Overview

```
                          USER / AI AGENT
                                │
                                ▼
┌─────────────────────────────────────────────────────────────────────┐
│                     GIRT MCP PROXY (Rust)                           │
│                                                                     │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │              HOOKWISE DECISION ENGINE                         │   │
│  │                                                               │   │
│  │  ┌─────────┐  ┌──────────┐  ┌─────────┐  ┌──────┐          │   │
│  │  │ Policy  │→│  Cache   │→│Similarity│→│  LLM │→ [HITL]   │   │
│  │  │ Rules   │  │(Decision)│  │(Embedding)│ │Eval  │          │   │
│  │  └────┬────┘  └────┬─────┘  └────┬─────┘  └──┬───┘          │   │
│  │       │ allow/deny  │ hit/miss    │ match     │ allow/deny/ask│  │
│  │       └─────────────┴─────────────┴───────────┘               │   │
│  └──────────────────────────────────────────────────────────────┘   │
│                                                                     │
│  ┌─ TOOL CREATION PATH ─────────────────────────────────────────┐   │
│  │                                                               │   │
│  │  [Capability Request]                                         │   │
│  │        │                                                      │   │
│  │        ▼                                                      │   │
│  │  ┌───────────┐   DECISION: Should this tool exist?            │   │
│  │  │ Creation  │──► AUTO-DENY (dangerous pattern)               │   │
│  │  │ Gate      │──► DEFER (existing tool / CLI utility)         │   │
│  │  │           │──► AUTO-ALLOW (benign, novel)                  │   │
│  │  │           │──► ASK (ambiguous → HITL)                      │   │
│  │  └─────┬─────┘                                                │   │
│  │        │ (allowed)                                            │   │
│  │        ▼                                                      │   │
│  │  ┌───────────┐                                                │   │
│  │  │ Architect │ Refines narrow request into robust, generic    │   │
│  │  │ (LLM)    │ reusable tool spec. Considers composability,   │   │
│  │  │           │ existing tool ecosystem, API design.           │   │
│  │  └─────┬─────┘                                                │   │
│  │        │ (refined spec)                                       │   │
│  │        ▼                                                      │   │
│  │  ┌───────────┐                                                │   │
│  │  │ Engineer  │ Generates Rust code + WIT interface            │   │
│  │  │ (LLM)    │ Compiles to wasm32-wasi Component               │   │
│  │  └─────┬─────┘                                                │   │
│  │        │                                                      │   │
│  │        ▼                                                      │   │
│  │  ┌───────────┐                                                │   │
│  │  │ QA Agent  │ Functional test suite against Wassette sandbox │   │
│  │  │ (LLM)    │ Outputs Bug Ticket on failure → loops to Eng.   │   │
│  │  └─────┬─────┘                                                │   │
│  │        │                                                      │   │
│  │        ▼                                                      │   │
│  │  ┌───────────┐                                                │   │
│  │  │ Red Team  │ Adversarial exploitation in Wassette sandbox   │   │
│  │  │ (LLM)    │ Outputs Bug Ticket on vuln found → loops to Eng│   │
│  │  └─────┬─────┘                                                │   │
│  │        │ (all tests pass, max 3 loops)                        │   │
│  │        ▼                                                      │   │
│  │  [Publish to OCI Registry] + [Generate policy.yaml]           │   │
│  │  [Cache locally for immediate use]                            │   │
│  └───────────────────────────────────────────────────────────────┘   │
│                                                                     │
│  ┌─ TOOL EXECUTION PATH ────────────────────────────────────────┐   │
│  │                                                               │   │
│  │  [Tool Call from Operator]                                    │   │
│  │        │                                                      │   │
│  │        ▼                                                      │   │
│  │  ┌───────────┐   DECISION: Should this invocation proceed?    │   │
│  │  │ Execution │──► Policy rules (Wassette YAML)                │   │
│  │  │ Gate      │──► Cached prior decisions                      │   │
│  │  │           │──► LLM evaluation (novel context)              │   │
│  │  │           │──► HITL (sensitive operations)                 │   │
│  │  └─────┬─────┘                                                │   │
│  │        │ (allowed)                                            │   │
│  │        ▼                                                      │   │
│  │  [Proxy to Wassette MCP Server]                               │   │
│  │        │                                                      │   │
│  │        ▼                                                      │   │
│  │  [Return result to Operator]                                  │   │
│  └───────────────────────────────────────────────────────────────┘   │
│                                                                     │
│  ┌─ SECRET WRAPPER ─────────────────────────────────────────────┐   │
│  │  host_auth_proxy(service_name) → Vault/env lookup             │   │
│  │  Wraps Wassette's env-var permissions with zero-knowledge     │   │
│  │  proxy. Secrets never enter WASM memory.                      │   │
│  └───────────────────────────────────────────────────────────────┘   │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
                                │
                                ▼
┌─────────────────────────────────────────────────────────────────────┐
│                    WASSETTE MCP SERVER                               │
│                                                                     │
│  ┌─────────────┐  ┌──────────────────┐  ┌───────────────────────┐  │
│  │ Component   │  │ Policy Engine    │  │ Wasmtime Sandbox      │  │
│  │ Registry    │  │ (YAML per-tool)  │  │ (WASI, deny-default)  │  │
│  └─────────────┘  └──────────────────┘  └───────────────────────┘  │
│  ┌─────────────┐  ┌──────────────────┐                             │
│  │ OCI Loader  │  │ WIT Introspection│                             │
│  └─────────────┘  └──────────────────┘                             │
└─────────────────────────────────────────────────────────────────────┘
```

---

## 3. Hookwise Decision Engine

The same tri-state decision cascade from Hookwise is applied to two distinct gates in GIRT. Each gate evaluates a request through progressively more expensive layers, short-circuiting as soon as a confident decision is reached.

### 3.1 Creation Gate — "Should this tool be built?"

| Layer | Input | Logic | Outcome |
|---|---|---|---|
| **Policy Rules** | Capability spec | Pattern-match against known-dangerous patterns (filesystem root access, shell exec, credential extraction, etc.) and known-safe patterns (math, string ops, read-only verified APIs) | AUTO-DENY / AUTO-ALLOW / pass-through |
| **Registry Lookup** | Capability spec | Search configured OCI registries (Epiphytic public, private, user-defined) for an existing tool matching the spec | DEFER (use existing) / pass-through |
| **CLI/Native Check** | Capability spec | Check if a well-known CLI utility (jq, curl, ripgrep, etc.) already handles this better than a WASM tool | DEFER (suggest native) / pass-through |
| **Similarity Check** | Capability spec embedding | Compare against embeddings of previously built tool specs. Near-match → suggest extending existing tool rather than building new | DEFER (extend existing) / pass-through |
| **LLM Evaluation** | Full context | LLM evaluates whether the tool should be built, considering the Operator's role, the user's intent, and organizational policy | ALLOW / DENY / ASK |
| **HITL** | LLM rationale + spec | User makes final decision | ALLOW / DENY |

**DEFER** is a fourth outcome unique to the Creation Gate. It does not reject the request — it redirects it to an existing capability, either:
- A tool already in a registry (loaded into Wassette directly)
- A CLI utility the Operator should invoke natively
- An existing tool that should be extended with the requested feature

**Cache behavior:** Decisions are cached by spec hash. A previously-denied spec with the same hash is auto-denied. A previously-allowed spec skips to the build pipeline. DEFER decisions are cached with a pointer to the deferred-to tool.

### 3.2 Execution Gate — "Should this invocation proceed?"

| Layer | Input | Logic | Outcome |
|---|---|---|---|
| **Wassette Policy** | Tool ID + params | Wassette's per-component YAML policy (network hosts, storage paths, env vars) | DENY (policy violation) / pass-through |
| **Cached Decisions** | Tool ID + param signature | Lookup prior allow/deny for same tool with similar parameters | ALLOW / DENY / pass-through |
| **LLM Evaluation** | Full invocation context | LLM evaluates whether this specific invocation is appropriate given the Operator's current task and role | ALLOW / DENY / ASK |
| **HITL** | LLM rationale + invocation | User makes final decision | ALLOW / DENY |

---

## 4. Multi-Agent Build Pipeline

Five LLM personas participate in the build pipeline. They communicate strictly through structured JSON artifacts (capability specs, source code, bug tickets), not conversational messages.

### 4.1 The Operator (Primary Agent)

The user-facing reasoning engine. It has no static tools except `request_capability`.

**Behavior:**
1. Receives user task
2. Determines what capabilities are needed
3. Calls `request_capability` with a JSON spec for each
4. Yields until GIRT notifies it that the tool is available (hot-reload via MCP)
5. Proceeds with the task using the newly available tools

**System Prompt:**
```
You are the primary Operator. You solve complex problems for the user.

You operate in a capability-on-demand environment. If you need a tool that is
not in your current registry, call `request_capability` with a JSON specification:

{
  "name": "descriptive_tool_name",
  "description": "What this tool does and why you need it",
  "inputs": { "param_name": { "type": "string", "description": "..." } },
  "outputs": { "type": "object", "description": "Expected return shape" },
  "constraints": {
    "network": ["api.example.com"],
    "storage": [],
    "secrets": ["EXAMPLE_API_KEY"]
  }
}

Do not apologize for missing tools. Do not write workaround scripts.
Request the capability and yield. You will be notified when it is available.
```

### 4.2 The Architect (Spec Refinement)

Sits between the Creation Gate and the Engineer. Its job is **not** policy enforcement (that's the Creation Gate) — it's **design quality**. The Architect takes the Operator's narrow, task-specific capability request and refines it into a robust, generic, reusable tool specification.

**Why this exists:** LLM Operators request tools for their immediate problem. "Fetch my open GitHub issues" becomes a single-purpose fetcher. The Architect thinks bigger: this should be a GitHub Issues tool with filtering, pagination, state queries, label support, and repo selection — a tool worth publishing to the registry for others to reuse.

**Responsibilities:**
1. **Generalization** — Broaden the spec from single-use to reusable. Add parameters the Operator didn't think to request but that make the tool generally useful.
2. **Composability** — Design the tool's interface so it composes well with other tools. Prefer small, focused tools over monoliths. If the request spans multiple concerns, split it into composable components.
3. **Ecosystem awareness** — Review existing tools in configured registries. If a similar tool exists, recommend extending it rather than building a parallel one. If the request overlaps with a standard library tool, recommend composition.
4. **API design** — Ensure inputs/outputs follow consistent conventions (naming, error shapes, pagination patterns) so tools built by GIRT feel like a coherent toolkit, not a random collection.
5. **Constraint refinement** — Tighten the security constraints. If the Operator requested broad network access, narrow it to the specific hosts actually needed. Minimize the permission surface.

**System Prompt:**
```
You are a Chief Software Architect specializing in tool design for sandboxed
WebAssembly environments. You do not write implementation code.

You receive a capability request from an Operator agent. Your job is to refine
it into a robust, generic, reusable tool specification that is worth publishing
to a shared registry.

Design Principles:
1. GENERALIZE: The Operator asks for what they need right now. You design for
   what anyone might need. Add parameters, filters, and options that make the
   tool broadly useful — but do not over-engineer. Every parameter must have
   a clear use case.

2. COMPOSE: Prefer small, focused tools over monoliths. If the request spans
   multiple concerns (e.g., "fetch issues and format as markdown"), split into
   two composable tools. Tools should do one thing well.

3. ECOSYSTEM FIT: You are provided with the current tool registry contents.
   If a similar tool exists, output a RECOMMEND_EXTEND directive with the
   existing tool ID and the features to add. Do not create duplicates.

4. CONSISTENT API: Follow these conventions for all tool interfaces:
   - Use snake_case for parameter and field names
   - Paginated endpoints return { items: [], next_cursor: string | null }
   - Errors return { error: string, code: string, details: object | null }
   - Optional parameters have sensible defaults documented in the spec

5. MINIMAL PERMISSIONS: Review the Operator's requested constraints and
   tighten them. If they asked for network access to "*.github.com", narrow
   to "api.github.com". If they requested storage but the tool can be
   stateless, remove it.

Output Format:
{
  "action": "BUILD" | "RECOMMEND_EXTEND",
  "spec": {
    "name": "tool_name",
    "version": "0.1.0",
    "description": "What this tool does (generic, not task-specific)",
    "inputs": { ... },
    "outputs": { ... },
    "constraints": {
      "network": ["specific.host.com"],
      "storage": [],
      "secrets": ["SERVICE_API_KEY"]
    },
    "design_notes": "Brief rationale for key design decisions"
  },
  "extend_target": "existing_tool_id (only if action is RECOMMEND_EXTEND)",
  "extend_features": ["feature to add (only if RECOMMEND_EXTEND)"]
}

Do not include conversational filler. Output only the JSON specification.
```

### 4.3 The Engineer (Builder)

Generates WASM Component source code from the Architect's refined spec.

**Language:** Rust is the default and preferred target. The Engineer MUST support other languages (Go, AssemblyScript, C) when the user or configuration specifies, or when a capability is better suited to another language.

**Input:** The Architect's refined JSON spec (not the Operator's raw request). The Engineer implements exactly what the Architect specified.

**System Prompt:**
```
You are a Senior Backend Engineer. You write functions that compile to
wasm32-wasi Components and execute inside a Wasmtime sandbox via Wassette.

Target: WebAssembly Component Model with WIT interface definitions.

Environment Constraints:
- No local filesystem access unless explicitly granted in the spec.
- No native network access. Use WASI HTTP for outbound calls.
- Network access is restricted to hosts listed in the spec's constraints.
- SECRETS: Never hardcode credentials. Call host_auth_proxy(service_name)
  to get authenticated responses. The host handles credential injection.

Output Format:
1. A WIT interface file (.wit) defining the component's world
2. Source code (Rust by default) implementing the interface
3. A policy.yaml file declaring required permissions

Do not include markdown, explanations, or conversational filler.
If you receive a Bug Ticket, output only the patched code addressing
the specific remediation_directive.
```

### 4.4 The QA Agent (Functional Tester)

Verifies that the Engineer's compiled WASM Component behaves according to spec.

**Execution environment:** Tests run against Wassette's sandbox via `call_tool`. The QA agent does not have its own sandbox — it uses the same runtime the tool will run in production.

**System Prompt:**
```
You are a QA Automation Engineer. You are given:
1. The Architect's refined tool specification (expected behavior)
2. The Engineer's compiled WASM Component (loaded in Wassette)

Your objective is to verify functional correctness:

1. Generate 5+ JSON input payloads covering:
   - Standard use cases (happy path)
   - Edge cases (empty inputs, boundary values, unicode)
   - Malformed inputs (wrong types, missing fields, oversized payloads)
2. Execute each payload against the component via Wassette's call_tool.
3. Compare outputs against the specification's expected behavior.

If ANY output does not match expected behavior, output a Bug Ticket:
{
  "target": "engineer",
  "type": "functional_defect",
  "input": <the failing input>,
  "expected": <what the spec says should happen>,
  "actual": <what actually happened>,
  "remediation_directive": <specific fix instruction>
}

Do NOT attempt to fix the code yourself. Do NOT pass a component that
produces incorrect results.
```

### 4.5 The Red Team Agent (Security Auditor)

Actively attempts to exploit the component within Wassette's sandbox.

**Execution environment:** Same Wassette sandbox as production, with instrumentation to detect policy violations and anomalous behavior.

**System Prompt:**
```
You are an Offensive Security Researcher. You are given:
1. The source code of a newly generated WASM Component
2. Its policy.yaml (declared permissions)
3. Access to execute payloads against it in Wassette's sandbox

Your Mission: Attempt to force the component to act outside its declared
capabilities.

Attack vectors to attempt:
- SSRF: Trick URL-handling logic into hitting disallowed hosts (cloud
  metadata IPs, localhost, internal DNS)
- Path traversal: ../../../etc/shadow or equivalent
- Prompt injection: If the tool processes text that may contain
  instructions, attempt to subvert its behavior
- Permission escalation: Attempt to access storage/network/env beyond
  what policy.yaml declares
- Resource exhaustion: Inputs designed to cause unbounded memory or
  CPU consumption
- Data exfiltration: Attempt to leak input data through allowed
  network channels (e.g., DNS, URL params to allowed hosts)

Execute your attack payloads via Wassette's call_tool.

If ANY payload successfully bypasses declared policy, output a Bug Ticket:
{
  "target": "engineer",
  "type": "security_vulnerability",
  "cwe": "<CWE ID if applicable>",
  "payload": <the exploit input>,
  "observed_behavior": <what happened>,
  "expected_containment": <what should have been blocked>,
  "remediation_directive": <specific fix instruction>
}
```

### 4.6 Build Loop Circuit Breaker

The build pipeline loops (Engineer → QA/Red Team → Bug Ticket → Engineer) a maximum of **3 iterations**. If the component does not pass both QA and Red Team after 3 attempts:

1. The pipeline **halts**
2. A diagnostic summary is generated containing all 3 attempts and their failures
3. The summary is escalated to the user via HITL
4. The user can: manually fix, adjust the spec, or abandon the request

This prevents infinite LLM loops and aligns with the global circuit breaker convention.

---

## 5. Tool Registry & Distribution

### 5.1 Registry Tiers

```
┌─────────────────────────────────────────────────┐
│           REGISTRY CONFIGURATION                 │
│                                                  │
│  Tier 1: Epiphytic Public Registry (default)     │
│  ├── oci://ghcr.io/epiphytic/girt-tools-public   │
│  ├── Curated, vetted tools                       │
│  ├── Readable by anyone                          │
│  └── Writable by Epiphytic org + delegates       │
│                                                  │
│  Tier 2: Epiphytic Private Registry              │
│  ├── oci://ghcr.io/epiphytic/girt-tools-private  │
│  ├── Internal pipeline output                    │
│  ├── Readable by org members                     │
│  └── Promoted to public after vetting            │
│                                                  │
│  Tier N: User-Defined Registries                 │
│  ├── Configured in girt.toml or similar          │
│  ├── Any OCI-compatible registry                 │
│  └── User controls trust and access              │
└─────────────────────────────────────────────────┘
```

### 5.2 Tool Lifecycle

```
[Engineer builds] → [QA passes] → [Red Team passes]
        │
        ▼
[Publish to Epiphytic Private Registry]
        │
        ▼ (manual vetting / promotion pipeline)
[Promote to Epiphytic Public Registry]
        │
        ▼
[Available to all GIRT users as default tool]
```

### 5.3 Local Cache

Tools fetched from any registry are cached locally for speed. The local cache is checked before any registry lookup. Cache invalidation follows OCI tag semantics — pinned versions are immutable, `latest` tags re-resolve on a configurable TTL.

### 5.4 Standard Library

The Epiphytic Public Registry ships pre-populated with common tools:
- HTTP client (GET/POST/PUT/DELETE to arbitrary allowed hosts)
- JSON parsing and transformation
- File I/O (read/write within granted paths)
- Common API integrations (GitHub, GitLab, etc.)
- Text processing (regex, templating)
- Cryptographic utilities (hashing, HMAC — no key generation)

These tools bypass the build pipeline entirely. They are maintained as versioned OCI artifacts in the public registry, with the same policy.yaml enforcement as any dynamically-built tool.

---

## 6. Secret Handling: Zero-Knowledge Wrapper

GIRT wraps Wassette's environment-variable permissions with a zero-knowledge proxy layer:

```
WASM Component                    GIRT Host                     Secret Store
     │                               │                              │
     │ host_auth_proxy("github")     │                              │
     │ ─────────────────────────────►│                              │
     │                               │  lookup("github")            │
     │                               │ ─────────────────────────────►│
     │                               │  ◄── GITHUB_TOKEN            │
     │                               │                              │
     │                               │  [Executes authenticated     │
     │                               │   HTTP request on behalf     │
     │                               │   of component]              │
     │                               │                              │
     │  ◄── sanitized JSON response  │                              │
     │  (no token in WASM memory)    │                              │
```

The component never receives the raw credential. It receives only the authenticated response body. This is implemented as a Wasmtime host function that GIRT injects when instantiating components through Wassette.

**Secret stores supported** (facade pattern — swappable):
- Environment variables (`.env.local`, default)
- OS keychain (macOS Keychain, Linux Secret Service)
- External vault (HashiCorp Vault, cloud KMS — future)

---

## 7. MCP Integration

### 7.1 GIRT as MCP Proxy

```
AI Agent ──MCP──► GIRT Proxy ──MCP──► Wassette Server
                     │
                     ├── Intercepts tool_call requests
                     ├── Runs Execution Gate decision cascade
                     ├── Proxies allowed calls to Wassette
                     ├── Intercepts request_capability calls
                     ├── Runs Creation Gate + build pipeline
                     └── Hot-reloads new tools into Wassette
```

### 7.2 Transport

GIRT exposes an MCP server via **stdio** (for local Claude Code integration) or **SSE** (for networked setups). It connects to Wassette as an MCP client.

### 7.3 Hot Reload

When a new tool passes the build pipeline:
1. GIRT calls Wassette's `load-component` with the compiled `.wasm` artifact
2. Wassette registers the component and generates JSON Schema from its WIT interface
3. GIRT sends an MCP `tools/list_changed` notification to the Operator
4. The Operator's tool registry updates without restarting

---

## 8. Pipeline Orchestration: Claude Agent Team

The build pipeline is orchestrated as a **Claude Code agent team** via a custom GIRT plugin. See **[ADR-007](../adrs/ADR-007-claude-agent-team-orchestration.md)** for the full decision record.

**Summary:** Each pipeline persona (Architect, Engineer, QA, Red Team) is a Claude Code agent. A Pipeline Lead agent polls a file-based queue (`~/.girt/queue/`), runs the Creation Gate, spawns teammates for each pipeline stage, manages the bug-ticket loop, and publishes passing tools. QA and Red Team run in parallel.

**Key properties:**
- Queue-driven: requests enter via `request_capability` tool, CLI command, or hook interception
- File-based queue with atomic operations (no external dependencies)
- Full pipeline visibility through Claude Code's task list UI
- HITL gates use Claude Code's native AskUserQuestion
- Migration path to standalone Rust orchestrator for high-throughput scenarios

**Plugin structure:**
```
girt-plugin/
├── plugin.json
├── .mcp.json                  # Wassette + GIRT proxy servers
├── agents/
│   ├── pipeline-lead.md       # Queue consumer + orchestrator
│   ├── architect.md           # Spec refinement
│   ├── engineer.md            # Code generation + compilation
│   ├── qa.md                  # Functional testing
│   └── red-team.md            # Security auditing
├── skills/
│   ├── request-capability.md  # Submit capability requests
│   ├── list-tools.md          # Browse registries
│   └── promote-tool.md        # Private → public promotion
├── hooks/
│   ├── capability-intercept.md
│   └── tool-call-gate.md
└── commands/
    ├── girt-status.md         # /girt-status
    ├── girt-build.md          # /girt-build
    └── girt-registry.md       # /girt-registry
```

---

## 9. Integration with Epiphytic Ecosystem

| Epiphytic Project | Relationship to GIRT |
|---|---|
| **Hookwise** (fka captain-hook) | GIRT embeds the Hookwise decision engine for both Creation and Execution gates. Hookwise's policy rules, cache, similarity matching, LLM eval, and HITL cascade are the core decision infrastructure. |
| **Wassette** (Microsoft, adopted) | GIRT's execution runtime. All WASM sandboxing, policy enforcement, and MCP tool serving is delegated to Wassette. GIRT manages the build pipeline and proxies execution. |
| **Claude Code Plugin** | The GIRT pipeline runs as a Claude Code plugin with an agent team. See [ADR-007](../adrs/ADR-007-claude-agent-team-orchestration.md). The plugin provides agents, skills, hooks, and commands for the full lifecycle. |
| **agent-fork-join** | Patterns from agent-fork-join inform the Pipeline Lead's orchestration of QA + Red Team parallel execution and the bug-ticket loop. |
| **duratii** | Future: GIRT could run as a cloud service on Cloudflare Workers via duratii, serving tool registries and build pipelines remotely. |

---

## 10. Configuration

```toml
# girt.toml

[operator]
role = "general"  # Role definition constraining what tools can be requested

[registries]
# Searched in order. First match wins.
default = [
  "oci://ghcr.io/epiphytic/girt-tools-public",
]

[registries.private]
url = "oci://ghcr.io/epiphytic/girt-tools-private"
auth = "github"  # Uses GitHub token from secret store

[registries.custom]
# Users can add their own
urls = []

[build]
default_language = "rust"
max_build_iterations = 3
publish_on_success = true          # Push passing tools to registry
publish_target = "private"         # "private" or "public"

[cache]
local_path = "~/.girt/cache"
ttl_latest = "1h"                  # Re-resolve :latest tags after 1h
ttl_pinned = "forever"             # Pinned versions never expire

[secrets]
backend = "env"                    # "env", "keychain", or "vault"
# vault_addr = "https://vault.example.com"

[hookwise]
# Path to hookwise rules/config
rules_path = "~/.girt/hookwise-rules.jsonl"
embedding_model = "local"          # or "api" for cloud embeddings
llm_model = "claude-sonnet-4-6"    # LLM for decision evaluation
```

---

## 11. Tool Artifact Format

Every tool published to an OCI registry is a self-contained artifact bundle:

```
<tool_name>:<version>
├── component.wasm          # Compiled WASM Component (wasm32-wasi)
├── component.wit           # WIT interface definition
├── policy.yaml             # Wassette permission declaration
├── spec.json               # Architect's refined capability spec
└── manifest.json           # GIRT metadata
```

### manifest.json

```json
{
  "girt_version": "0.1.0",
  "name": "github_issues",
  "version": "0.3.0",
  "description": "Query and manage GitHub issues with filtering and pagination",
  "source_language": "rust",
  "built_by": "girt-pipeline",
  "built_at": "2026-02-20T14:30:00Z",
  "build_iterations": 1,
  "checksum": {
    "wasm_sha256": "abc123...",
    "wit_sha256": "def456..."
  },
  "dependencies": [],
  "tags": ["github", "issues", "api"]
}
```

This metadata enables:
- **Registry search** — tags and description power the Creation Gate's registry lookup
- **Provenance** — `built_by`, `built_at`, and `build_iterations` track how the tool was created
- **Integrity** — SHA-256 checksums verify artifacts weren't tampered with
- **Ecosystem queries** — the Architect uses `spec.json` to evaluate composability and duplication

---

## 12. Error Handling

Each failure point in the pipeline has a defined recovery path:

| Failure Point | Behavior | Recovery |
|---|---|---|
| **Creation Gate — policy rules crash** | Fail open to next layer (cache). Log error at ERROR level. | If all layers fail, escalate to HITL. Never silently allow. |
| **Creation Gate — registry unreachable** | Skip registry lookup layer. Log at WARN. Continue to next layer. | Tool may be built even if a duplicate exists. Acceptable tradeoff vs. blocking. |
| **Architect — LLM call fails** | Retry once. If second failure, pass the Operator's raw spec directly to the Engineer unrefined. Log at WARN. | Unrefined spec may produce a less reusable tool, but the pipeline isn't blocked. |
| **Engineer — compilation fails** | Counts as a build iteration. Bug ticket with compiler errors routes back to Engineer. | Circuit breaker after 3 iterations. |
| **Engineer — LLM call fails** | Retry once. If second failure, halt pipeline and escalate to user. | User can retry or provide code manually. |
| **QA — test execution fails (Wassette error)** | Distinguish from functional failure. Wassette errors are infrastructure, not code bugs. Retry once. | If Wassette is persistently down, halt pipeline with infra error message. |
| **Red Team — no exploits found** | This is the success path. Component passes security audit. | N/A |
| **Publishing — OCI registry unreachable** | Cache locally. Queue for retry. Tool is still usable from local cache. | Background retry with exponential backoff. Notify user if retry exhausted. |
| **Execution Gate — Wassette policy violation** | DENY. Return structured error to Operator with the specific policy rule violated. | Operator can request a new tool with broader permissions (goes through Creation Gate again). |
| **Secret store — lookup fails** | Return structured error to component. Do NOT fall back to plaintext env vars. | User must fix secret store configuration. |

### Structured Error Format

All errors surfaced to the Operator follow a consistent shape:

```json
{
  "error": "capability_build_failed",
  "code": "GIRT_BUILD_CIRCUIT_BREAKER",
  "details": {
    "tool_name": "github_issues",
    "attempts": 3,
    "last_failure": "functional_defect",
    "summary": "Pagination cursor handling fails on empty result sets"
  }
}
```

---

## 13. Observability

Per the global convention, GIRT uses structured JSON logging at all levels.

### Log Events

| Event | Level | Fields |
|---|---|---|
| Capability request received | INFO | `request_id`, `tool_name`, `source` |
| Creation Gate decision | INFO | `request_id`, `decision` (allow/deny/defer/ask), `layer` (which layer decided), `rationale` |
| Architect spec produced | INFO | `request_id`, `action` (build/extend), `spec_name`, `spec_version` |
| Engineer build started | INFO | `request_id`, `iteration`, `language` |
| Compilation result | INFO/ERROR | `request_id`, `iteration`, `success`, `error_summary` |
| QA test suite result | INFO | `request_id`, `iteration`, `tests_run`, `tests_passed`, `tests_failed` |
| Red Team audit result | INFO | `request_id`, `iteration`, `exploits_attempted`, `exploits_succeeded` |
| Bug ticket created | WARN | `request_id`, `iteration`, `ticket_type`, `summary` |
| Circuit breaker triggered | ERROR | `request_id`, `total_iterations`, `failure_summary` |
| Tool published | INFO | `request_id`, `registry`, `tool_name`, `version`, `checksum` |
| Execution Gate decision | INFO | `tool_id`, `decision`, `layer` |
| Tool invocation | DEBUG | `tool_id`, `params_hash`, `duration_ms`, `result_size` |
| Secret proxy call | INFO | `service_name`, `success` (no credential content logged) |

### Metrics (Future)

When a metrics backend is configured:
- `girt.build.duration_ms` — histogram of end-to-end build times
- `girt.build.iterations` — histogram of iterations per build
- `girt.build.success_rate` — counter of pass/fail builds
- `girt.gate.decisions` — counter by gate (creation/execution) and outcome (allow/deny/defer/ask)
- `girt.cache.hit_rate` — ratio of cache hits to total lookups
- `girt.execution.duration_ms` — histogram of tool invocation times

---

## 14. Resource Limits

WASM execution through Wassette must be bounded to prevent resource exhaustion:

| Resource | Default Limit | Configurable |
|---|---|---|
| **Memory** | 256 MB per component instance | Yes, in policy.yaml |
| **CPU (Wasmtime fuel)** | 1,000,000,000 fuel units (~10s equivalent) | Yes, in policy.yaml |
| **Execution timeout** | 30 seconds wall-clock | Yes, in girt.toml |
| **Network response size** | 10 MB per HTTP response | Yes, in policy.yaml |
| **Concurrent instances** | 1 per component (no parallel invocations of same tool) | Yes, in girt.toml |

These limits are enforced by Wassette's Wasmtime configuration. GIRT generates the appropriate settings in `policy.yaml` based on the Architect's spec and the defaults in `girt.toml`.

### Resource limit in policy.yaml

```yaml
version: "1.0"
permissions:
  network:
    allow:
      - host: "api.github.com"
  storage: {}
  environment: {}
resources:
  memory_mb: 128
  fuel: 500000000
  timeout_seconds: 15
  max_response_bytes: 5242880
```

---

## 15. Testing Strategy for GIRT Itself

GIRT is tested at three levels:

### 15.1 Unit Tests

- **Hookwise decision engine:** Test each cascade layer in isolation. Mock registry, embedding store, and LLM. Verify correct short-circuiting behavior.
- **Queue operations:** Test atomic file moves, concurrent access, malformed request handling.
- **MCP proxy:** Test request interception, routing, and response forwarding.
- **Secret wrapper:** Test host function injection, secret lookup, and response sanitization. Verify secrets never appear in logs or WASM memory.
- **OCI client:** Test artifact push/pull, cache TTL, tag resolution.

### 15.2 Integration Tests

- **Full build pipeline (mocked LLMs):** Feed a capability spec through the pipeline with deterministic LLM responses. Verify the correct sequence: Creation Gate → Architect → Engineer → QA → Red Team → Publish.
- **Wassette integration:** Load a known-good .wasm component into Wassette via GIRT. Verify execution, policy enforcement, and hot-reload.
- **Bug ticket loop:** Simulate QA failure, verify the ticket routes to Engineer, verify the circuit breaker fires after 3 iterations.
- **Registry round-trip:** Build a tool, publish to a local OCI registry, verify it's discoverable by the Creation Gate's registry lookup.

### 15.3 End-to-End Tests

- **Real LLM, real Wassette:** Submit a capability request ("build a tool that converts Celsius to Fahrenheit"). Verify the full pipeline produces a working, published WASM component that returns correct results.
- **Rejection path:** Submit a known-dangerous request (e.g., "read /etc/shadow"). Verify the Creation Gate denies it without reaching the Engineer.
- **DEFER path:** Pre-load a "temperature converter" tool in the registry. Submit a similar request. Verify the Creation Gate defers to the existing tool.
- **Circuit breaker E2E:** Submit a request that's intentionally difficult to implement correctly. Verify the pipeline halts after 3 iterations and escalates.

### 15.4 Testing Convention

Per global conventions:
- E2E tests must pass before merging to main
- Tests are never disabled to force a build to pass
- Tests are only updated when business logic changes

---

## 16. Implementation Phases

### Phase 0: Foundation (Weeks 1-2)

**Goal:** Skeleton project with MCP proxy and Wassette integration.

- [ ] Initialize Rust project with Cargo workspace
- [ ] Implement minimal MCP proxy (stdio transport, pass-through to Wassette)
- [ ] Verify: agent connects through GIRT to Wassette, existing tools work
- [ ] Set up CI (build, lint, unit tests)
- [ ] Establish OCI registry structure on ghcr.io/epiphytic

**Deliverable:** GIRT as a transparent MCP proxy. No decision engine, no build pipeline. Just a working pipe between agent and Wassette.

### Phase 1: Decision Engine (Weeks 3-4)

**Goal:** Creation and Execution gates with the hookwise cascade.

- [ ] Implement policy rules layer (pattern matching for known-good/known-bad)
- [ ] Implement decision cache (spec hash → decision)
- [ ] Implement registry lookup layer (OCI registry query)
- [ ] Implement CLI/native check layer (known CLI utility list)
- [ ] Implement LLM evaluation layer (Anthropic API call)
- [ ] Implement HITL layer (surfaces to user via MCP or plugin)
- [ ] Wire Creation Gate into `request_capability` interception
- [ ] Wire Execution Gate into `call_tool` interception
- [ ] Unit + integration tests for each layer

**Deliverable:** GIRT intercepts tool calls and capability requests, makes allow/deny/defer/ask decisions. No build pipeline yet — allowed creation requests return "not yet implemented."

### Phase 2: Build Pipeline (Weeks 5-8)

**Goal:** Architect → Engineer → QA → Red Team pipeline, producing real WASM tools.

- [ ] Implement Architect agent (LLM call with spec refinement prompt)
- [ ] Implement Engineer agent (LLM call → Rust code → `cargo component build`)
- [ ] Implement QA agent (LLM generates test payloads → executes via Wassette)
- [ ] Implement Red Team agent (LLM generates exploit payloads → executes via Wassette)
- [ ] Implement bug ticket routing (QA/Red Team → Engineer)
- [ ] Implement circuit breaker (max 3 iterations → HITL escalation)
- [ ] Implement policy.yaml generation from Architect spec
- [ ] Implement OCI publishing on success
- [ ] Implement local cache for built tools
- [ ] Implement hot-reload notification (`tools/list_changed`)
- [ ] Integration tests: full pipeline with mocked LLMs
- [ ] E2E test: real LLM builds a simple tool end-to-end

**Deliverable:** Complete build pipeline. An agent can request a capability and receive a working WASM tool.

### Phase 3: Claude Code Plugin (Weeks 9-11)

**Goal:** Package GIRT as a Claude Code plugin with agent team orchestration.

- [ ] Create plugin.json manifest
- [ ] Create agent definitions (pipeline-lead, architect, engineer, qa, red-team)
- [ ] Implement file-based queue (`~/.girt/queue/`)
- [ ] Implement Pipeline Lead orchestration logic
- [ ] Create skills (request-capability, list-tools, promote-tool)
- [ ] Create hooks (capability-intercept, tool-call-gate)
- [ ] Create commands (/girt-status, /girt-build, /girt-registry)
- [ ] Wire MCP servers (.mcp.json for Wassette + GIRT proxy)
- [ ] Integration test: full pipeline via Claude Code agent team
- [ ] E2E test: user requests capability, team builds and delivers it

**Deliverable:** Installable Claude Code plugin. Full pipeline runs as an agent team.

### Phase 4: Secret Wrapper + Standard Library (Weeks 12-14)

**Goal:** Zero-knowledge secret handling and pre-built tool ecosystem.

- [ ] Implement `host_auth_proxy` Wasmtime host function
- [ ] Implement secret store facade (env, keychain backends)
- [ ] Build standard library tools (HTTP client, JSON, file I/O, GitHub, GitLab, text processing, crypto)
- [ ] Publish standard library to Epiphytic Public Registry
- [ ] Implement similarity check layer (embedding-based spec matching)
- [ ] E2E test: tool uses secret proxy to call authenticated API

**Deliverable:** Secret handling works. Standard library available. Similarity matching reduces duplicate builds.

### Phase 5: Hardening + Multi-Language (Weeks 15-18)

**Goal:** Production readiness.

- [ ] Add Go support to Engineer agent
- [ ] Add AssemblyScript support to Engineer agent
- [ ] Implement resource limits in policy.yaml generation
- [ ] Implement structured logging at all pipeline stages
- [ ] Implement metrics collection (optional backend)
- [ ] Security audit of the GIRT proxy itself (not just generated tools)
- [ ] Load testing: concurrent capability requests
- [ ] Documentation: user guide, plugin installation, registry contribution guide
- [ ] Implement tool promotion pipeline (private → public)

**Deliverable:** Production-grade GIRT with multi-language support, observability, and hardened security.

---

## 17. Open Design Decisions (Future ADRs)

These items need dedicated Architecture Decision Records before implementation:

1. **ADR-001: Wassette fork strategy.** Under what conditions do we fork Wassette vs. contribute upstream vs. wrap externally? Define the threshold.
2. **ADR-002: WIT interface standardization.** Should GIRT-generated tools follow a standard WIT world definition, or is each tool's interface bespoke?
3. **ADR-003: Tool versioning semantics.** When the Engineer patches a bug, is it v1.1 of the same tool or a new tool? How do consumers pin versions?
4. **ADR-004: Multi-language build targets.** How does the Engineer select the appropriate language? User config? Spec hint? Automatic based on capability type?
5. **ADR-005: Remote GIRT service.** Should the build pipeline run locally, or as a cloud service that returns pre-built tools? Cost and latency tradeoffs.
6. **ADR-006: Tool promotion pipeline.** What criteria and process moves a tool from Epiphytic Private to Public? Automated? Manual review? Both?
7. **ADR-007: Claude Agent Team Orchestration.** *(Written)* Pipeline runs as a Claude Code agent team via plugin. See [docs/adrs/ADR-007](../adrs/ADR-007-claude-agent-team-orchestration.md).
8. **ADR-008: Queue polling vs. filesystem watching.** Should the Pipeline Lead poll on a timer or use inotify/kqueue? Tradeoffs between latency, API cost, and portability.
9. **ADR-009: Pipeline Lead persistence model.** Long-running agent (preserves context, risks bloat) vs. fresh-per-request (clean context, loses inter-request learning). Hybrid options.
