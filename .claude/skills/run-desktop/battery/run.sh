#!/usr/bin/env bash
# Run the desktop layout battery. Idempotent: the slow parts (image,
# cargo build) happen once and are reused from named volumes.
#
#   .claude/skills/run-desktop/battery/run.sh [--rebuild]
#
# Everything runs in the lab container, so the host needs docker and
# nothing else — no Xvfb, no Python, and no human watching a window.
set -uo pipefail

repo_root="${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel 2>/dev/null)}"
[ -n "$repo_root" ] || { echo "not in a git repo" >&2; exit 1; }
skill="$repo_root/.claude/skills/run-desktop"
lab="${TMPDIR:-/tmp}/outl-uxlab"
img=outl-uxlab
name=outl-ux

command -v docker >/dev/null || {
  echo "docker not found. The battery runs the Linux build in a container;" >&2
  echo "there is no host-side fallback." >&2
  exit 1
}

say() { printf '\n== %s\n' "$1"; }

if [ "${1:-}" = "--rebuild" ]; then
  docker rm -f "$name" >/dev/null 2>&1
  docker volume rm outl-uxlab-build >/dev/null 2>&1
fi

if ! docker image inspect "$img" >/dev/null 2>&1; then
  say "building the lab image (once, a few minutes)"
  docker build -t "$img" "$skill/docker" || exit 1
fi

# The app binary lives in a named volume, so a second run skips cargo.
if ! docker run --rm -v outl-uxlab-build:/build "$img" \
       test -x /build/target/debug/outl-desktop 2>/dev/null; then
  say "building outl-desktop for Linux (once, a few minutes)"
  docker run --rm \
    -v "$repo_root":/src:ro \
    -v outl-uxlab-build:/build \
    -v outl-uxlab-cargo:/root/.cargo/registry \
    -v "$skill/docker":/s:ro \
    "$img" bash /s/build.sh || exit 1
else
  # Source changed since the binary was built? Only the frontend matters
  # for a layout battery, and vite serves it from the copy, so refresh
  # that rather than paying for cargo again.
  say "refreshing the frontend in the lab"
  docker run --rm -v "$repo_root":/src:ro -v outl-uxlab-build:/build "$img" \
    bash -c 'cd /src && tar -cf - crates/outl-desktop/src crates/outl-frontend-shared/src \
             | (cd /build/repo && tar -xf -)' || exit 1
  # Files written into the container this way raise no watch event vite
  # can see, and a change inside the SHARED package is outside its watch
  # root anyway. Restarting the app alone is not enough: the page
  # reloads and vite serves the stylesheet it already had. Measuring
  # that looks exactly like a fix that does not work, which cost four
  # wrong readings before a red `outline` proved the rule was absent.
  if docker ps --format '{{.Names}}' | grep -qx "$name"; then
    docker exec "$name" bash -lc \
      'for p in $(pgrep -f "[v]ite"); do kill $p; done' >/dev/null 2>&1 || true
  fi
fi

say "fixture workspace"
# Clear the CONTENTS, never the directory: a bind mount follows the
# inode, so deleting and recreating `ws` on the host leaves the running
# container looking at a directory that no longer exists. The app then
# sits on "Opening your workspace..." forever and the battery reports a
# page that never painted.
mkdir -p "$lab/ws" "$lab/out"
find "$lab/ws" -mindepth 1 -delete 2>/dev/null
bash "$skill/battery/fixture.sh" "$lab/ws" || exit 1

if ! docker ps --format '{{.Names}}' | grep -qx "$name"; then
  docker rm -f "$name" >/dev/null 2>&1
  docker run -d --name "$name" --shm-size=512m \
    -v outl-uxlab-build:/build \
    -v "$lab/ws":/ws \
    -v "$lab/out":/out \
    -v "$skill":/skill:ro \
    "$img" sleep infinity >/dev/null || exit 1
fi

say "starting the app"
docker exec "$name" bash -lc '
  mkdir -p /cfg/outl
  printf "[workspace]\nlast = \"/ws\"\n[theme]\npreset = \"outl\"\n[sync]\ntransport = \"file\"\n" \
    > /cfg/outl/config.toml' || exit 1
docker exec -e XDG_CONFIG_HOME=/cfg "$name" bash /skill/docker/run.sh || exit 1

say "layout battery"
docker exec "$name" python3 /skill/battery/probe.py
rc=$?

# The Stop hook reads this to know the battery has seen exactly these
# bytes. A hash, not a timestamp: restoring a file from a backup leaves
# the content identical and the mtime new, and the hook would then ask
# for a run that proves what this one just proved. Only a pass stamps.
if [ $rc -eq 0 ]; then
  bash "$skill/battery/surfaces-hash.sh" "$repo_root" \
    > "$repo_root/.claude/.ux-battery-stamp"
fi

printf '\nframes: %s\n' "$lab/out"
exit $rc
