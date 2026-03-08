#!/usr/bin/env bash
set -euo pipefail

BASE="${TOPCLAW_SELF_BASE:-$HOME/topclaw-self}"
STABLE="$BASE/stable"
CANDIDATE="$BASE/candidate"
RELEASES="$BASE/releases"
LOGS="$BASE/logs"

if [[ "${1:-}" == "--help" ]]; then
  cat <<'EOF'
Prepare a writable candidate copy for controlled TopClaw self-improvement.

Environment:
  TOPCLAW_SELF_BASE   Override the default base directory ($HOME/topclaw-self)

Behavior:
  - creates the base directories if missing
  - requires an existing stable checkout
  - replaces candidate with a fresh copy of stable
EOF
  exit 0
fi

mkdir -p "$STABLE" "$CANDIDATE" "$RELEASES" "$LOGS"

if [[ ! -d "$STABLE/.git" ]]; then
  echo "Stable checkout not found at $STABLE" >&2
  echo "Clone TopClaw there first, for example:" >&2
  echo "  git clone https://github.com/jackfly8/TopClaw.git \"$STABLE\"" >&2
  exit 1
fi

rm -rf "$CANDIDATE"
cp -a "$STABLE" "$CANDIDATE"

if git -C "$CANDIDATE" rev-parse --verify main >/dev/null 2>&1; then
  git -C "$CANDIDATE" checkout -B self-improve/candidate main >/dev/null 2>&1 || true
fi

cat <<EOF
Prepared candidate workspace:
  stable:    $STABLE
  candidate: $CANDIDATE
  releases:  $RELEASES
  logs:      $LOGS
EOF
