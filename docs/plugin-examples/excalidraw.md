# Excalidraw

> **Capabilities:** `content-transformer:rich` + `slash-command` + `toolbar-button` + `config-schema`, with the `network` (two domains), `secrets` and `storage:local` permissions · [Source on GitHub](https://github.com/outlmd/outl/tree/main/examples/excalidraw)

Renders a ` ```excalidraw ` fence as the actual drawing — inline SVG with hand-drawn strokes, hachure fills, arrows and text.
The scene is either a `.excalidraw` file pasted into the fence, or an [Excalidraw+](https://plus.excalidraw.com) scene id pulled over the public API and cached locally.

The other examples each isolate one capability.
This one is the opposite: it is the **largest example in the repo**, and it is here because a real integration is where the capabilities start constraining each other.
Using it as a template is the point; [its README](https://github.com/outlmd/outl/tree/main/examples/excalidraw#readme) is the user-facing documentation, and this page is why it is built the way it is.

## What it demonstrates

**A transformer cannot fetch, so the fetch is a command.**
`ctx.content.register`'s function must return its descriptor *within the call* — the host reads the return value of `__outl_dispatchTransform` and only then drains the job queue, so a `Promise` is not a descriptor.
`ctx.net.fetch` is also blocking on the one thread every plugin feature shares, so a fetch per repaint would stall the app rather than merely being slow.
The shape that falls out: `excalidraw-sync` pulls over the network and writes `ctx.storage`, and the transformer only ever reads that cache.
Any plugin that wants to render remote data lands on this same split.

**A cache belongs in `ctx.storage`, not in an `Op`.**
Root invariant 7 says state that must converge goes through the op log.
This state must not: it is a projection of someone else's system, rebuildable by running the sync again, and different per device.
`storage:local` is the right home precisely because it does *not* converge.

**`ctx.storage.get` throws, and a transformer that lets it throw renders nothing.**
Without `storage:local` the call raises, an uncaught throw costs the whole descriptor, and the block then draws an empty box while saying nothing about why.
`src/cache.ts` catches it and returns a three-way result — hit, missing, unavailable — because "run the sync" and "a permission is missing" are different instructions for the user.
Writing this example is also what surfaced the host-side half of that bug: `transform_block`, `sync_push` and `sync_pull` set up `ctx.secrets` but never `ctx.storage`, so the call threw even *with* the permission granted.
They now go through one `prepare_turn` / `finish_turn` pair (`crates/outl-plugins/CLAUDE.md` → Execution model).

**Reading a *binary* response.**
An excalidraw.com share link is an AES-GCM cipher text on a public backend, with the key in the URL fragment.
A response body crosses into JS inside a JSON string, so the default text read replaces every byte that is not valid UTF-8 with U+FFFD and corrupts it silently — which is why `ctx.net.fetch` grew `encoding: "base64"` ([Plugin API](../plugin-api.md#host-api--plugincontext)).
`src/share/` then does what the web app does with WebCrypto and pako: AES-128 in CTR mode and a DEFLATE decoder, in plain JS, because the plugin runtime has neither.
Worth reading as the worked example of **what "pure JS on an interpreter" costs**: `aes.ts` keeps its state in sixteen locals rather than a `Uint8Array` because the array version measured 400ms on 2.3 KB, and `bytes.ts`'s `utf8` builds its string with `+=` because the chunked `String.fromCharCode.apply` version measured *slower*.
Both carry the numbers in a comment, so the next person optimises from a measurement instead of an instinct.
The decoder also declines a payload over 32 KB rather than blocking the plugin thread for a minute.

**A token lives in the keychain.**
`config.schema.json` marks `token` with `"x-outl-secret": true`, so a client's settings form routes it to the OS keychain instead of the lockfile, and the plugin reads it with `ctx.secrets.get("token")` only to put it in an `Authorization` header.
See [Plugin API → Secrets, the full flow](../plugin-api.md#secrets--the-full-flow).

**The network grant is a constant, not config.**
`network:api.excalidraw.com` is what the user approved, so the API origin is hardcoded next to it.
A configurable `apiUrl` would promise a self-hosted instance the host then refuses to connect to — the permission is the contract, and config cannot widen it.

**A scene is data from a stranger, and the renderer is a sink.**
The drawing is built by string concatenation and handed to the client as `rich`, which injects it with `srcdoc` into a sandboxed iframe — and `sandbox="allow-scripts"` does *not* block the network, while the desktop ships `csp: null`.
So a scene's `dataURL` field, which is a free string, reached an `<image href>` and would have made the webview fetch it on every repaint: a read receipt and an IP for whoever shared the scene, through a request the `network:` allowlist never sees because it never goes through `ctx.net.fetch`.
`isInlineImage` restricts it to `data:image/`, and the page declares its own CSP rather than trusting the host's.
The same reflex applies to types: a colour can arrive as `1` and a `fontFamily` as `"constructor"` (which resolves through `Object.prototype`), and `(x || "").trim()` throws on both — a throw the host reads as "no descriptor", so one bad field blanks the whole fence.
And a size ceiling on a compressed payload says nothing about the decompressed one: DEFLATE reaches 1029:1, so 32 KB of input is 32 MB of output, which is minutes of inflating on the plugin thread.
Both ceilings exist, and `test/share.test.ts` keeps each guard honest with a refusal case.

**What it cannot draw, it says.**
Elements past the renderer's cap, element kinds it has no drawing for, and images dropped from the cache all appear in the caption under the drawing.
A render that silently omits part of a scene looks exactly like a complete one, and the user would only find out by comparing against Excalidraw.

## The fence

````text
```excalidraw
scene: 7Hs9kQmV2
theme: dark
height: 320
```
````

or, with no key and no network, a pasted `.excalidraw` file:

````text
```excalidraw
{"type":"excalidraw","elements":[ … ]}
```
````

An excalidraw.com `#json=` share link is refused with an explanation: those scenes are end-to-end encrypted with the key in the URL fragment, and the plugin has no primitives to decrypt one.

Full syntax, commands, config and troubleshooting live in the [plugin's README](https://github.com/outlmd/outl/tree/main/examples/excalidraw#readme).

## The code

Too large for one page — twelve modules, most of them an SVG renderer.
The layout and what each module owns is in the README; the pieces worth reading for plugin-authoring reasons are:

| File | Why it is interesting |
|---|---|
| `src/index.ts` | `activate`: one transformer, three commands, and the branch between cache states |
| `src/cache.ts` | the synchronous read a transformer needs, and the throw it has to catch |
| `src/commands.ts` | one list request plus one content request per scene, and a re-runnable `appendTree` |
| `src/api.ts` | a client that returns refusals instead of throwing, because `ctx.net.fetch` does |
| `src/render/rough.ts` | a seeded PRNG, so a drawing does not flicker on every repaint |

## Try it

```sh
outl -w <workspace> plugin install ./examples/excalidraw --yes
# paste a .excalidraw file into a ```excalidraw fence and open the page on desktop/mobile
```

For the `scene:` form, add a key and sync:

```sh
outl plugin secret set app.outl.examples.excalidraw token
outl -w <workspace> plugin run app.outl.examples.excalidraw excalidraw-scenes
outl -w <workspace> plugin run app.outl.examples.excalidraw excalidraw-sync
```

`rich` is GUI-only, so the fence draws on desktop and mobile; the TUI shows the text you wrote.
