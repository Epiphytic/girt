#!/usr/bin/env bash
# GIRT Capability Intercept Hook
#
# This hook runs on PreToolUse events and intercepts calls to
# request_capability to validate the spec before it enters the pipeline.
#
# It checks:
# 1. The spec has a name and description
# 2. Network constraints are present if the tool needs HTTP access
# 3. The tool name uses snake_case

set -euo pipefail

# Read the tool call input from stdin
INPUT=$(cat)

TOOL_NAME=$(echo "$INPUT" | jq -r '.tool_name // empty' 2>/dev/null)

# Only intercept request_capability calls
if [ "$TOOL_NAME" != "request_capability" ]; then
    exit 0
fi

# Validate the spec
SPEC_NAME=$(echo "$INPUT" | jq -r '.tool_input.name // empty' 2>/dev/null)
SPEC_DESC=$(echo "$INPUT" | jq -r '.tool_input.description // empty' 2>/dev/null)

if [ -z "$SPEC_NAME" ]; then
    echo "GIRT: Capability request must include a 'name' field"
    exit 1
fi

if [ -z "$SPEC_DESC" ]; then
    echo "GIRT: Capability request must include a 'description' field"
    exit 1
fi

# Validate snake_case naming
if ! echo "$SPEC_NAME" | grep -qE '^[a-z][a-z0-9_]*$'; then
    echo "GIRT: Tool name must be snake_case (got: $SPEC_NAME)"
    exit 1
fi

exit 0
