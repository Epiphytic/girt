---
name: "girt:girt-build"
description: "Manually trigger a GIRT build for a capability. Accepts a tool name or spec file path."
args: "[tool_name_or_spec_path]"
---

# /girt-build

Manually trigger a build in the GIRT pipeline.

## When This Command Is Invoked

### Parse Arguments

The user may provide:
- A tool name: `/girt-build github_issue_query` -- look up the spec or prompt the user for details
- A spec file path: `/girt-build ./spec.json` -- read the spec from the file
- No arguments: `/girt-build` -- prompt the user for a capability description

### Steps

1. **Determine the spec**:
   - If a file path is given, read the JSON spec from the file
   - If a tool name is given, check if it's already in the cache or queue
   - If no arguments, use the `request-capability` skill to gather the spec from the user

2. **Enqueue the request**:
   - Create the queue directories if needed: `mkdir -p ~/.girt/queue/{pending,in_progress,completed,failed}`
   - Write the request JSON to `~/.girt/queue/pending/`
   - Use a UUID-based filename for uniqueness

3. **Trigger the pipeline**:
   - If the GIRT MCP proxy is running, call the `request_capability` tool directly
   - Otherwise, notify the user that the Pipeline Lead agent should be spawned to process the queue

4. **Report status**:
   - Confirm the request was enqueued
   - Show the request ID for tracking
   - Tell the user to check `/girt-status` for progress
