set -euo pipefail
# A workspace with the block shapes that catch layout bugs, written as
# plain markdown: outl adopts orphan `.md` on first open, so no CLI build
# is needed to make one.
#
# Deterministic on purpose. A battery that runs against the developer's
# own graph measures a different page every week and cannot fail twice
# the same way.
WS="${1:-/ws}"

# Start from nothing. The lab's workspace outlives a run, and outl is
# op-log-first: once a day has been opened, the log remembers what that
# page is and the app RE-PROJECTS it over whatever is on disk. So a run
# that opened an empty 2026-11-13 left an op log saying "that day is
# empty", and the next run's freshly seeded markdown was truncated back
# to 0 bytes before the battery ever looked — the file on the host read
# perfect, and the app was right to ignore it.
#
# That is the opposite of the determinism this file claims three lines
# down, and no amount of seeding fixes it: the authority is `.outl/ops`,
# not the `.md`.
#
# The CONTENTS go, never the directory: the container bind-mounts this
# path, so replacing the inode strands the mount inside a running lab
# (`run.sh` recycles on that, but a wipe every run would make it recycle
# every run). Guarded on the basename for the obvious reason — this
# takes a path argument, and a typo'd one would run `rm -rf` somewhere
# real.
case "$WS" in
  */ws) find "$WS" -mindepth 1 -maxdepth 1 -exec rm -rf {} + 2>/dev/null || true ;;
  *) echo "fixture: refusing to wipe '$WS' — expected a path ending in /ws" >&2; exit 1 ;;
esac

mkdir -p "$WS/journals" "$WS/pages"

# The CONTENT is fixed; the day it lands on cannot be. The app opens
# today's journal, so a hardcoded filename measures a page that exists
# only until midnight. It shipped that way and the battery failed on the
# next morning's first run, reporting "the outline never painted" for an
# app that had painted an empty day correctly.
#
# `date` here is the HOST's, and the app's is the CONTAINER's, which
# runs UTC. So the two disagree for as many hours as the developer is
# behind UTC — three, in UTC-3 — and in that window the battery seeded
# one day and the app opened the next, empty. Same failure as the
# midnight one, with a timezone instead of a clock.
#
# So the content lands on today AND tomorrow. Seeding both is cheaper
# than making the two clocks agree: forcing the container's TZ would
# still leave `outl_actions::clock`'s own configured timezone free to
# resolve a third day, and the battery only ever reads one of these.
TODAY="$(date +%F)"
YESTERDAY="$(date -v-1d +%F 2>/dev/null || date -d 'yesterday' +%F)"
TOMORROW="$(date -v+1d +%F 2>/dev/null || date -d 'tomorrow' +%F)"

cat > "$WS/journals/$TODAY.md" <<'MD'
- a short block
- a block carrying a property
  remind:: 9am
- a block long enough to wrap onto a second and probably a third visual line, which is what makes one keypress look like several skipped lines
- a parent block
  - a nested child
  - another child
    - one level deeper
- a block with [[a ref]] and a #tag in it
- TODO an open task
- DONE a closed task
-
- the last block
MD

# Whichever of the two the app calls "today", it finds the same page.
cp "$WS/journals/$TODAY.md" "$WS/journals/$TOMORROW.md"

cat > "$WS/pages/a-ref.md" <<'MD'
- so the ref above resolves
MD

# A page taller than any window, for everything that only goes wrong
# past the fold: keyboard navigation that does not scroll, a cursor
# that parks under a fixed overlay, a last row that cannot be reached.
{
  for i in $(seq -w 1 80); do
    echo "- block $i on a long page that needs scrolling"
  done
} > "$WS/journals/$YESTERDAY.md"
