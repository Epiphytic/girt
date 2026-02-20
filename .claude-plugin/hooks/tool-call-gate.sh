#!/usr/bin/env bash
# GIRT Tool Call Gate Hook
#
# This hook runs on PreToolUse events and logs tool calls for
# the Execution Gate audit trail. In the future, this will be
# replaced by the GIRT proxy's built-in Execution Gate.
#
# Currently: logs tool calls to ~/.girt/audit.log
# Future: intercepts and routes through the decision engine

set -euo pipefail

# Read the tool call input from stdin
INPUT=$(cat)

TOOL_NAME=$(echo "$INPUT" | jq -r '.tool_name // "unknown"' 2>/dev/null)

# Skip GIRT's own tools and standard Claude Code tools
case "$TOOL_NAME" in
    request_capability|girt-*|Read|Write|Edit|Bash|Glob|Grep|TaskCreate|TaskUpdate|TaskList|SendMessage)
        exit 0
        ;;
esac

# Ensure audit log directory exists
AUDIT_DIR="$HOME/.girt"
mkdir -p "$AUDIT_DIR"

# Log the tool call
TIMESTAMP=$(date -u +%Y-%m-%dT%H:%M:%SZ)
echo "{\"timestamp\":\"$TIMESTAMP\",\"tool\":\"$TOOL_NAME\"}" >> "$AUDIT_DIR/audit.log"

exit 0
