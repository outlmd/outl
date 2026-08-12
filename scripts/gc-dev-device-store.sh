#!/usr/bin/env bash
# Drop actor bindings in `.dev-device-store/actors/` whose workspace is gone.
#
# WHY THIS EXISTS
#
# `.cargo/config.toml` points `$OUTL_DEVICE_DIR` at `.dev-device-store` so the
# iroh identity survives `cargo clean` (deleting it rotates this machine's node
# id and voids every pairing). The trade is that nothing prunes it any more:
# the suite opens a temp workspace per test and each one leaves an actor
# binding behind. One session left 935 records, all pointing at `TempDir`
# paths that no longer exist.
#
# WHAT IS SAFE TO DELETE, AND WHAT IS NOT
#
#   actors/   safe. A binding is a workspace→actor mapping; drop one and the
#             next open mints a fresh actor for that workspace. For a temp
#             workspace that never runs again, it is pure garbage.
#   iroh/     NOT safe. `identity.key` IS this device's node id. Deleting it
#             is the exact failure this store was moved to prevent.
#
# So this only ever touches `actors/`, and only records whose `root=` path is
# gone from disk. A live workspace keeps its actor.
set -euo pipefail

store="${OUTL_DEVICE_DIR:-$(cd "$(dirname "$0")/.." && pwd)/.dev-device-store}"
actors="$store/actors"
[ -d "$actors" ] || { echo "no actor store at $actors"; exit 0; }

dry_run=false
[ "${1:-}" = "--dry-run" ] && dry_run=true

removed=0
kept=0
for record in "$actors"/*; do
  [ -f "$record" ] || continue
  root=$(sed -n 's/^root=//p' "$record" | head -1)
  # A record with no `root=` is malformed, not orphaned. Leave it: guessing
  # about a record we cannot read is how a GC deletes something live.
  if [ -z "$root" ]; then
    kept=$((kept + 1))
    continue
  fi
  if [ -d "$root" ]; then
    kept=$((kept + 1))
  else
    $dry_run || rm -f "$record"
    removed=$((removed + 1))
  fi
done

if $dry_run; then
  echo "would remove $removed orphaned binding(s), keep $kept"
else
  echo "removed $removed orphaned binding(s), kept $kept"
fi
