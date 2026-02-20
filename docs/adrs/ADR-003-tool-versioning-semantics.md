# ADR-003: Tool Versioning Semantics

**Status:** Accepted
**Date:** 2026-02-20
**Context:** How GIRT tools are versioned, updated, and retired

---

## Decision

### Versioning

GIRT tools follow [Semantic Versioning 2.0.0](https://semver.org/):

- **MAJOR** — Breaking changes to the WIT interface (renamed functions, changed parameter types, removed exports)
- **MINOR** — New functionality added in a backwards-compatible manner (new optional parameters, new exports)
- **PATCH** — Bug fixes and security patches that don't change the interface

When the Engineer patches a bug via the build loop (QA/Red Team failure), the resulting fix increments the **patch** version. The tool retains its identity — it's the same tool, improved.

When the Architect recommends extending an existing tool with new features, the resulting update increments the **minor** version.

### Version Pinning

Consumers reference tools by OCI tag:

- `oci://ghcr.io/epiphytic/girt-tools-public/github_issues:1.2.3` — exact version (immutable)
- `oci://ghcr.io/epiphytic/girt-tools-public/github_issues:1.2` — latest patch in 1.2.x
- `oci://ghcr.io/epiphytic/girt-tools-public/github_issues:latest` — latest version (resolves on TTL)

The Creation Gate's registry lookup matches against all available versions and selects the latest compatible one.

### Staleness Policy

Tools must be actively maintained to remain in Epiphytic registries:

| Age Without Update | Action |
|---|---|
| 0-6 months | Active. No action. |
| 6-9 months | **Flagged.** Tool is marked `stale` in registry metadata. Users see a warning when the tool is loaded. |
| 9-12 months | **Deprecated.** Tool is deprioritized in registry search results. The Creation Gate prefers building a fresh alternative over using a deprecated tool. |
| 12+ months | **Candidate for removal.** Removed from the registry after review. Source remains in the `girt-tools` repository for reference. |

**Exceptions:**
- Tools with no external dependencies and pure computational logic (math, string ops, hashing) may be exempt from staleness if their test suites still pass.
- Staleness timers reset on any version bump (including patch).

### Update Pipeline

In future versions, a scheduled CI job will:
1. Rebuild all tools against the latest WIT standard and Wassette version
2. Run QA and Red Team agents against the rebuilt artifacts
3. If tests pass, publish a patch bump automatically
4. If tests fail, flag the tool as stale and open an issue in the `girt-tools` repository

## Context

Without versioning discipline, registries accumulate abandoned tools that may have unpatched vulnerabilities or incompatible interfaces. The staleness policy ensures the ecosystem stays healthy and that users can trust tools in the Epiphytic registry are maintained.

## Rationale

- **Semver is the industry standard** for communicating the impact of changes. Using it means tooling (package managers, CI, dependabot) works out of the box.
- **Staleness is a proxy for security risk.** A tool last updated 12 months ago likely hasn't been tested against current dependencies or threat models.
- **Automated rebuilds reduce maintenance burden.** Most staleness is benign neglect, not intentional abandonment. Auto-rebuilding healthy tools keeps them fresh without human effort.

## Consequences

- Every tool's `manifest.json` includes a `version` field following semver.
- The OCI registry enforces immutability for exact version tags (you can't overwrite `1.2.3`).
- The `girt-tools` CI pipeline includes a staleness checker that runs monthly.
- The Architect is aware of tool versions and prefers the latest compatible version when recommending existing tools.
