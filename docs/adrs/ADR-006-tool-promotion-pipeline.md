# ADR-006: Tool Promotion Pipeline

**Status:** Accepted
**Date:** 2026-02-20
**Context:** How tools move from development to public availability

---

## Decision

### Initial Approach: All Public via GitHub Actions

For the initial release, the promotion model is simple:

1. **All tool source code lives in a single public repository:** `epiphytic/girt-tools`
2. **All tools are built and published via GitHub Actions.** There is no separate private-then-promote flow initially.
3. **Publishing is automated.** When source code is merged to `girt-tools` main branch, CI builds the WASM component, runs QA and Red Team validation, generates provenance attestations (see [ADR-002](./ADR-002-wit-interface-standardization.md)), and publishes to the Epiphytic Public Registry.

### Repository Structure

```
epiphytic/girt-tools/
├── MANIFEST.md                  # Registry of all tools
├── girt-world/
│   └── wit/
│       └── girt-standard.wit    # Standard WIT world definition (ADR-002)
├── tools/
│   ├── http-client/
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   ├── component.wit
│   │   ├── policy.yaml
│   │   └── spec.json
│   ├── github-issues/
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   ├── component.wit
│   │   ├── policy.yaml
│   │   └── spec.json
│   └── ...
├── .github/
│   └── workflows/
│       ├── build-and-publish.yml    # CI: build, test, attest, publish
│       ├── staleness-check.yml      # Monthly: flag stale tools (ADR-003)
│       └── rebuild-all.yml          # Periodic: rebuild against latest deps
└── docs/
    └── contributing.md
```

### GitHub Actions Workflow

```
PR merged to main
    │
    ▼
[Detect changed tools]
    │
    ▼ (for each changed tool)
[Compile to WASM Component]
    │
    ▼
[Run QA validation in Wassette sandbox]
    │
    ▼
[Run Red Team validation in Wassette sandbox]
    │
    ▼ (all pass)
[Generate SLSA provenance attestation]
    │
    ▼
[Publish to oci://ghcr.io/epiphytic/girt-tools-public]
    │
    ▼
[Update MANIFEST.md]
```

### Contribution Model

- **Epiphytic org members** can commit directly to `girt-tools`.
- **Delegated contributors** (trusted external contributors) submit PRs that require org member approval.
- **Community contributions** are accepted via PR with mandatory code review by an org member.
- **AI-generated tools** from the GIRT pipeline that pass QA + Red Team can be auto-submitted as PRs to `girt-tools` by the Pipeline Lead, but still require human merge approval.

### Future: Private Registry Tier

The two-tier model (private → public) described in the main design document will be implemented when:
- The internal pipeline generates enough tools to warrant a staging area
- There's a defined vetting criteria beyond "QA + Red Team pass"
- The org needs tools that can't be public (proprietary integrations, internal APIs)

At that point, the `girt-tools-private` registry will be added with its own CI pipeline and a promotion process to move tools to public.

## Context

The original design proposed a two-tier system (private → public). For the initial release, this adds complexity without clear benefit — we don't yet have enough tools or contributors to need a staging layer. Starting with everything public and automated keeps the process simple and transparent.

## Rationale

- **Simplicity for launch.** One repo, one registry, one CI pipeline. Easy to understand, easy to contribute to.
- **Transparency builds trust.** All source is public, all builds are in CI, all artifacts have attestations. Users can verify everything.
- **GitHub Actions is the build authority.** No local builds are published. This eliminates supply chain risk from compromised developer machines.
- **The private tier is deferred, not rejected.** When the need arises, it's a straightforward addition — same CI, separate registry, promotion step.

## Consequences

- `epiphytic/girt-tools` must be created as a public repository.
- GitHub Actions workflows must be configured for WASM compilation, Wassette-based testing, and OCI publishing.
- SLSA attestation generation must be integrated into the publish workflow.
- `MANIFEST.md` in `girt-tools` serves as the human-readable tool catalog.
- The GIRT Pipeline Lead needs a mechanism to submit PRs to `girt-tools` when it builds a tool worth publishing.
