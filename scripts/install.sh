#!/usr/bin/env bash
# GIRT install script
# Builds and installs the girt binary, and installs girt.toml to the standard user config path.
# After running this, `girt` works from any directory and any Claude Code project.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
CONFIG_DIR="${HOME}/.config/girt"

echo "==> Installing girt binary..."
cd "${REPO_ROOT}"
cargo install --path crates/girt-proxy --locked
echo "    Installed: $(which girt)"

echo "==> Installing config to ${CONFIG_DIR}/girt.toml..."
mkdir -p "${CONFIG_DIR}"

if [ -f "${CONFIG_DIR}/girt.toml" ]; then
  echo "    Config already exists — skipping (edit ${CONFIG_DIR}/girt.toml to change settings)"
else
  cp "${REPO_ROOT}/girt.toml" "${CONFIG_DIR}/girt.toml"
  echo "    Copied girt.toml → ${CONFIG_DIR}/girt.toml"
fi

echo ""
echo "==> Done!"
echo ""
echo "    To use GIRT with Claude Code, add to your project's .mcp.json:"
echo '    {'
echo '      "mcpServers": {'
echo '        "girt": {'
echo '          "command": "girt",'
echo '          "env": { "GIRT_LOG": "info" }'
echo '        }'
echo '      }'
echo '    }'
echo ""
echo "    Or open the girt/ repo in Claude Code — .mcp.json is already there."
echo ""
echo "    Credentials: set ANTHROPIC_API_KEY, or use OpenClaw auth (already configured)."
