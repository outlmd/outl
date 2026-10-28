set -euo pipefail
# A workspace with the block shapes that catch layout bugs, written as
# plain markdown: outl adopts orphan `.md` on first open, so no CLI build
# is needed to make one.
#
# Deterministic on purpose. A battery that runs against the developer's
# own graph measures a different page every week and cannot fail twice
# the same way.
WS="${1:-/ws}"
mkdir -p "$WS/journals" "$WS/pages"

# The CONTENT is fixed; the day it lands on cannot be. The app opens
# today's journal, so a hardcoded filename measures a page that exists
# only until midnight. It shipped that way and the battery failed on the
# next morning's first run, reporting "the outline never painted" for an
# app that had painted an empty day correctly.
TODAY="$(date +%F)"
YESTERDAY="$(date -v-1d +%F 2>/dev/null || date -d 'yesterday' +%F)"

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
