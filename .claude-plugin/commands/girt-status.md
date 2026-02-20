---
name: "girt:girt-status"
description: "Show the current status of the GIRT pipeline, queue, and cached tools."
---

# /girt-status

Display a comprehensive status report of the GIRT system.

## When This Command Is Invoked

Gather and display the following information:

### 1. Queue Status

Check each queue directory and count files:

```bash
echo "=== Queue Status ==="
echo "Pending:     $(ls ~/.girt/queue/pending/*.json 2>/dev/null | wc -l) requests"
echo "In Progress: $(ls ~/.girt/queue/in_progress/*.json 2>/dev/null | wc -l) requests"
echo "Completed:   $(ls ~/.girt/queue/completed/*.json 2>/dev/null | wc -l) requests"
echo "Failed:      $(ls ~/.girt/queue/failed/*.json 2>/dev/null | wc -l) requests"
```

### 2. Cached Tools

List all built tools with their status:

```bash
for tool_dir in ~/.girt/tools/*/; do
  if [ -f "$tool_dir/manifest.json" ]; then
    name=$(basename "$tool_dir")
    echo "- $name"
  fi
done
```

### 3. Pipeline Configuration

Report on the GIRT binary and Wassette availability:

```bash
echo "=== Configuration ==="
which girt 2>/dev/null && echo "GIRT proxy: installed" || echo "GIRT proxy: not found"
which wassette 2>/dev/null && echo "Wassette: installed" || echo "Wassette: not found"
which cargo-component 2>/dev/null && echo "cargo-component: installed" || echo "cargo-component: not found"
```

### 4. Present Results

Format the output as a clean status report for the user. Include any warnings (missing dependencies, failed builds, etc.).
