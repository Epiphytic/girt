---
name: "girt:request-capability"
description: "Submit a capability request to the GIRT build pipeline. Describe what tool you need and GIRT will build, test, and publish it as a sandboxed WASM component."
---

# Request Capability

Submit a new capability request to the GIRT build pipeline.

## Usage

When the user or an agent needs a tool that doesn't exist, use this skill to request it.

## Steps

1. **Gather the spec** from the user or infer it from context:
   - `name`: A descriptive snake_case name for the tool
   - `description`: What the tool does and why it's needed
   - `inputs`: Input parameter schema (JSON object)
   - `outputs`: Expected output schema (JSON object)
   - `constraints`: Security constraints
     - `network`: List of allowed hosts (e.g., ["api.github.com"])
     - `storage`: List of allowed storage paths
     - `secrets`: List of required secrets (e.g., ["GITHUB_TOKEN"])

2. **Write the request** to the queue:
   ```bash
   cat > ~/.girt/queue/pending/$(date +%s)-${name}.json << 'SPEC'
   {
     "id": "req_$(uuidgen | tr -d '-' | head -c 16)",
     "timestamp": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
     "source": "operator",
     "spec": {
       "name": "<name>",
       "description": "<description>",
       "inputs": { ... },
       "outputs": { ... },
       "constraints": {
         "network": [],
         "storage": [],
         "secrets": []
       }
     },
     "status": "pending",
     "priority": "normal",
     "attempts": 0
   }
   SPEC
   ```

3. **Notify the user** that the request has been queued and the pipeline will process it.

4. Alternatively, use the `request_capability` MCP tool if GIRT is running as a proxy:
   ```
   Call request_capability with the spec as arguments
   ```

## Example

User says: "I need a tool to fetch GitHub issues"

Request:
```json
{
  "name": "github_issue_query",
  "description": "Query GitHub issues with filtering by state, labels, and assignee, with pagination support",
  "inputs": {
    "owner": "string (required)",
    "repo": "string (required)",
    "state": "string (optional, default: open)",
    "labels": "array of strings (optional)",
    "page": "integer (optional, default: 1)",
    "per_page": "integer (optional, default: 30)"
  },
  "outputs": {
    "issues": "array of issue objects",
    "total_count": "integer",
    "next_page": "integer or null"
  },
  "constraints": {
    "network": ["api.github.com"],
    "storage": [],
    "secrets": ["GITHUB_TOKEN"]
  }
}
```
