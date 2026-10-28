---
name: run-desktop
description: Launch outl-desktop and drive it for real — click, type, hover, screenshot, measure. Use when a change has to be seen in the running app rather than in a test, when hunting UX defects, or when reproducing a bug report about the desktop client. Covers the workspace sandbox that keeps the user's real graph out of reach.
---

# Running outl-desktop for real

Tests prove the logic. They do not catch a hover that pushes the page
down 18pt, a modal that never takes the keyboard, or a picker whose
first screen is four years old. Those need the app.

## Never drive the real workspace

The desktop opens `[workspace] last` from the global config, which on a
daily driver is the user's actual graph. Driving it means synthetic
keystrokes land in real notes — and they do land: a stray `a` is vim's
append, so `abc` typed at the wrong moment becomes text in a block.

`XDG_CONFIG_HOME` is honoured by **both** `outl_config::config_dir` and
`outl_core`'s device dir, so one variable isolates config, actor and
device identity together.

```bash
LAB=<scratch>/uxlab
cp -Rc ~/outl-p2p "$LAB/ws"          # APFS clonefile: 375M in under a second
rm -f "$LAB/ws/.outl/peers.json" "$LAB/ws/.outl/.lock" "$LAB/ws/.outl/.peers.lock"
mkdir -p "$LAB/cfg/outl"
cat > "$LAB/cfg/outl/config.toml" <<TOML
[workspace]
last = "$LAB/ws"
[sync]
transport = "file"
TOML
```

`transport = "file"` plus a deleted `peers.json` is the part that
matters: a clone carries the user's peer list, and an iroh transport
would dial their real devices and sync a new actor's ops into the real
graph. Clone first, cut the network second, launch third.

**Deleting the workspace's `peers.json` is not enough on its own.**
`migrate_global_peers_if_absent` runs on first touch and copies
`~/.outl/peers.json` back in, so the lab log reads `peers migration:
copied ...` on the first open and the file is there again. The
`transport = "file"` line is what actually keeps it inert, which makes
it the load-bearing half rather than the belt to the delete's braces.
Check the log for that line and confirm no endpoint was bound before
driving anything.

Then:

```bash
cd crates/outl-desktop
env XDG_CONFIG_HOME="$LAB/cfg" bun run tauri dev
```

The first build is minutes. Afterwards `target/debug/outl-desktop` can
be launched directly with the same env, which is faster and survives
the shell that started it.

## Driving it on macOS

The desktop is WKWebView, not Chromium — no CDP, no Playwright, and the
argent tools do not attach. Input goes through Quartz `CGEvent`,
observation through `screencapture`. Three traps, all paid for:

- **`System Events` cannot find the window** under a tiling WM. Use
  `aerospace list-windows --all` to get the id and
  `aerospace focus --window-id <id>` to bring it up.
- **WKWebView ignores a CGEvent carrying only a unicode string on
  keycode 0.** A whole word typed that way lands nowhere. Send real
  keycodes; keep the unicode payload as the fallback for characters
  that have none.
- **Focusing the window by the WM is not the same as clicking into it.**
  After a WM focus the webview may not be first responder, and keys go
  to the document instead of to a focused input. When a test needs
  "normal" focus, click a block first.
- **Check focus before every key, not once at the start** — use
  `drive.require_focus(id)`. A window can die or lose focus between two
  calls, and the keys then land in whatever is in front. An `Escape`
  and two typed words went into a different terminal session this way.
- **A modifier set only as a flag leaks into the next keystroke.**
  Press and release the modifier as its own key around the chord, and
  clear flags on every plain key. `Cmd+P` then typing `meet` turned the
  `m` into `Cmd+M` and minimised the window mid-test.

`scripts/drive.py` in this skill's directory is the driver: `key`,
`text`, `move`, `click`, `scroll`, `shot`, `focus`.

**It needs pyobjc, which no python on this machine has.** Not the
system one, not homebrew's. Import it through `uv` instead, and add
`ApplicationServices` when the script also resizes a window:

```bash
uv run --with pyobjc-framework-Quartz \
       --with pyobjc-framework-ApplicationServices \
       python3 <your-script>.py
```

`screencapture -x -o -l <CGWindowID>` captures the window alone, with
no desktop behind it and no drop shadow, which is what a screenshot for
a PR wants. The id comes from `Quartz.CGWindowListCopyWindowInfo`, not
from aerospace, though on this machine the two happen to agree.
**Under a tiling WM the window keeps its tile size**, so an
Accessibility `AXSize` write is reverted; run `aerospace layout
floating` on it first, then set the size.

## Run the A/B fast, and run it more than once

A single sample is not a result, and a slow harness is what tempts you
to take one. Three things make the difference between minutes and
seconds per arm:

- **Swap the frontend, do not relaunch the app.** Vite HMR picks up an
  edit in about a second. Relaunch only when the test needs a state the
  app only has at startup.
- **Poll for readiness, never `sleep`.** Wait for the window to appear
  and for the screenshot to stop being blank. A fixed sleep is a guess
  that gets padded every time it is wrong.
- **Run both arms in one process**, N trials each, and report the rate.

The picker-focus investigation took minutes per sample until it was
rewritten this way; the rewrite did 10 trials in 37 seconds, and that
is what showed the proposed fix changed nothing (5/5 both arms). One
sample per arm had looked like a fix.

## Measure, don't squint

A screenshot proves the app launched. It does not prove a layout is
stable. Pillow plus a row-ink profile turns "the text dances" into a
number:

```python
pa = (255 - numpy.asarray(before.convert("L"), float)).sum(axis=1)
pb = (255 - numpy.asarray(after.convert("L"), float)).sum(axis=1)
# best alignment shift of a band = how far that band moved
```

That is what turned "o `+ prop` faz o texto dançar" into "hovering one
row pushes everything below it down by exactly 18pt", which names the
CSS line responsible. Park the pointer off the content column between
captures, or every frame carries a hover state you did not intend.

## What macOS cannot answer

Layout, CSS, keybindings and ordering are decided by the frontend and
the Rust core, so they reproduce anywhere the app runs. First-responder
and window-manager behaviour do not: that seam is WKWebView on macOS and
WebKitGTK on Linux.

For everything in the first group, `docker/` here builds the Linux app
and runs it under Xvfb, driven by `xdotool` — repeatable, and it does
not hold a human's screen.

```bash
docker build -t outl-uxlab docker/
docker run --rm -v "$PWD":/src:ro -v outl-uxlab-build:/build \
  -v outl-uxlab-cargo:/root/.cargo/registry -v "$PWD/docker":/s:ro \
  outl-uxlab bash /s/build.sh                       # ~3 min warm
docker run -d --name outl-ux -v outl-uxlab-build:/build \
  -v "$LAB/ws":/ws -v "$LAB/out":/out outl-uxlab sleep infinity
docker exec outl-ux bash /s/run.sh
```

`run.sh` carries the four traps in comments; read it before changing it.

**Keyboard works, but only with a window manager running.** Without one
the window maps and even takes clicks, while every key is dropped — and
the symptom is indistinguishable from an app that ignores shortcuts. The
first read of this was wrong: `GTK_IM_MODULE=gtk-im-context-simple` was
set to "fix typing" and it did let characters reach a focused field,
through the input method, *without* a JS `keydown`. So shortcuts stayed
dead and the lab looked keyboard-blind. With matchbox running and the IM
module unset, `ctrl+p`, `Escape`, `j`/`k` and typing all behave.

`run.sh` starts the WM and leaves `GTK_IM_MODULE` unset for that reason.
If keys stop landing, check the WM is up before suspecting the app.

Two worked examples: the `+ prop` reflow, reproduced at 18px and
re-measured at 0px with mousemove alone; and keyboard navigation walking
the cursor off the bottom of a long page, found by pressing `j` sixty
times and asking where the selection was.
