#!/usr/bin/env bash
# Every RFC in `docs/rfcs/` is reachable from `docs/SUMMARY.md`.
#
# `docs/SUMMARY.md` is described as the index of `docs/`, and an RFC that
# is not in it exists only for whoever already knows its number.
#
# This exists because the index was silently wrong five times in a row.
# The branch that added compaction, the snapshot cache lifecycle, the
# `tree → .md` executor, the `Op::Create` inverse and the index sidecar
# GC added five RFCs and listed none of them — including the two
# documenting the changes most capable of destroying data. The index was
# complete each time it was last edited; five separate changes each
# forgot the same step.
#
# Nothing failed when that happened, which is exactly why it happened
# five times. The fix for a step everyone forgets is not to remind
# people, it is to make forgetting fail.
#
# Deliberately narrow: it checks reachability, not wording. The title in
# the index is the RFC author's to write, and a script that second-
# guessed it would become a second owner of that fact (root `CLAUDE.md`
# → one owner per fact).
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
summary="$repo_root/docs/SUMMARY.md"
rfc_dir="$repo_root/docs/rfcs"

if [ ! -f "$summary" ]; then
  printf 'check-docs-index: %s not found\n' "$summary" >&2
  exit 1
fi

missing=()
count=0

for path in "$rfc_dir"/[0-9][0-9][0-9][0-9]-*.md; do
  [ -e "$path" ] || continue
  file="$(basename "$path")"
  count=$((count + 1))
  # Match the filename as it would appear in a link target. Any link
  # shape is fine — the question is reachability, not formatting.
  if ! grep -qF "rfcs/$file" "$summary"; then
    missing+=("$file")
  fi
done

if [ ${#missing[@]} -gt 0 ]; then
  printf 'FAIL: %d RFC(s) not reachable from docs/SUMMARY.md\n\n' "${#missing[@]}" >&2
  for file in "${missing[@]}"; do
    title="$(head -1 "$rfc_dir/$file" | sed 's/^# *//')"
    printf '  %s\n    %s\n' "$file" "$title" >&2
  done
  printf '\nAdd one line per RFC to the RFC list in docs/SUMMARY.md:\n' >&2
  printf '  * [%s — <title>](rfcs/%s)\n' "${missing[0]%%-*}" "${missing[0]}" >&2
  printf '\nWrite the title yourself — this script deliberately does not\n' >&2
  printf 'generate it, because the summary line belongs to the RFC author.\n' >&2
  exit 1
fi

printf 'PASS: %d RFC(s), all reachable from docs/SUMMARY.md\n' "$count"
