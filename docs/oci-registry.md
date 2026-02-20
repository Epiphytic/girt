# OCI Registry Structure

GIRT tools are distributed as OCI artifacts via container registries.

## Registry Hierarchy

### Epiphytic Public Registry (default)

```
ghcr.io/epiphytic/girt-tools/<tool-name>:<version>
```

- Curated, reviewed tools available to all GIRT users
- Included by default in every GIRT configuration
- Contributions from Epiphytic org members or trusted delegates
- Requires SLSA provenance attestations (ADR-002)

### Epiphytic Private Registry

```
ghcr.io/epiphytic/girt-tools-private/<tool-name>:<version>
```

- Internal pipeline tools, pre-review
- Accessible only to Epiphytic org members
- Tools graduate to public after review

### User-Defined Registries

Users can configure additional registries in their GIRT config:

```toml
# ~/.config/girt/config.toml
[[registries]]
name = "epiphytic-public"
url = "ghcr.io/epiphytic/girt-tools"
default = true

[[registries]]
name = "epiphytic-private"
url = "ghcr.io/epiphytic/girt-tools-private"

[[registries]]
name = "my-org"
url = "ghcr.io/my-org/girt-tools"
```

## Artifact Format

Each tool artifact contains:

```
<tool>.wasm          # Compiled WebAssembly component
manifest.json        # Tool metadata (name, version, WIT world, description)
provenance.json      # SLSA provenance attestation
```

## Naming Conventions

- Tool names: lowercase, hyphen-separated (e.g., `http-fetch`, `json-transform`)
- Versions: semver (e.g., `1.0.0`, `0.2.1-beta`)
- Tags: `latest` tracks the most recent stable release

## Registry Lookup Order

When GIRT resolves a tool request, registries are searched in config order:

1. Local cache (`~/.cache/girt/tools/`)
2. Configured registries (top to bottom)
3. If no match: trigger Creation Gate pipeline

## GitHub Actions Publish Workflow

Tools in the `girt-tools` monorepo are published via GitHub Actions:

```yaml
# On tag push matching v*
- Build WASM component
- Generate SLSA provenance
- Push to ghcr.io/epiphytic/girt-tools/<name>:<version>
```

## Staleness Policy (ADR-003)

- Tools follow semver
- Tools unused for 6 months are flagged for review
- Deprecated tools are soft-deleted (tagged `deprecated`, removed from `latest`)
