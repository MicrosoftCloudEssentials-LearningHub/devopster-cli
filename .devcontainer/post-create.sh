#!/usr/bin/env bash
set -euo pipefail

echo "[post-create] Running in-container bootstrap..."
make bootstrap
