#!/usr/bin/env bash
# CI counterpart to `.claude/hooks/file-size-guard.sh`.
#
# The hook is a `PostToolUse` hook, which means it only ever runs when
# *Claude Code* edits a file. A human in an editor, a Copilot PR, a
# dependabot bump — none of them pass through it. The repo's file-size
# discipline was therefore a property of who happened to be typing, and
# the divergence shows: `Journal.tsx` reached 3,212 lines without one
# warning ever being emitted, because it is TypeScript and the hook only
# read `.rs` until 2026-09.
#
# This script is the half that runs for everyone.
#
# ## Ratchet, not a cliff
#
# 66 files are already at or past the 600-line threshold. Failing on all
# of them would make this job permanently red, which trains everyone to
# ignore it — the failure mode that makes a guard worse than no guard.
#
# So the existing debt is frozen in `.github/file-size-baseline.txt` and
# the rule is directional:
#
#   - a file NOT in the baseline that crosses the threshold → FAIL
#   - a file in the baseline that grows past its recorded size → FAIL
#   - a file in the baseline that shrinks → PASS, with a nudge to
#     re-record the lower number so the ratchet tightens
#
# Existing large files stay editable — you simply cannot make them
# bigger. The baseline only moves down.
#
# Usage:
#   scripts/check-file-size.sh            # check (CI mode)
#   scripts/check-file-size.sh --update   # rewrite the baseline

set -uo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
baseline_file="${repo_root}/.github/file-size-baseline.txt"

# Matches the hook's "warning" threshold. A file at or past this is
# either recorded in the baseline or a new violation.
THRESHOLD=600

# Emit `<lines> <repo-relative-path>` for every source file we police,
# sorted by path so the baseline diffs cleanly.
collect() {
  cd "$repo_root" || exit 1
  # `-prune` rather than `-not -path`: the latter filters results but still
  # walks into node_modules/ and target/, which is most of the bytes on disk.
  find crates \
    \( -path '*/target' -o -path '*/node_modules' -o -path '*/dist' -o -path '*/gen' \) -prune \
    -o \( -name '*.rs' -o -name '*.ts' -o -name '*.tsx' \) \
    -print0 \
  | xargs -0 wc -l 2>/dev/null \
  | awk -v t="$THRESHOLD" '$2 != "total" && $1 >= t { print $1, substr($0, index($0, $2)) }' \
  | sort -k2
}

if [ "${1:-}" = "--update" ]; then
  # Build into a temp file and move it into place only once the scan
  # succeeded. Writing straight to `$baseline_file` truncates it first, and
  # `collect`'s `cd … || exit 1` is not in a subshell, so a failed scan left a
  # header-only baseline on disk — after which every one of the 66 recorded
  # files reads as a new violation on the next CI run.
  tmp=$(mktemp)
  trap 'rm -f "$tmp"' EXIT

  {
    cat <<HEADER
# Files at or past ${THRESHOLD} lines, frozen as a ratchet.
# Regenerate with: scripts/check-file-size.sh --update
#
# This file records existing debt so CI can block *growth* without
# blocking every edit to a file that was already too big. Numbers
# here should only ever go down. Adding a row is not a normal part
# of landing a change — it means a new file crossed the line, and
# the split should happen instead.
HEADER
    collect
  } > "$tmp"

  found=$(grep -vc '^#' "$tmp")
  if [ "$found" -eq 0 ]; then
    echo "error: scanned 0 files under crates/. Baseline left untouched." >&2
    exit 1
  fi

  mv "$tmp" "$baseline_file"
  trap - EXIT
  echo "baseline updated: $found files at or past ${THRESHOLD} lines"
  exit 0
fi

if [ ! -f "$baseline_file" ]; then
  echo "error: $baseline_file is missing. Generate it with: scripts/check-file-size.sh --update" >&2
  exit 1
fi

current=$(collect)

violations=()
shrunk=0
total=0

while read -r lines path; do
  [ -n "$path" ] || continue
  total=$((total + 1))
  recorded=$(awk -v p="$path" '$2 == p { print $1 }' "$baseline_file")

  if [ -z "$recorded" ]; then
    violations+=("  $path — $lines lines (not in baseline)")
  elif [ "$lines" -gt "$recorded" ]; then
    violations+=("  $path — $lines lines, baseline $recorded (+$((lines - recorded)))")
  elif [ "$lines" -lt "$recorded" ]; then
    shrunk=$((shrunk + 1))
  fi
done <<< "$current"

# A scan that matched nothing is a broken scan, not a clean repo. Without
# this the script prints "PASS: 0 file(s)" and exits 0 when `crates/` was
# renamed, the checkout is partial, or `find` failed — the ratchet silently
# stops ratcheting. `set -e` is deliberately absent (the loop needs to keep
# going past a missing baseline row) and `wc -l 2>/dev/null` swallows errors,
# so nothing else would catch it.
if [ "$total" -eq 0 ]; then
  echo "error: scanned 0 files under crates/. Expected hundreds." >&2
  echo "Something is wrong with the checkout or the find filters, not with the repo." >&2
  exit 1
fi

if [ "${#violations[@]}" -gt 0 ]; then
  echo "FAIL: ${#violations[@]} file(s) over the ${THRESHOLD}-line ratchet"
  echo
  printf '%s\n' "${violations[@]}"
  cat >&2 <<'MSG'

How to split a file this size: docs/architecture.md, or the
`refactor-architect` agent.

If the growth is genuinely unavoidable, say so in the PR and run
`scripts/check-file-size.sh --update` — that raises the ceiling
permanently, so it needs a reason a reviewer agrees with.
MSG
  exit 1
fi

echo "PASS: $total file(s) at or past ${THRESHOLD} lines, none grew."

if [ "$shrunk" -gt 0 ]; then
  echo
  echo "note: $shrunk file(s) shrank below their recorded size."
  echo "Run 'scripts/check-file-size.sh --update' to tighten the ratchet."
fi
