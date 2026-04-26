#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"

echo "DevOpster macOS installer"
echo ""
"$SCRIPT_DIR/install-macos-app.sh" "$@"
echo ""
read -r -n 1 -s -p "Installation finished. Press any key to close..."
echo ""
