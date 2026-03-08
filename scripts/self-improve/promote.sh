#!/usr/bin/env bash
set -euo pipefail

BASE="${TOPCLAW_SELF_BASE:-$HOME/topclaw-self}"
STABLE="$BASE/stable"
CANDIDATE="$BASE/candidate"
RELEASES="$BASE/releases"
LOGS="$BASE/logs"
DENYLIST="${DENYLIST:-$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/deny_paths.txt}"
STAMP="$(date +%Y%m%d-%H%M%S)"
NEWREL="$RELEASES/$STAMP"
BACKUP="$BASE/stable.backup.$STAMP"
LOGFILE="$LOGS/promote-$STAMP.log"

mkdir -p "$RELEASES" "$LOGS"

trim_line() {
  local line="$1"
  line="${line#"${line%%[![:space:]]*}"}"
  line="${line%"${line##*[![:space:]]}"}"
  printf '%s' "$line"
}

path_matches_rule() {
  local path="$1"
  local rule="$2"
  if [[ "$rule" == */ ]]; then
    [[ "$path" == "$rule"* ]]
  else
    [[ "$path" == "$rule" || "$path" == "$rule/"* ]]
  fi
}

collect_changed_paths() {
  local stable_dir="$1"
  local candidate_dir="$2"
  python3 - "$stable_dir" "$candidate_dir" <<'PY'
import filecmp
import os
import sys

stable = sys.argv[1]
candidate = sys.argv[2]
changed = set()


def rel(base: str, path: str) -> str:
    value = os.path.relpath(path, base)
    return "." if value == "." else value.replace(os.sep, "/")


def walk(dcmp: filecmp.dircmp) -> None:
    for name in dcmp.left_only:
        changed.add(rel(stable, os.path.join(dcmp.left, name)))
    for name in dcmp.right_only:
        changed.add(rel(candidate, os.path.join(dcmp.right, name)))
    _, mismatch, errors = filecmp.cmpfiles(
        dcmp.left,
        dcmp.right,
        dcmp.common_files,
        shallow=False,
    )
    for name in mismatch + errors:
        changed.add(rel(stable, os.path.join(dcmp.left, name)))
    for sub in dcmp.subdirs.values():
        walk(sub)


walk(filecmp.dircmp(stable, candidate, ignore=["target", ".git", ".gitignore"]))

for item in sorted(path for path in changed if path != "."):
    print(item)
PY
}

assert_no_denied_path_changes() {
  local stable_dir="$1"
  local candidate_dir="$2"
  local denylist_file="$3"
  local changed_paths
  local -a denied_hits=()

  if [[ ! -f "$denylist_file" ]]; then
    echo "Denylist file not found: $denylist_file" >&2
    exit 1
  fi

  changed_paths="$(collect_changed_paths "$stable_dir" "$candidate_dir")"
  if [[ -z "$changed_paths" ]]; then
    return 0
  fi

  while IFS= read -r raw_rule; do
    local rule
    rule="$(trim_line "$raw_rule")"
    [[ -z "$rule" || "$rule" == \#* ]] && continue
    while IFS= read -r changed_path; do
      [[ -z "$changed_path" ]] && continue
      if path_matches_rule "$changed_path" "$rule"; then
        denied_hits+=("$changed_path")
      fi
    done <<< "$changed_paths"
  done < "$denylist_file"

  if (( ${#denied_hits[@]} > 0 )); then
    printf 'Promotion blocked: candidate changes denied path(s):\n' >&2
    printf '  %s\n' "${denied_hits[@]}" | sort -u >&2
    exit 1
  fi
}

if [[ ! -d "$CANDIDATE" ]]; then
  echo "Candidate directory not found: $CANDIDATE" >&2
  exit 1
fi

if [[ ! -d "$STABLE" ]]; then
  echo "Stable directory not found: $STABLE" >&2
  exit 1
fi

if [[ ! -f "$CANDIDATE/Cargo.toml" ]]; then
  echo "Candidate does not look like a TopClaw checkout: $CANDIDATE" >&2
  exit 1
fi

{
  echo "Promoting candidate at $STAMP"
  echo "Base: $BASE"
  echo "Candidate: $CANDIDATE"
  echo "Stable: $STABLE"
  echo "Denylist: $DENYLIST"
  echo

  assert_no_denied_path_changes "$STABLE" "$CANDIDATE" "$DENYLIST"

  cd "$CANDIDATE"

  cargo fmt --all -- --check
  cargo clippy --all-targets --all-features -- -D warnings
  cargo test
  cargo build --release

  mkdir -p "$NEWREL"
  cp -a "$CANDIDATE/." "$NEWREL/"

  if [[ -d "$STABLE" ]]; then
    mv "$STABLE" "$BACKUP"
  fi
  cp -a "$NEWREL" "$STABLE"

  echo
  echo "Promoted release: $STAMP"
  echo "Backup saved at: $BACKUP"
} 2>&1 | tee "$LOGFILE"
