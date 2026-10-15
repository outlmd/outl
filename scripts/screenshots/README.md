# Release screenshots

`shoot.mjs` regenerates the product screenshots for desktop, iPhone, Android and the TUI.
The release workflow runs it for every release and attaches the images to the draft before it is published (`.github/workflows/release-screenshots.yml`).
Nothing it produces is committed.

## How it works

1. `fixture/` holds the "Atlas" demo workspace as plain outl markdown: person pages with `type:: person`, `[[@Name]]` mentions, tags, page and block properties, tasks, a table, a code block, a `remind::` and a template.
2. The real `outl` binary imports it (`outl import logseq`, the importer that takes a directory of `.md` as-is), and the CLI's JSON (`page get`, `page prop get`, `backlinks page`, `search`, `template list`) becomes the data for the web clients.
3. Desktop and mobile run their Vite dev servers under `?mock=1` in headless Chromium.
   `lib/bridge.js` sits in front of the mock backend and answers the content commands from that JSON, so every block, token and backlink on screen is backend output.
4. The TUI is the real binary on the same workspace, recorded with [vhs](https://github.com/charmbracelet/vhs) from the tapes in `tui/`.
5. `lib/frame.mjs` puts each capture on a background, as a window or a phone.

The fixture is dated relative to the day the script runs.
`journals/day-1.md` becomes yesterday's journal and `{{day:+7}}` inside a file becomes the date a week from today, so "today" is always a populated journal.
That placeholder only exists in the fixture; it is expanded before the import.

## Output

```
<out>/raw/<platform>-<shot>-<light|dark>.png
<out>/framed/<platform>-<shot>-<light|dark>-framed.png
<out>/hero/<platform>-<shot>-<scheme>.png   # the picks in HERO (shots.mjs)
```

Platforms are `desktop`, `mobile-iphone`, `mobile-android` and `tui`.
The shot list lives in `shots.mjs` (web) and `tui/*.tape` (TUI).

## Running it locally

You need the release CLI, the frontend dependencies, Chromium for Playwright, and `vhs` + `ttyd` + `ffmpeg` on `PATH` for the TUI.

```sh
cargo build --release --bin outl
bun install                                  # repo root, for the frontends
cd scripts/screenshots
bun install
bunx playwright install chromium
node shoot.mjs                               # writes target/screenshots/
node shoot.mjs --only desktop,mobile --out /tmp/shots
```

Ports 1420 and 1421 must be free; the script starts and stops the dev servers itself.

## What is left out, and why

These would show a bug instead of the feature, so the shot list skips them:

- Block refs and embeds on mobile: outl-mobile never calls `resolve_embeds`, so `((blk-…))` renders as the raw handle.
- Desktop pages with page properties opened from the sidebar, the quick switcher or a deep link: `AppShell.applyView` drops `page_properties` and skips embeds.
  The desktop shots reach those pages by clicking a `[[ref]]`, which goes through `OutlineView.applyView` and keeps them.
- Block refs, embeds and query results anywhere: their handles only exist after the import, so the fixture can't name them.
