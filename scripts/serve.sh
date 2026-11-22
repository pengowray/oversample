#!/usr/bin/env bash
# Serves the web app at http://localhost:21347 with `trunk serve --release`.
# Extra arguments go to trunk, e.g. scripts/serve.sh --open
# Runs from the repo root.
set -euo pipefail
cd "$(dirname "$0")/.."
exec trunk serve --release --port 21347 "$@"
