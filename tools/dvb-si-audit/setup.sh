#!/usr/bin/env bash
# One-shot venv setup.
set -euo pipefail

cd "$(dirname "$0")"

if [[ ! -d .venv ]]; then
    python3 -m venv .venv
fi

.venv/bin/pip install --quiet --upgrade pip
.venv/bin/pip install --quiet -r requirements.txt

echo "Ready. Activate with: source tools/dvb-si-audit/.venv/bin/activate"
