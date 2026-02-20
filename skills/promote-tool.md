---
name: "girt:promote-tool"
description: "Promote a locally built tool to the public OCI registry. Runs additional validation before publishing."
---

# Promote Tool

Promote a tool from the local cache to the public OCI registry.

## Prerequisites

- Tool must exist in `~/.girt/tools/<tool_name>/`
- Tool must have passed both QA and Red Team (check manifest.json)
- User must confirm the promotion

## Steps

1. **Verify the tool exists** and has passing results:
   ```bash
   cat ~/.girt/tools/<tool_name>/manifest.json | jq '.qa_result.passed, .security_result.passed'
   ```

2. **Show the tool spec** to the user for confirmation:
   ```bash
   cat ~/.girt/tools/<tool_name>/manifest.json | jq '.spec'
   ```

3. **Ask the user** to confirm promotion:
   - Tool name and description
   - Network/storage/secrets permissions
   - QA test results
   - Security audit results

4. **Push to OCI registry** (when implemented):
   ```bash
   # Future: oras push ghcr.io/epiphytic/girt-tools/<tool_name>:latest \
   #   ~/.girt/tools/<tool_name>/tool.wasm:application/wasm \
   #   ~/.girt/tools/<tool_name>/policy.yaml:text/yaml \
   #   ~/.girt/tools/<tool_name>/manifest.json:application/json
   ```

5. **Report success** to the user with the OCI reference.

## Note

OCI registry push is not yet implemented. Currently tools are stored in the local cache only. This skill will be fully functional after Phase 5 (Hardening).
