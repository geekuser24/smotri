#!/usr/bin/env bash
# Platform icons are checked in and derived from assets/smotri/icon.png.
# Do not regenerate the removed upstream owl artwork.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
python3 "$ROOT/packaging/smotri-icons.py"
