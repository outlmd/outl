#!/usr/bin/env bash
# Print a content hash of the UX surfaces, or nothing if none exist.
#
# Content, not mtime: restoring a file from a backup rewrites its
# timestamp while leaving the bytes identical, and a timestamp check
# then asks for a battery run that would prove exactly what the last
# one proved. That fired within an hour of the hook being written.
set -uo pipefail
root="${1:-.}"
list="$root/.claude/skills/run-desktop/battery/ux-surfaces.txt"
[ -f "$list" ] || exit 0
cd "$root" || exit 0
while IFS= read -r entry; do
  case "$entry" in ''|'#'*) continue ;; esac
  if [ -d "$entry" ]; then
    find "$entry" -type f \( -name '*.ts' -o -name '*.tsx' -o -name '*.css' \) -print0
  elif [ -f "$entry" ]; then
    printf '%s\0' "$entry"
  fi
done < "$list" | sort -z | xargs -0 -r shasum -a 256 2>/dev/null | shasum -a 256 | cut -d' ' -f1
