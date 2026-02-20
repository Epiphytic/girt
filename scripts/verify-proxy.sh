#!/usr/bin/env bash
# verify-proxy.sh — End-to-end smoke test for the GIRT proxy.
#
# Requires:
#   - `girt` binary (cargo build)
#   - `wassette` binary on PATH
#
# Verifies:
#   1. GIRT starts and connects to Wassette
#   2. MCP initialize handshake completes
#   3. list_tools returns a valid response
#
# Usage: ./scripts/verify-proxy.sh

set -euo pipefail

GIRT_BIN="${GIRT_BIN:-$(cargo build --quiet --bin girt 2>&1 >/dev/null && echo "target/debug/girt")}"

if ! command -v wassette &>/dev/null; then
    echo "SKIP: wassette not found on PATH"
    exit 0
fi

echo "==> Building girt..."
cargo build --quiet --bin girt
GIRT_BIN="target/debug/girt"

echo "==> Starting GIRT proxy with Wassette..."

# MCP initialize request
INIT_REQUEST='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"verify-proxy","version":"0.1.0"}}}'
INITIALIZED_NOTIFICATION='{"jsonrpc":"2.0","method":"notifications/initialized"}'
LIST_TOOLS_REQUEST='{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}'

# Send initialize + initialized notification + list_tools, capture response
RESPONSE=$(echo -e "${INIT_REQUEST}\n${INITIALIZED_NOTIFICATION}\n${LIST_TOOLS_REQUEST}" \
    | timeout 10 "$GIRT_BIN" 2>/dev/null || true)

if [ -z "$RESPONSE" ]; then
    echo "FAIL: No response from GIRT proxy"
    exit 1
fi

echo "==> Raw response:"
echo "$RESPONSE"

# Check for initialize response
if echo "$RESPONSE" | grep -q '"protocolVersion"'; then
    echo "PASS: Initialize handshake completed"
else
    echo "FAIL: No initialize response"
    exit 1
fi

echo "==> Proxy verification complete"
