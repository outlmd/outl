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
#
# RELATIONSHIP TO THE REAL GC
#
# `outl-core`'s `device/gc.rs` is the product answer, reached through
# `outl doctor --repair`. This script is the developer's faster sweep of a
# store that holds nothing but `TempDir` debris, and it differs in exactly
# one way now: it has **no TTL**, because test garbage does not deserve a
# 30-day wait.
#
# It used to differ in a second, worse way: it deleted on "the directory is
# missing" alone. That is the rule `gc.rs` explicitly rejects, because an
# unplugged drive and a deleted folder are the same observation. The parent
# check below closes that gap, so pointing `$OUTL_DEVICE_DIR` at a real
# store no longer applies a rule the product side refuses to apply.
set -euo pipefail

repo_store="$(cd "$(dirname "$0")/.." && pwd)/.dev-device-store"
store="${OUTL_DEVICE_DIR:-$repo_store}"
actors="$store/actors"
[ -d "$actors" ] || { echo "no actor store at $actors"; exit 0; }

if [ "$store" != "$repo_store" ]; then
  echo "warning: $store is not this repo's .dev-device-store." >&2
  echo "         This sweep has no TTL. For a real store prefer \`outl doctor --repair\`." >&2
fi

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
  # The absence has to be one we could observe. A missing parent means the
  # volume or the mount is gone, not the workspace, and every binding under
  # it has to survive: dropping one forks that workspace's actor when the
  # drive comes back. Same rule as `gc.rs`, same reason.
  elif [ ! -d "$(dirname "$root")" ]; then
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
