#!/bin/sh
set -eu
TASK_DIR=$(CDPATH= cd "$(dirname "$0")" && pwd)
export PATH="$HOME/.local/bin:$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"
exec python3 "$TASK_DIR/panel.py" "$@"
