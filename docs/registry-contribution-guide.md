# Registry Contribution Guide

## Overview

GIRT tools are distributed as OCI artifacts via the Epiphytic public registry at `ghcr.io/epiphytic/girt-tools`. This guide explains how to contribute tools to the registry.

## Who Can Contribute

- Epiphytic organization members
- Trusted delegates approved by the org

All contributions go through the [girt-tools](https://github.com/Epiphytic/girt-tools) monorepo on GitHub.

## Tool Requirements

Every tool submitted to the registry must meet these requirements:

### Source Availability (ADR-002)
- Full source code included in the submission
- Buildable from source with standard toolchain
- No binary-only submissions

### SLSA Provenance (ADR-002)
- GitHub Actions generates provenance attestations automatically
- Unattested tools are blocked by default in GIRT configurations

### Interface Compliance
- Tools implement the `girt-world@0.1.0` WIT interface
- Standard input/output schemas documented in the tool manifest

### Security
- No hardcoded secrets or credentials
- Uses `host_auth_proxy` for authenticated API calls
- Passes Red Team audit (SSRF, path traversal, prompt injection checks)
- Network access restricted to declared hosts in constraints

### Quality
- Passes QA functional testing
- Includes a `manifest.json` with name, version, description, WIT world
- Follows semver for versioning

## Submitting a Tool

### 1. Fork the girt-tools monorepo

```bash
git clone https://github.com/Epiphytic/girt-tools.git
cd girt-tools
```

### 2. Add your tool

Create a directory for your tool:

```
tools/
  my-tool/
    src/lib.rs          # Source code (Rust, Go, or AssemblyScript)
    world.wit           # WIT interface definition
    manifest.json       # Tool metadata
    policy.yaml         # Wassette policy (resource limits, network hosts)
    Cargo.toml          # Build configuration
```

### 3. Write the manifest

```json
{
  "name": "my-tool",
  "version": "1.0.0",
  "description": "What this tool does",
  "wit_world": "girt-world@0.1.0",
  "language": "rust",
  "constraints": {
    "network": ["api.example.com"],
    "storage": [],
    "secrets": ["EXAMPLE_API_KEY"]
  }
}
```

### 4. Build and test locally

```bash
# Build the WASM component
cargo component build --release

# Test with GIRT's QA pipeline
cargo run --bin girt -- --test-tool tools/my-tool/
```

### 5. Submit a pull request

```bash
git checkout -b add-my-tool
git add tools/my-tool/
git commit -m "feat: add my-tool for <description>"
git push origin add-my-tool
# Create PR on GitHub
```

### 6. Automated review

GitHub Actions will:
1. Build the WASM component from source
2. Run the QA agent against it
3. Run the Red Team agent against it
4. Generate SLSA provenance attestation
5. If all checks pass, publish to `ghcr.io/epiphytic/girt-tools/my-tool:1.0.0`

## Versioning (ADR-003)

- All tools follow [semver](https://semver.org/)
- The `latest` tag tracks the most recent stable release
- Breaking changes require a major version bump

## Staleness Policy (ADR-003)

| Duration | Action |
|----------|--------|
| 6 months unused | Flagged for review |
| 9 months unused | Deprecated (soft-deleted, removed from `latest`) |
| 12 months unused | Removal candidate |

Healthy tools are automatically rebuilt to stay current with dependency updates.

## Naming Conventions

- Lowercase, hyphen-separated: `http-fetch`, `json-transform`, `github-api`
- Descriptive of the tool's primary function
- No vendor prefixes unless wrapping a specific service

## Standard Library

The following tools are maintained by the Epiphytic team and included by default:

| Tool | Description |
|------|-------------|
| `http-fetch` | HTTP client with WASI HTTP |
| `json-transform` | JSON parsing and transformation |
| `text-process` | Text manipulation and regex |
| `github-api` | GitHub REST API client |
| `gitlab-api` | GitLab REST API client |
| `file-io` | Sandboxed file read/write |
| `crypto-hash` | Cryptographic hashing (SHA-256, BLAKE3) |
