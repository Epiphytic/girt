#!/usr/bin/env bash
set -euo pipefail

echo "=== GIRT E2E Test Runner ==="
echo ""

# Check prerequisites
MISSING=()

if ! command -v cargo-component &>/dev/null; then
    MISSING+=("cargo-component (cargo install cargo-component)")
fi

if ! command -v wassette &>/dev/null; then
    MISSING+=("wassette (https://github.com/microsoft/wassette)")
fi

if ! command -v oras &>/dev/null; then
    echo "WARN: oras not found — OCI push tests will be skipped"
fi

if ! curl -sf http://localhost:8000/v1/models >/dev/null 2>&1; then
    MISSING+=("vLLM on localhost:8000 (not reachable)")
fi

if [ ${#MISSING[@]} -gt 0 ]; then
    echo "ERROR: Missing prerequisites:"
    for dep in "${MISSING[@]}"; do
        echo "  - $dep"
    done
    exit 1
fi

echo "All prerequisites OK."
echo ""
echo "Running E2E tests (this may take several minutes per test)..."
echo ""

cargo test -p girt-pipeline --test e2e_pipeline -- --ignored --nocapture "$@"
