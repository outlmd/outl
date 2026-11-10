# Excalidraw

Draw in [Excalidraw](https://excalidraw.com), keep the drawing in your notes.

A ```` ```excalidraw ```` fence renders as the actual drawing — hand-drawn strokes,
hachure fills, arrows, text — as inline SVG, right in the block. Three ways to
point it at a scene:

- **Paste the share link** from excalidraw.com (Menu → Share → Shareable link).
  No account, no API key.
- **An [Excalidraw+](https://plus.excalidraw.com) scene id**, pulled over the
  public API with a token from your OS keychain.
- **A pasted `.excalidraw` file**, which needs no network at all.

Your markdown stays clean either way: a fence with one line in it, no base64
blob, no image file to keep in sync.

## Quick start

Paste a share link, run the sync:

````markdown
```excalidraw
https://excalidraw.com/#json=A1b2C3dEf,xYz0-_K9
```
````

```sh
outl plugin run app.outl.examples.excalidraw excalidraw-sync
```

Two lines of markdown and nothing else, if you have a drawing to paste:

````markdown
```excalidraw
{"type":"excalidraw","elements":[{"type":"rectangle","x":0,"y":0,"width":120,"height":80,"seed":1}]}
```
````

With Excalidraw+, point at a scene instead and sync it once:

````markdown
```excalidraw
scene: 7Hs9kQmV2
```
````

```sh
outl plugin secret set app.outl.examples.excalidraw token
outl plugin run app.outl.examples.excalidraw excalidraw-sync
```

Don't know your scene ids? Run `excalidraw-scenes` and the plugin writes them
into a page for you, each with its fence ready to use.

## The fence

The body is either a pasted `.excalidraw` file (anything starting with `{`) or
`key: value` lines:

````markdown
```excalidraw
scene: 7Hs9kQmV2
theme: dark
height: 320
background: scene
title: Sync architecture
```
````

| Key | Value | Default |
|---|---|---|
| `scene` | An excalidraw.com share link, an Excalidraw+ scene id, or a URL carrying one. A bare value on its own line works too, so the key is optional. | — |
| `theme` | `auto`, `light` or `dark`. `auto` follows the client's colour scheme. | config, then `auto` |
| `height` | Fixed height in px. Otherwise the drawing keeps its aspect ratio up to `maxHeight`. | — |
| `background` | `scene` draws the scene's own background colour; `none` keeps it transparent. | `none` |
| `title` | Caption text. `title:` with nothing after it hides the caption. | the scene's name |

An unknown key is an error the block shows you, not a line it ignores. Same for
a theme that isn't one of the three, and a `height` that isn't a number.

## Where a scene comes from

**A share link** is the shortest path: draw on excalidraw.com, Menu → Share →
Shareable link, paste it. No account and no API key, because the link carries
its own decryption key.

Those scenes are end-to-end encrypted — the key lives in the URL fragment and
never reaches Excalidraw's server — so the plugin does by hand what the web app
does with WebCrypto: fetch the cipher text from `json.excalidraw.com`, decrypt
it (AES-128-GCM), inflate it, and read the scene. Two things follow from that:

- **The key sits in your markdown**, because it is part of the link. Anyone who
  can read the note can open the drawing. That is what a share link *is*, but
  it is worth knowing before pasting one into a shared page.
- **Decoding is pure JS on an interpreter** and costs roughly 0.3 ms per
  compressed byte — about ¾ of a second for a 200-element scene. It runs in the
  sync command, never while rendering, so it costs you once per change. Past
  32 KB compressed (a few thousand elements) the sync declines and tells you to
  paste the file instead, rather than freezing the app.

**An Excalidraw+ `scene:` reference** keeps the markdown to one line and the
drawing editable where you drew it. Edit in Excalidraw, run the sync, the block
updates. The cost is an API key and a sync step.

**A pasted `.excalidraw` file** works with no key, no network and no cache —
including on a machine that has never heard of your Excalidraw+ workspace. Good
for a drawing that is done, and for anything you want to survive the plugin
being uninstalled. The cost is a long line in your markdown.

Two excalidraw.com link formats are refused, each by name rather than as an
empty frame:

- A **`#room=` collaboration link** is a live websocket session, not a stored
  scene. There is nothing to fetch. Use the share link instead.
- A **`#url=` link** points at a file on some other host, and the network
  permissions you approved name Excalidraw's own domains. Config cannot widen a
  permission, so open that file and paste its contents.

## Commands

| Command | What it does |
|---|---|
| `excalidraw-sync` | Pulls every scene your fences reference into the local cache. The ✏️ toolbar button runs this. |
| `excalidraw-scenes` | Writes your Excalidraw+ scenes into a page, one block each with a ready-made fence under it. |
| `excalidraw-cache-clear` | Drops every cached scene. |

From the command palette they read as "Excalidraw: sync scenes" and friends. Headless:

```sh
outl plugin run app.outl.examples.excalidraw excalidraw-sync
```

`excalidraw-scenes` is safe to re-run. It reads what the page already references
and appends only what's missing, so a second run adds the scenes you created
since the first one instead of duplicating the page.

## The API key

Only **Excalidraw+** scenes need one; a share link and a pasted file do not. Create it in your Excalidraw+ workspace settings,
then:

```sh
outl plugin secret set app.outl.examples.excalidraw token
# Value for app.outl.examples.excalidraw/token: ••••••••
```

It lands in the OS keychain (macOS Keychain, Windows Credential Manager, Linux
Secret Service), namespaced to this plugin. Not in your workspace, not in the
lockfile, not synced with your notes. The plugin only reads it, and only to put
it in an `Authorization` header on a request to `api.excalidraw.com`.

Two kinds of key work. A personal key (starts with `uk-`) acts as you and can
see your private scenes; a workspace key cannot. If `excalidraw-scenes` comes
back emptier than you expected, that's usually why.

## Config

Shown by `outl plugin config show app.outl.examples.excalidraw`, set with
`outl plugin config set app.outl.examples.excalidraw <field> <value>`, or
through a client's plugin settings form.

| Field | Meaning | Default |
|---|---|---|
| `theme` | Theme for fences that don't set one. | `auto` |
| `maxHeight` | Tallest a drawing gets before it scales down. | `480` |
| `indexPage` | Page slug `excalidraw-scenes` writes to. | `excalidraw` |
| `token` | The API key. Routed to the keychain, never stored here. | — |

## Which clients draw it

The fence is a `content-transformer:rich` — HTML in a sandboxed iframe — so:

| Client | Fence | Commands |
|---|:---:|:---:|
| Desktop | drawn | ✅ |
| Mobile | drawn | ✅ |
| TUI / CLI | shown as the text you wrote | ✅ |

A terminal has nowhere to put an SVG, so the TUI drops the descriptor and leaves
your fence as written. That is also why the fence body is short, readable
`key: value` lines: on the TUI, `scene: 7Hs9kQmV2` under a heading is still a
usable note.

## What it draws

Rectangles, diamonds and ellipses with their fills (`hachure`, `cross-hatch`,
`solid`), rounded corners, lines and arrows (straight, curved, closed, filled),
every arrowhead Excalidraw offers, freehand strokes, text, images, and frames.
Plus the attributes that go with them: stroke colour and width, dashed and
dotted strokes, opacity, rotation, and the three roughness levels.

Deliberate gaps, each visible in the caption rather than silent:

- **Embeds and iframes** are counted as unsupported, not drawn. A drawing that
  depends on a YouTube embed is not something an offline SVG can show.
- **Past 1200 elements** it stops and says how many it left out. The plugin runs
  in an interpreter with a loop-iteration cap, and hitting that cap would
  render the whole fence as nothing.
- **Images over ~1.5 MB in total** are dropped from the cache and draw as
  labelled placeholders, so a photo-heavy scene cannot turn `storage.json` into
  a hundred-megabyte file the host re-parses every turn.
- **Fonts.** The iframe has no network, so Excalifont and Nunito render only if
  they happen to be installed. The fallbacks keep the character (cursive for the
  hand-drawn fonts), not the exact shapes.

Freehand strokes are drawn as a weighted smooth path rather than through
perfect-freehand's filled outline — right shape, right weight, no pressure taper.

## How it works

Three pieces, and the split between them is the whole design:

```
 ```excalidraw fence ──▶ transformer (sync, no network) ──▶ SVG in an iframe
                               │
                               ▼
                        ctx.storage cache
                               ▲
                               │
 excalidraw-sync ──┬──▶ api.excalidraw.com   (keychain token)
                   └──▶ json.excalidraw.com  (bytes → AES → inflate)
```

**The transformer never fetches.** A content transformer has to return its
descriptor within the call — it cannot `await` — and `ctx.net.fetch` blocks the
one thread every plugin feature shares. A fetch per repaint would stall the app.
So rendering only ever reads the local cache, and pulling is an explicit command.

That split is what makes a synced drawing work offline: once cached, it renders
with no key and no connection. It is also what makes a share link possible at
all — decrypting one takes the better part of a second, which is fine once per
sync and impossible once per repaint.

**The cache holds elements, not a rendered SVG**, because theme and height are
per-fence: two fences can point at one scene and ask for different renderings.
Re-rendering is cheap next to a round trip.

**It lives in `ctx.storage`**, which is per-device and outside the op log. That
is the right home for a projection of someone else's state: throw it away, run
the sync, get it back. Nothing here needs to converge between devices, so
nothing here is modelled as an `Op`.

## Layout

```
excalidraw/
├── plugin.json           # manifest: capabilities, permissions, commands
├── config.schema.json    # the settings form, incl. the keychain-routed token
├── src/
│   ├── index.ts          # activate: registers the transformer + 3 commands
│   ├── fence.ts          # parsing a fence body
│   ├── api.ts            # the Excalidraw+ client (reads only)
│   ├── cache.ts          # ctx.storage wrapper, file budget, inline-image guard
│   ├── commands.ts       # sync / list scenes / clear cache
│   ├── html.ts           # the iframe document: drawing page, message page, CSP
│   ├── escape.ts         # the one XML escaper, and the one string coercion
│   ├── types.ts          # the slice of the Excalidraw format we read
│   ├── share/            # opening an excalidraw.com share link
│   │   ├── index.ts      # the payload layout, and both size ceilings
│   │   ├── aes.ts        # AES-128 in CTR mode (no WebCrypto here)
│   │   ├── inflate.ts    # DEFLATE, with the zlib wrapper peeled
│   │   └── bytes.ts      # base64, big-endian reads, chunk split, UTF-8
│   └── render/
│       ├── scene.ts      # viewBox, z-order, defs, element dispatch
│       ├── rough.ts      # seeded PRNG, jittered paths, double stroke
│       ├── shapes.ts     # rectangle, diamond, ellipse, image, frame
│       ├── linear.ts     # lines, arrows, arrowheads, freehand
│       ├── text.ts       # text elements and font stacks
│       ├── fill.ts       # hachure / cross-hatch as SVG patterns
│       └── attrs.ts      # stroke, dash, rotation, opacity, corner radius
├── test/
│   └── share.test.ts     # the decoder, against payloads built like Excalidraw's
├── index.js              # the shipped bundle (esbuild, iife)
├── README.md
└── LICENSE
```

## Developing

```sh
bun install          # from the repo root; examples/* is a workspace
cd examples/excalidraw
bun run typecheck
bun test             # the decoder suite
bun run build        # writes index.js, which is what ships
```

Install it into a scratch workspace rather than your real notes:

```sh
outl init /tmp/excal-ws
outl -w /tmp/excal-ws plugin install ./examples/excalidraw --yes
outl -w /tmp/excal-ws plugin run app.outl.examples.excalidraw excalidraw-sync
```

`outl plugin install` points at this directory, so **rebuild before you
reinstall** — the bundle is what gets copied, not `src/`.

### Testing

The renderer is covered from three sides, and all three matter:

- **`crates/outl-plugins/tests/excalidraw_bundle.rs`** runs the shipped
  `index.js` through the real Boa engine, the same way a client does. That is
  the only test that proves the code runs *there* rather than in Node — it is
  how the `ctx.storage`-throws-without-`storage:local` case was found.
- **`test/share.test.ts`** (`bun test`) drives the share-link decoder with
  payloads assembled the way excalidraw.com assembles them — Node's zlib is
  the same DEFLATE pako emits, its `aes-128-gcm` the same cipher. Two of its
  cases are refusals rather than round-trips: a compression bomb, and a scene
  whose `dataURL` points at a remote host. Those guards are one line each and
  nothing else fails if they go.
- **Looking at it.** A render that throws no error and draws the wrong picture
  passes every assertion. Point a browser at the HTML the transformer returns
  before you trust a change to `render/`.

```sh
bun test                                            # decoder
cargo test -p outl-plugins --test excalidraw_bundle # the bundle, under Boa
```

Both run in CI: the root `bun run test` fans out over every workspace that
declares the script.

### Adding an element kind

1. Add the fields you need to `ExcalidrawElement` in `src/types.ts` — optional,
   always: the scene on the other side was written by a newer Excalidraw than
   this, and an element it doesn't understand has to count as skipped, never
   crash.
2. Treat every field as hostile. It is JSON somebody else wrote, so a colour
   can arrive as `1` and a `fontFamily` as `"constructor"`; use `asString`
   from `src/escape.ts` rather than `(x || "").trim()`, which throws on both.
   A throw costs the whole descriptor, so one bad field blanks the fence.
3. Write the renderer in `src/render/shapes.ts` (closed) or `linear.ts` (open).
   Fill and stroke are **separate paths**: a sketchy outline is disconnected
   `M…Q` sub-paths and cannot be filled, so build a clean closed path for the
   fill and a jittered one for the stroke. `rough.ts` has both for every shape.
4. Add a `case` to `renderElement` in `src/render/scene.ts`.
5. Extend the scene in `excalidraw_bundle.rs`, and look at the result.

Returning `""` from a renderer counts the element as unsupported and says so in
the caption. That is the correct answer for something you can't draw — a silently
missing element is the failure mode this plugin tries hardest to avoid.

## Troubleshooting

**"Not synced yet"** — the scene isn't cached. Run `excalidraw-sync`.

**"Cache unavailable"** — the plugin is loaded without `storage:local`, so
`scene:` fences can't work. Pasted scenes still do.

**"No Excalidraw+ API key"** — set it with `outl plugin secret set`, above.

**"the API key was rejected"** — the key was revoked, or it's a workspace key
reaching for a private scene. `excalidraw whoami` in the
[official CLI](https://github.com/excalidraw/cli) tells you which kind you have.

**"could not decrypt the shared scene"** — the link is truncated. A share link
is `#json=<id>,<key>` and both halves are needed; copying only up to the comma
leaves nothing to decrypt with.

**"that share link has expired"** — Excalidraw drops shared scenes that go
untouched for long enough. Re-share the drawing for a fresh link.

**"over the 32 KB this can decrypt"** — the scene is too big to decode without
blocking the app. Export it as a `.excalidraw` file and paste it into the
fence; a pasted file skips the decrypt entirely.

**The sync reports failures** — the count is in the notification, the reason per
scene is in the log. A scene deleted in Excalidraw+ keeps its last cached copy
and fails on every sync until you remove the fence.

**Nothing renders on the TUI** — expected; see the client table above.

## License

MIT. See [LICENSE](LICENSE).
