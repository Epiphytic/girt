# GIRT Plugin Installation

## Prerequisites

- [Claude Code](https://claude.ai/claude-code) installed
- Rust toolchain (stable)
- [Wassette](https://github.com/microsoft/wassette) installed and on PATH

## Install the Plugin

### From source (development)

```bash
# Clone the repository
git clone https://github.com/Epiphytic/girt.git

# Install as a Claude Code plugin
claude plugin add /path/to/girt
```

### Verify installation

```bash
# Check plugin is loaded
claude /girt-status
```

## What Gets Installed

The GIRT plugin adds:

### Slash Commands
- `/girt-status` — Show pipeline, queue, and tool cache status
- `/girt-build [name_or_spec]` — Manually trigger a capability build
- `/girt-registry [list|add|remove]` — Manage OCI registry configuration

### Skills
- `request-capability` — Submit a capability request to the build pipeline
- `list-tools` — Browse cached and built tools
- `promote-tool` — Push a tool to an OCI registry

### Agents
The plugin defines 5 agent personas that form the build pipeline team:

| Agent | Role |
|-------|------|
| Pipeline Lead | Queue consumer, orchestration, circuit breaker |
| Architect | Spec refinement and generalization |
| Engineer | Code generation and WASM compilation |
| QA | Functional correctness testing |
| Red Team | Adversarial security auditing |

### Hooks
- `capability-intercept` — Validates capability specs on `request_capability` calls
- `tool-call-gate` — Audit logging for all tool calls

### MCP Server
The plugin configures GIRT as an MCP server via `.mcp.json`, building from Cargo and launching with Wassette as the backend runtime.

## Configuration

After installation, configure GIRT in your project's `girt.toml`:

```toml
[build]
default_language = "rust"
default_tier = "standard"

[[registries.additional]]
name = "my-org"
url = "ghcr.io/my-org/girt-tools"
```

## Uninstall

```bash
claude plugin remove girt
```

## Troubleshooting

### "Wassette not found"
Ensure `wassette` is on your PATH:
```bash
which wassette
# If missing, install from https://github.com/microsoft/wassette
```

### Build failures
Check the Rust toolchain:
```bash
rustup show
cargo build --workspace
```

### Plugin not loading
Verify the plugin directory contains `.claude-plugin/plugin.json`:
```bash
ls /path/to/girt/.claude-plugin/plugin.json
```
