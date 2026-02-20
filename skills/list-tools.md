---
name: "girt:list-tools"
description: "Browse available tools in the GIRT local cache and OCI registries. Shows built tools, their specs, and build status."
---

# List Tools

Browse tools available in the GIRT ecosystem.

## Steps

1. **Check local cache** for built tools:
   ```bash
   ls ~/.girt/tools/
   ```

2. **For each tool**, read its manifest:
   ```bash
   cat ~/.girt/tools/<tool_name>/manifest.json
   ```

3. **Check the queue** for in-progress builds:
   ```bash
   ls ~/.girt/queue/pending/
   ls ~/.girt/queue/in_progress/
   ```

4. **Present results** to the user in a clear format:

   | Tool | Description | Status | Build Iterations |
   |------|-------------|--------|------------------|
   | github_issue_query | Query GitHub issues | Built | 1 |
   | csv_parser | Parse CSV files | Building | - |

5. If the user needs a tool that isn't listed, suggest using the `request-capability` skill to build it.
