---
name: "girt:girt-registry"
description: "Manage GIRT tool registries. List, add, or remove OCI registry configurations."
args: "[list|add|remove] [registry_url]"
---

# /girt-registry

Manage GIRT tool registry configurations.

## When This Command Is Invoked

### Subcommands

**`/girt-registry list`** (default if no args):
- Show configured registries from `~/.girt/config.json`
- Show the local cache path (`~/.girt/tools/`)
- Show registry health status if available

**`/girt-registry add <url>`**:
- Add a new OCI registry URL to the configuration
- Validate the URL format
- Test connectivity if possible
- Save to `~/.girt/config.json`

**`/girt-registry remove <url>`**:
- Remove a registry URL from the configuration
- Confirm with the user before removing

### Configuration File

Registry config is stored in `~/.girt/config.json`:

```json
{
  "registries": [
    {
      "url": "ghcr.io/epiphytic/girt-tools",
      "type": "oci",
      "default": true
    }
  ],
  "local_cache": "~/.girt/tools/",
  "queue_path": "~/.girt/queue/"
}
```

### Note

OCI registry integration is planned for Phase 5. Currently, the local cache at `~/.girt/tools/` is the only available registry. This command prepares the configuration infrastructure for future registry support.
