#!/usr/bin/env sh
set -eu

ROOT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" >/dev/null 2>&1 && pwd || pwd)"
LEGACY_INSTALLER="$ROOT_DIR/zeroclaw_install.sh"

if [ ! -f "$LEGACY_INSTALLER" ]; then
  echo "error: zeroclaw_install.sh not found from repository root." >&2
  exit 1
fi

exec "$LEGACY_INSTALLER" "$@"
