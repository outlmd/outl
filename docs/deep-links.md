# Deep links (`outl://`)

One scheme, one parser in `outl-actions`, and one wiring section per client.
The contract below is the part users and integrators care about; the wiring sections are what a contributor needs before touching a handler.

---

## The contract

External launchers open a client at a specific page or daily note through an `outl://` URL.
The Raycast extension's "Enter → open in app" is the first consumer; links shared into the mobile app are the second.
You can also write one yourself, in a shell alias, an Alfred workflow, a calendar event or a note in another app.

The scheme is tiny and identical on every platform:

| URL | Opens |
|---|---|
| `outl://daily/today` | today's journal |
| `outl://daily/2026-06-25` | the daily for that ISO date |
| `outl://page/<slug>` | the page (slug may nest: `outl://page/ai-agent/learning`) |
| `outl://capture` | the quick capture box over today's journal. Nothing is written until you submit a line |

The `<slug>` is the file name without `.md`, the form [`[[Page Name]]` folds to](concepts.md#slugs): `[[São Paulo]]` is `outl://page/sao-paulo`.
A trailing slash is ignored.
Anything else (another scheme, an unknown kind, a date that is not `YYYY-MM-DD`, a slug with `..` in it) is logged and ignored.
The app does not crash and does not create a stray page.

**An unknown slug creates the page.**
`outl://page/new-idea` opens an empty `new-idea` page, the same as following a `[[new-idea]]` link to a page that does not exist yet.
A link in a web page can therefore add an empty page to your graph.
The link carries no content and no secret, so the worst outcome is clutter.

## Try it

```sh
open "outl://daily/today"          # macOS
xdg-open "outl://page/inbox"       # Linux
start "" "outl://capture"          # Windows (cmd)
```

| Client | Opens deep links |
|---|---|
| **Desktop** | Yes, on macOS, Linux and Windows. A running instance is focused rather than started twice |
| **Mobile (iOS, Android)** | Yes. A link tapped anywhere on the phone opens the app on the target |
| **TUI, CLI** | No. A terminal has no URL handler. `outl page get <slug>` and `outl daily today` are the equivalents |

On macOS the scheme is registered by the installed app bundle, so a build you ran from source with `cargo tauri dev` does not answer `open "outl://…"`.
Install the release app and open it once.

> **Not the same as MCP resource URIs.**
> `outl mcp serve` also names resources `outl://workspace/info`, `outl://daily/today` and `outl://page/<slug>`.
> Those are read by an agent over MCP and return a page rendered as markdown.
> They never go through a client window.
> See [MCP](mcp.md).

## Troubleshooting

| You see | Why | Fix |
|---|---|---|
| Nothing happens on click | The app is not installed, or another app claimed `outl://` and the OS shows a chooser | Pick outl in the chooser, or remove the other app's handler |
| The wrong page opens | The slug differs from the title: the URL takes the slug | Open the page and read its file name, or use `outl page list` |
| An empty page appears | The slug did not exist | Expected, see above. Delete the page or fix the link |
| A date link does nothing | The date is not ISO | Write `outl://daily/2026-06-25`, not `outl://daily/25-06-2026` |

## For integrators

Parsing lives in **one** place: `outl_actions::parse_deep_link` returns a `DeepLinkTarget` (`Today` / `Daily(date)` / `Page(slug)` / `Capture`).
Each client maps that target onto the same `open_*` command its UI already calls (`open_today_journal` / `open_journal_for` / `open_page_by_slug`) and focuses its window.
The parser never touches a `Workspace` — it is pure string → enum — so the desktop and mobile handlers cannot drift on the contract the way two hand-rolled URL parsers would.

A malformed URL (wrong scheme, unknown kind, bad date, path-traversal slug) returns `DeepLinkError`; the client logs it and no-ops.
It must never crash the app or materialise a stray page.

Registration is per-client transport, not shared logic:
the desktop registers the scheme via `tauri-plugin-deep-link` (+ `tauri-plugin-single-instance` so the URL reaches an already-running instance on Linux/Windows);
iOS registers the same scheme by a hand-maintained `CFBundleURLTypes` entry in `gen/apple/outl-mobile_iOS/Info.plist` — *not* through plugin config, which for `deep-link` is desktop-only (see [Mobile wiring](#mobile-wiring-outl-mobile)).
Universal Links (`https://outl.app/…`) are a later addition — they need an Associated Domains entitlement and a hosted `apple-app-site-association`, so the custom scheme ships first.

---

## Desktop wiring (`outl-desktop`)

The desktop registers the `outl://` scheme so external launchers (the Raycast extension, shared links) jump straight to a page or daily note (issue #98).
The scheme contract and shared parser live in `outl-actions` — see [The contract](#the-contract) above — so handlers can't drift.

Wiring (all in `src-tauri/src/lib.rs`):

- **Plugins.**
  `tauri-plugin-single-instance` is registered **first**.
  Its `deep-link` feature forwards an `outl://` URL opened while the app runs to the existing instance on Linux/Windows; the callback just focuses the `main` window.
  `tauri-plugin-deep-link` follows.
  The scheme is declared in `tauri.conf.json` (`plugins.deep-link.desktop.schemes`), granted via `deep-link:default` in `capabilities/default.json`.
- **Warm path** (`dispatch_deep_link`, fired by `on_open_url`) parses the URL with `outl_actions::parse_deep_link` — the one owner, this crate adds no parsing.
  It then **emits** `deep-link://navigate` with one of `{kind:"today"}` / `{kind:"daily",date}` / `{kind:"page",slug}` and focuses the window.
  A malformed URL is logged at `warn` and ignored — never a crash, never a stray page.
- **Cold path** (a URL that *launched* the app) can't emit — the frontend listener isn't mounted yet.
  So `setup()` buffers the parsed payload in a managed `PendingDeepLink(Mutex<Option<Value>>)` instead, and the `take_pending_deep_link` command drains it once on mount.
  Only the launch URL populates the buffer; the warm path never does, so a stale target can't replay on the next plain launch.
- **Frontend.**
  `AppShell` listens via `onDeepLinkNavigate` (`lib/events.ts`) for the warm path.
  On mount it calls `takePendingDeepLink()` (`lib/api.ts`) for the cold path — a buffered target wins over loading today's journal, which would otherwise race and overwrite it.
  Both map onto the same `openTodayJournal` / `openJournalFor` / `openPageBySlug` commands the picker already calls, then `applyView`.
  The backend, not the frontend, owns parsing + window focus.

**Testing on macOS needs a bundled, installed app.**
macOS registers URL schemes only via LaunchServices from the bundle's `CFBundleURLTypes` (written by `tauri-plugin-deep-link` at `cargo tauri build`), so `cargo tauri dev` does **not** register `outl://`.
To test: `cargo tauri build`, copy the `.app` into `/Applications`, open it once so LaunchServices indexes it, then `open "outl://page/<slug>"`.
Linux/Windows register at runtime (`register_all()` in `setup`), so dev mode works there.

---

## Mobile wiring (`outl-mobile`)

The mobile app registers the `outl://` scheme so links shared into it (or the Raycast extension on the same Mac, once Handoff is in play) open a specific page or daily note (issue #98).
The scheme contract and the shared parser live in `outl-actions` — see [The contract](#the-contract) above — so the mobile and desktop handlers can't drift.

Wiring:

- **Plugin.**
  `tauri-plugin-deep-link` is registered in `lib.rs`'s builder.
  No single-instance plugin — iOS is single-instance by construction, so the OS routes the URL to the running app.
- **Scheme registration is the iOS `Info.plist`, not config.**
  Tauri's `plugins.deep-link.desktop.schemes` key is desktop-only.
  For an iOS **custom scheme** the `CFBundleURLTypes` entry is added directly to `gen/apple/outl-mobile_iOS/Info.plist`, alongside the existing `UIBackgroundModes` / iCloud keys this project already hand-maintains there.
  Universal Links (`https://outl.app/…`) would need the `mobile` config + Associated Domains + a hosted `apple-app-site-association` — a separate follow-up.
- **Warm path** (`dispatch_deep_link`, on `on_open_url`) mirrors desktop: parse via `outl_actions::parse_deep_link`, emit `deep-link://navigate` (`today`/`daily`/`page`), focus the window.
  A malformed URL is logged at `warn` and ignored.
- **Cold path** (a URL that *launched* the app) buffers the parsed payload in a managed `PendingDeepLink(Mutex<Option<Value>>)` during `setup()`, because the frontend listener isn't up yet.
  The `take_pending_deep_link` command drains it once `Journal` mounts.
  Same shape as the desktop buffer; only the launch URL populates it.
- **Frontend.**
  `Journal.tsx` registers `listenForDeepLink()` in `onMount` (warm) and, right after `loadTodayWithRetry`, drains `take_pending_deep_link` (cold).
  Both call the shared `navigateDeepLink` helper, which maps onto the same `openTodayJournal` / `openJournalFor` / `openPageBySlug` commands the ref-tap path uses, then `applyView`.
  The warm listener skips while a block is being edited (`editingId()` guard) so it never resets the textarea mid-edit.
  The cold drain runs after the workspace is open, so it overrides today's journal with the launch target.

**Validation needs a device build.**
The Rust side is `cargo check`-clean, but scheme registration + OS routing only exercise on a real device / simulator build (`cargo tauri ios dev`), same constraint as `BGTaskScheduler` / `NSMetadataQuery`.
Don't mark the mobile half "verified" from a host `cargo check` alone.

### Android registration

Android is the third place the scheme is spelled, and it is neither config nor a plist: it is an `intent-filter` on `MainActivity` in `gen/android/app/src/main/AndroidManifest.xml`.

```xml
<intent-filter>
  <action android:name="android.intent.action.VIEW" />
  <category android:name="android.intent.category.DEFAULT" />
  <category android:name="android.intent.category.BROWSABLE" />
  <data android:scheme="outl" />
</intent-filter>
```

Two things follow from it:

- **The capture widget and the quick settings tile depend on it.**
  Both write nothing; they fire `outl://capture` through `startActivity`, so without the filter they open nothing at all.
  That is also why they could ship before the inbox existed (RFC 0363 step 1).
- **`BROWSABLE` means a web page can reach it.**
  A tapped `outl://page/<slug>` link resolves through the same `parse_deep_link`, and an unknown slug **creates** the page (`open_or_create_by_name`), so a link can add an attacker-named page to the graph.
  The category is kept for parity with iOS, where Safari reaches a registered custom scheme the same way, rather than because the widget or tile need it — they are in-process.
  If that exposure is ever unwanted, drop `BROWSABLE` first: the widget and tile keep working.

`android:scheme="outl"` has no host or path restriction, so every target in [The contract](#the-contract) resolves, `outl://capture` included.
Android does not guarantee scheme uniqueness — another installed app can claim `outl://` and show a chooser — but the URL carries no secret and no content, so the worst case is the capture sheet not opening.
