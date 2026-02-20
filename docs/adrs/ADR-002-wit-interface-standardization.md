# ADR-002: WIT Interface Standardization and Tool Provenance

**Status:** Accepted
**Date:** 2026-02-20
**Context:** Whether GIRT-generated tools follow a standard WIT world definition, and how tool integrity and provenance are guaranteed

---

## Decision

### Standard WIT World

All GIRT-generated tools MUST implement a standard WIT world definition maintained by the Epiphytic org. This standard will be versioned (e.g., `girt-world@0.1.0`) and updated as new metadata or features are added.

The standard world defines:
- Common input/output type conventions
- Error reporting interface
- Metadata introspection (tool name, version, capabilities)
- Host function imports (`host_auth_proxy`, logging, etc.)

Individual tools extend the standard world with their specific function exports.

### Source Availability

All GIRT tools MUST be **source-available and inspectable**. Specifically:

1. **Source code is published.** Every tool's source (Rust/Go/etc.), WIT definition, and policy.yaml are committed to a public repository.
2. **Tools are buildable from source.** Any user can clone the source and reproduce the compiled `.wasm` artifact using the documented build process.
3. **Binary-only tools are not permitted** in Epiphytic registries. If source cannot be published (proprietary dependency, licensing issue), the tool cannot be listed.

### Provenance Attestations

All tools in Epiphytic registries MUST have valid provenance attestations:

1. **Final builds are done in GitHub Actions.** The CI pipeline compiles source to `.wasm`, runs QA and Red Team validation, and publishes to the OCI registry. Local builds are for development only — published artifacts come from CI.
2. **Attestations are attached to artifacts.** Each published tool includes a [SLSA provenance attestation](https://slsa.dev/) or equivalent, signed by the GitHub Actions workflow. This proves the artifact was built from the committed source in a controlled environment.
3. **Attestation verification is enforced by default.** GIRT validates attestations before loading tools from any registry. Tools without valid attestations are **blocked by default**.
4. **Users can opt out.** A configuration flag in `girt.toml` allows users to permit unattested tools. This is an explicit, informed choice — not the default.

```toml
# girt.toml
[security]
require_attestations = true   # default: true
# Set to false to allow tools without provenance attestations.
# WARNING: This disables supply chain verification.
```

## Context

The WASM Component Model uses WIT (WebAssembly Interface Types) to define component interfaces. Without a standard, every GIRT-generated tool would have a bespoke interface, making ecosystem tooling (search, composition, compatibility checks) fragile.

Supply chain attacks are a top threat in the Wassette threat model. LLM-generated code is particularly risky because the generation process is non-deterministic — the same prompt can produce different code. Provenance attestations close this gap by ensuring the published binary matches audited source built in a trusted environment.

## Rationale

- **Standardized WIT enables ecosystem tooling.** The Architect can reason about tool compatibility, the registry can validate interfaces, and users can compose tools knowing they share common conventions.
- **Source availability enables trust.** Users and auditors can inspect what a tool does before allowing it. The Red Team agent's audit is a point-in-time check; source availability enables ongoing review.
- **CI-only publishing eliminates "it works on my machine" attacks.** A compromised developer workstation cannot inject malicious code into published artifacts if the build happens in GitHub Actions.
- **Attestation-by-default follows zero-trust principles.** Users who need unattested tools can enable them, but the safe path is the default path.

## Consequences

- The `girt-world` WIT standard is maintained in the `girt-tools` repository and versioned with semver.
- Breaking changes to the WIT standard require a major version bump and a migration guide.
- The Engineer agent's system prompt references the current WIT standard version and must be updated when the standard changes.
- The GitHub Actions workflow for tool publishing must include attestation generation.
- Tools from user-defined registries without attestations are blocked unless the user explicitly opts out.

## WIT Standard (Initial v0.1.0)

```wit
// girt-world@0.1.0

package girt:standard@0.1.0;

interface types {
    record error-response {
        error: string,
        code: string,
        details: option<string>,
    }

    record paginated-response {
        items: list<string>,        // JSON-encoded items
        next-cursor: option<string>,
    }

    record tool-metadata {
        name: string,
        version: string,
        description: string,
        girt-world-version: string,
    }
}

interface introspection {
    use types.{tool-metadata};
    get-metadata: func() -> tool-metadata;
}

world girt-tool {
    import girt:host/auth-proxy@0.1.0;
    import girt:host/logging@0.1.0;
    import wasi:http/outgoing-handler@0.2.0;

    export introspection;
    // Individual tools add their specific exports here
}
```
