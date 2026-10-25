set -euo pipefail
# Bring the lab up. Each piece here cost a failed run to find.
export DISPLAY=:99

# Keep the GTK input method OUT of the way. With it set, characters
# reach a focused field through the IM and produce no JS `keydown`, so
# every document-level shortcut looks dead while typing looks fine —
# which is how this lab was first mis-diagnosed as keyboard-blind. With
# a window manager running, plain X key events work without it.
unset GTK_IM_MODULE

# 1. An X server with a keymap, and a window manager. Without a WM the
#    window maps but never takes input focus, so nothing reacts.
mkdir -p /tmp/xdg && chmod 700 /tmp/xdg
pgrep -f '[X]vfb' >/dev/null || { Xvfb :99 -screen 0 1600x1000x24 -nolisten tcp & sleep 2; }
pgrep -f '[m]atchbox-window-manager' >/dev/null || \
  { matchbox-window-manager -use_titlebar no >/tmp/wm.log 2>&1 & sleep 1; }

# 2. A debug build reads `devUrl` from tauri.conf.json, NOT the bundled
#    `dist`. With nothing on that port the window renders exactly one
#    line: "Could not connect to localhost: Connection refused".
#    It must be started detached — a child of `docker exec` without -d
#    is reaped when that exec returns, which looks identical.
curl -sf -o /dev/null http://localhost:1421/ || {
  cd /build/repo/crates/outl-desktop
  nohup bun run dev --port 1421 --strictPort >/tmp/vite.log 2>&1 &
  for _ in $(seq 1 30); do sleep 1; curl -sf -o /dev/null http://localhost:1421/ && break; done
}

# 3. The app. Kill by EXACT command line: a loose `pkill -f outl-desktop`
#    matches the shell running this script and kills its own parent.
for p in $(pgrep -x -f "/build/target/debug/outl-desktop" || true); do kill "$p"; done
sleep 1
cd / && nohup /build/target/debug/outl-desktop >/tmp/app.log 2>&1 &

for _ in $(seq 1 40); do
  sleep 1
  W=$(xdotool search --onlyvisible --name outl 2>/dev/null | tail -1)
  [ -n "${W:-}" ] && { xdotool windowactivate "$W" >/dev/null 2>&1; echo "window $W"; exit 0; }
done
echo "no window after 40s; see /tmp/app.log" >&2
exit 1
