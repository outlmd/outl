# Markdown Format

The outl markdown dialect, the sidecar `.outl` format, and the 3-level matching algorithm that bridges them.

## Why this matters

The user sees the `.md`.
The op log uses stable IDs.
We need both, without visible metadata in the file.
Every design decision here serves that.

> **Why the dialect looks like this, and what it costs to add a token to it:** [RFC 0008](rfcs/0008-markdown-dialect-and-sidecar-tokens.md).

---

## The .md file

```markdown
title:: meu projeto x
type:: project
status:: active
tags:: #produto #2026-q2
created:: [[2026-05-24]]

- objetivo principal #okr
  priority:: high
  owner:: [[avelino]]
  - métrica: 30% redução de custo
  - prazo: [[2026-06-30]]
- riscos
  - dependência de [[fornecedor y]] #risco
```

That is the **entire file**.
No IDs, no UUIDs, no HTML comments, no frontmatter delimiter.
What you see is what's on disk.

### Page properties (top of file)

Lines at the start of the file in the form `key:: value` are page-level properties.
Parsing stops at the first blank line or first `-` outline item.

```
title:: my page
type:: project
status:: active

- first block...
```

The `type::` key carries the page's semantic kind and is consumed by surfaces that filter pages by role — `type:: person` is the canonical example, recognised by the `@` mention autocomplete (see [§Mentions](#mentions-name)).
Other types (`type:: project`, `type:: meeting`, …) are free-form today; the autocomplete only filters on `person`.

### Outline items

Standard markdown unordered lists:

```
- top level block
  - child block
    - grandchild block
- another top level
```

Indent is **two spaces** per level.
Tab is normalized to two spaces on parse and rendered as two spaces.

### Multi-line block text (continuation lines)

A bullet's text can span multiple lines.
Subsequent lines are indented **one level deeper** than the bullet and contain no marker of their own:

```
- first line of the block
  second line of the same block
  third line
```

Parsed as a single block with `text = "first line of the block\nsecond line of the same block\nthird line"`.
Renders back identically.

**A blank line inside the text is not a separator — if it carries the indent.**
While continuation is still open, a blank line indented one level deeper than the bullet stays part of the block's own text (a blank line the user typed inside a multi-paragraph block).

The indent is what distinguishes the two cases, and it is invisible in a code fence, so `·` stands for a space below:

```
-·briefing
··paragraph·one
··
··paragraph·two
-·next·block
```

Parses to one block with `text = "briefing\nparagraph one\n\nparagraph two"`, and `next block` as its sibling.

**A genuinely empty line (no indent at all) ends continuation**, and that is not a detail — it is the whole rule:

```
-·briefing
··paragraph·one

··paragraph·two
-·next·block
```

Here the empty line closes the block, and `paragraph two` is recovered as a child block with a warning rather than folded into the text above it.
Nothing is lost either way; what changes is where the line lands.

This is what the renderer emits, so the round-trip holds by construction: `render` writes every line of `text` after the first at `indent + 1`, and an empty line in the text becomes an indented blank one.

**A continuation line's own indentation survives.**
The renderer writes each continuation line one level deeper than the bullet.
A line whose own text is itself indented (`"head\n  detail"`) comes back one level deeper still on parse, and only that extra level is stripped — so nested indentation inside a block's text round-trips verbatim instead of flattening.

Continuation **ends** at the first line that is a block marker (`-`), a property (`key:: value`), a genuinely empty line, or once a child block has claimed the slot after one.
After that, a line the grammar still can't place — over-indented, no open continuation to absorb it — is recovered as a **child block** at the depth it was written (see "Permissive parsing & warnings" below).
Nothing is silently skipped, at any depth.

In the TUI: `Alt+Enter` (or `Ctrl+J`, or `Shift+Enter` in kitty-protocol-aware terminals) inserts a soft newline inside the current block.
Plain `Enter` commits and creates a sibling — unless the cursor is inside an open fenced code block, in which case `Enter` auto-detects and inserts a soft newline instead (see below).

### Block-level prefixes (TODO / DOING / DONE / quote)

A block's *kind* (open task, task in progress, completed task, blockquote) is encoded as a **text prefix** on the bullet's body, not as a separate AST field.
This keeps the wire format stable, round-trips through any CommonMark renderer, and lets a user drop the marker by erasing the prefix in their editor.

| Prefix | Meaning | Helpers |
|---|---|---|
| `TODO ` | Open task | `outl_actions::todo::TODO_PREFIX` / `split_todo` / `cycle_todo` |
| `DOING ` | Task somebody has started | `outl_actions::todo::DOING_PREFIX` / `split_todo` / `cycle_todo` |
| `DONE ` | Completed task | `outl_actions::todo::DONE_PREFIX` / `split_todo` / `cycle_todo` |
| `> ` | Blockquote (CommonMark-compatible) | `outl_actions::quote::QUOTE_PREFIX` / `split_quote` / `toggle_quote` |

```markdown
- TODO write the RFC
- DOING ship the parser
- DONE read the paper
```

The toggle chord (`Ctrl+T` in the TUI, `⌘T` on desktop) walks one stop per press: `(none) → TODO → DOING → DONE → (none)`.

**The CommonMark checkbox is a second spelling of the same three states**, accepted on read ([issue #230](https://github.com/outlmd/outl/issues/230)):

| You type | outl reads it as |
|---|---|
| `- [ ] buy milk` | `TODO` |
| `- [/] buy milk` | `DOING` |
| `- [x] buy milk` / `- [X] buy milk` | `DONE` |

A block written that way is a task everywhere a `TODO ` block is one: it draws a checkbox, answers `status:` queries, counts in the progress chip, and toggles.

Two things to know about it:

- **Only the word form is ever written back.**
  The first toggle rewrites `[ ] buy milk` into `DOING buy milk`, and it does not return to bracket form.
  Until you act on the block, the bytes on disk stay exactly as typed — recognition alone never edits the file.
- **The trailing space is what separates a checkbox from a link.**
  `[x](https://example.com)` is a markdown link whose anchor text is `x`, and it stays a link.
  `[]`, `[ ]` alone, and `[y] foo` are prose.

Rules:

- One space follows the marker.
  `>foo` (no space) is **not** a quote — same CommonMark rule that decides `>foo` is a literal paragraph and `> foo` is a blockquote.
  The same rule makes `DOINGs are piling up` prose, not a started task.
- **`DOING ` is one character wider than the other two.**
  Anything doing cursor math around the marker must measure the prefix it is adding or removing (`TodoState::prefix`), never assume five.
- Progress counters (the TUI's `●● 3/7` chip) count `DOING` toward the **total**, never toward the done half — a started task is unfinished work.
- Markers compose in a **canonical order**: `"TODO > body"` / `"DONE > body"` (TODO/DONE before the quote marker).
  `toggle_quote` and `cycle_todo` peel both prefixes off and re-emit in canonical order, so a user who authors them in the other order (`"> TODO foo"`) gets normalised on the next toggle.
  Why canonical order matters: the backend's `split_todo` reads from the **start** of the text.
  A `"> TODO foo"` would surface in the `OutlineNode` DTO as `todo = null` and the literal `"> TODO foo"` in `text` — checkbox disappears mid-flight on mobile / desktop.
  The TUI's `split_block_prefixes` still accepts either order for *display* (so externally authored `.md` reads correctly), but every mutation in `outl-actions` emits the canonical form.
- Children of a quoted block are **not** implicitly quoted — the marker lives on the block, not on its subtree.
  Same policy as TODO/DONE.
- Multi-line quote bodies keep the `> ` on every continuation line so the file stays a valid CommonMark blockquote when an external tool opens it.

Single-line:

```
- > the only way to do great work is to love what you do
- regular block
```

Multi-line:

```
- > quote line one
  > quote line two
  > quote line three
- next regular block
```

Inline tokens (`**bold**`, `[[ref]]`, `#tag`, `((blk-…))`) tokenize **inside** the quoted body — the wrapper is transparent to the inline tokenizer.

### Fenced code blocks inside a bullet

CommonMark code fences are preserved literally:

```
- intro paragraph
  ```lisp
  (+ 1 2)
  ```
- next bullet
```

The opening ` ``` ` may live on the bullet line itself (`` - ```lisp ``) — parser, renderer, and the [`outl-exec`] engine all handle that shape correctly.

What makes fences different from regular continuation:

- Content between the opener and closer is preserved **verbatim**.
  No `-`, no `key::`, no inline syntax recognition.
- The closer is a line whose trimmed content is exactly `` ``` `` (with optional trailing backticks) at the same indent as the opener.
- A missing closer is gracefully synthesized at EOF so the rendered output stays well-formed; the parser also breaks out of a fence when a sibling bullet outdents below the fence indent — better than swallowing the rest of the document.

[`outl-exec`]: ../crates/outl-exec/

#### Language tag aliases

The opening fence's info-string (`` ```rs `` / `` ```javascript `` / `` ```py3 ``) gets canonicalised through a single shared alias table — [`outl_md::lang::canonical`] in Rust, [`@outl/shared/highlight::canonical`] in TypeScript.
Both layers use the same canonical names so what runs at the backend, what gets syntax-highlighted in the desktop / mobile editor, and what the user types in the fence all line up.

A handful of the common aliases:

| You write | Resolves to | Notes |
|---|---|---|
| `js`, `javascript`, `node`, `nodejs` | `js` | Maps to the `js` runtime in `outl-exec`. Before the alias table, `` ```javascript `` failed with "no runtime registered". |
| `rs`, `rust` | `rust` | |
| `py`, `python`, `python3` | `python` | |
| `sh`, `bash`, `zsh` | `shell` | Highlight only — no runtime yet. |
| `yml` | `yaml` | |
| `md` | `markdown` | |
| `c++`, `cxx`, `cc`, `cpp` | `cpp` | |
| `cs`, `c#` | `csharp` | |
| `query`, `tasks` | `query` | Maps to the `query` runtime — workspace queries as code blocks (see [Query code blocks](#query-code-blocks) below). |

The full table lives in `crates/outl-md/src/lang.rs::KNOWN_ALIASES`; the TS mirror is `crates/outl-frontend-shared/src/highlight/aliases.ts`.
Add a row in both files in the same commit — the `doc-sync-guard` hook treats this as a shortcut-level change and refuses the edit otherwise.

#### Syntax highlighting (desktop + mobile)

`outl-desktop` and `outl-mobile` both render code fences in read mode through the shared `<HighlightedCode />` component.
It lazy-loads [`highlight.js`'s "common" bundle][hljs-common] (~30 popular languages, ~80 KB) and applies the brand palette defined in `crates/outl-frontend-shared/src/highlight/styles.css`.

Unknown / empty languages fall back to a plain `<pre>` with the brand-dark canvas — we never use highlight.js's `"auto"` detection because the misclassification cost (Bash highlighted as Perl) is worse than visual flatness.

The TUI renders fences as monospace text without syntax colouring today; the planned approach when this lands is `syntect` keyed on the same canonical names from `outl_md::lang`.

[`outl_md::lang::canonical`]: ../crates/outl-md/src/lang.rs
[`@outl/shared/highlight::canonical`]: ../crates/outl-frontend-shared/src/highlight/aliases.ts
[hljs-common]: https://github.com/highlightjs/highlight.js/blob/main/src/index.js

#### Query code blocks

A ` ```query ` fence runs a declarative DSL against the workspace and renders matching blocks as **live embed references**.
Query blocks auto-run on every page load — no `gx` or `auto-run::` needed.

Full syntax reference, examples, and architecture: [Query code blocks](query.md).

### Block properties

A line in the form `key:: value` *as a child of an outline item* is a block property:

```
- objective
  priority:: high
  owner:: [[avelino]]
  - this is a regular child block
```

`priority::` and `owner::` are properties of `objective`, not children.
The third line (`- this is a regular child block`) is a real child.

#### `remind::` — the one property the parser validates

Every other `key:: value` is opaque to the parser. `remind::` is not: it carries a notification rule with its own grammar, so the parser checks it as it reads.

```
- TODO #fup [[@joão]] about project abc [[2026-12-12]]
  remind:: 3pm every 1h until DONE
```

A rule the grammar can't read **never** costs you the property or the block — the line stays on disk verbatim, and the only consequence is that it doesn't schedule.
The recovery is reported as a `ParseWarning` carrying the exact source line (`remind_missing_anchor`, `remind_invalid_time`, `remind_invalid_interval`, `remind_invalid_stop`, `remind_max_clamped`), so the parse banner and `outl doctor` can point at it.

Full syntax, defaults, quiet hours, and which clients deliver: [Reminders](reminders.md).

### Inline syntax

| Syntax | Meaning |
|--------|---------|
| `[[name]]` | Reference to page named "name" |
| `[[2026-05-24]]` | Reference to journal "2026-05-24" (rendered as date) |
| `[[@name]]` | Mention — reference to the person page `name` (page-level `type:: person`); the `@` is the link affordance, not part of the page identity |
| `#name` | Tag (page reference with classification semantics) |
| `((blk-XXXXXX))` | Block reference — renders as the source block's text, links to it |
| `!((blk-XXXXXX))` | Block embed — renders the source block expanded with its subtree |
| `![alt](url)` | Image / embedded asset — renders inline (`<img>` on desktop/mobile, a `🖼`/`📄` placeholder in the TUI); `url` is a workspace-relative `assets/<hash>.<ext>` path or a remote URL. See [Asset links](#asset-links-nameassetshashext) |
| `:shortcode:` | GitHub gemoji shortcode — renders as the unicode glyph (`:tada:` → 🎉) |
| `{{query: ...}}` | Inline query token (legacy — parsed as opaque; use ` ```query ` code blocks instead, see [Query code blocks](#query-code-blocks) below) |
| `**bold**`, `*italic*` / `_italic_`, `~~strike~~`, `` `code` `` | Standard CommonMark (underscore emphasis rules apply — see below) |
| `==highlight==` | Highlight — renders the inner text marked (the on-disk form of Roam's `^^highlight^^` after import). The inner span may not begin or end with a space, so a spaced comparison (`a == b`) stays plain text |

#### Underscore emphasis and intra-word identifiers

`_italic_` works when the underscores are at word boundaries (surrounded by whitespace or punctuation).
An underscore **inside** a word does not open or close emphasis — it is rendered literally.

Examples that stay plain (no italic):

```
chamados_chat
inc_lag1
prod.ml_atendimento
databricks_2_train
```

Use `*italic*` if you need emphasis inside or adjacent to a word-like token.
`_italic_` in isolation (word-boundary underscores) still works.

This follows the CommonMark spec and is enforced by `try_italic_under` / `try_bold_under` in `outl-md::inline` via the `closing_underscore` helper.

#### Block refs and embeds

`((blk-XXXXXX))` is an inline reference to another block.
The handle is short, lowercase, and human-typeable — it's the last 6 Crockford base32 characters of the block's ULID, prefixed with `blk-`.
Renderers resolve the handle through the workspace index and display the source block's text in place; the on-disk `.md` keeps the literal `((blk-XXXXXX))`.

`!((blk-XXXXXX))` is the embed form.
Same lookup, but the consumer expands the source block **and its subtree** inline.
Mirrors the markdown image syntax (`![alt](url)` → "expand") so the `!` reads as the visual hint for inflation.

```
- decide which database to use #decision
- in [[Project X]], see ((blk-r6s4a1)) for context
- the whole thread: !((blk-r6s4a1))
```

Handles are persisted in the sidecar (see [§sidecar](#the-outl-sidecar)) so a future change to the derivation scheme cannot break references already living in `.md` files.
An orphaned handle (citation points at a block that no longer exists) renders dimmed in the TUI and is flagged by `outl doctor`.

Handle collisions are vanishingly unlikely — 6 lowercase base32 chars is ~30 bits, ~5×10⁻⁶ birthday probability at 100k blocks.
When two blocks do land on the same base handle, the second block's handle is lazily expanded one character at a time (from the ULID's Crockford base32 tail) until unique within the workspace.
Both the winner and the loser stay resolvable through their own (distinct) handles.
The on-disk sidecar still records the deterministic 6-char handle — the divergence lives in memory until a future reconcile rewrites it.
Workspaces that ever expanded a handle to 7+ characters keep working forever because lookup goes through the in-memory handle, not the literal sidecar field.

#### Asset links (`[name](assets/<hash>.<ext>)`)

> **Why asset bytes are content-addressed blobs and deliberately outside the op log:** [RFC 0202](rfcs/0202-file-assets.md).

An uploaded file is copied into `<workspace>/assets/` under its content hash and referenced from a block.
`outl asset add`, the MCP `outl_asset_add` tool, and the desktop/mobile "Attach file" action all do this.
So does dropping a file onto an outline row (desktop, and mobile on iPad) or pasting a dropped file's path in the TUI's Insert mode — see [clients.md → Attach / drag-and-drop file import](clients.md#attach--drag-and-drop-file-import).

The reference takes one of two forms, chosen by the file kind:

```markdown
- the diagram: ![diagram.png](assets/9a8b7c...d6e5.png)   ← image → embed form, renders inline
- see the spec: [proposal.pdf](assets/1b2c3d...e4f5.pdf)   ← other file → plain link
```

- **Images** (`png`, `jpg`, `jpeg`, `gif`, `webp`, `svg`, `bmp`, `avif`, `ico`, `tiff`, `tif`) use the **embed form `![alt](url)`** and render **inline**.
  Desktop and mobile show an `<img>`, the TUI shows a `🖼 alt` placeholder (a terminal can't paint pixels).
- **Every other file** (PDF, anything) uses the **plain link `[name](url)`** and renders as a **file chip** (`📄 name`); activating it opens the file in the OS default app (TUI `g x`, desktop/mobile tap).

The importers (`outl import roam|logseq|obsidian`) apply the same rule: an imported image lands as `![…]` and renders inline, while other imported files stay `[…]` links.
The filename stem is the hex SHA-256 of the file's bytes, so re-uploading identical content reuses the same file and reference everywhere.
A file over `[assets] max_bytes` in `config.toml` (default 100 MiB) is rejected before the copy; see [Configuration](config.md).

Inline image rendering resolves the `assets/<hash>.<ext>` path against the workspace root on the client and loads the bytes through the backend (no external network fetch for a local asset); a remote `http(s)` image `url` is loaded directly.
A `![…]` whose target is **not** an image extension (e.g. `![notes](assets/x.pdf)`) degrades to the same file chip as the plain-link form, so no reference is ever left unrendered.

Asset bytes are **not** workspace state and never enter the op log — only the link text does, as an ordinary block edit.
The file itself replicates like the `.md` projections: the file transport (iCloud/shared filesystem) carries it for free, and the iroh transport ships it over a dedicated `outl-asset/1` stream (see [Sync](sync.md)).

#### Emoji shortcodes (`:tada:`)

`:shortcode:` is the GitHub / Slack / Discord / Logseq convention.
The catalog is GitHub gemoji (~1800 shortcodes), backed by the [`emojis`](https://crates.io/crates/emojis) crate.

```
- shipped the new build :tada: :rocket:
- meeting moved to friday :calendar:
- need to fix this :fire: :warning:
```

**Disk form is the shortcode, never the glyph.**
The `.md` always stores `:tada:`; only the renderer translates to 🎉 at display time.
Same discipline as IDs (invariant #2): the file stays greppable, diffable, and font-independent across devices.

Clients **never** retro-translate a pasted unicode codepoint back into a shortcode — multiple shortcodes can alias the same codepoint (`:+1:` and `:thumbsup:` both → 👍) and the round-trip would be lossy.
A user who wants the shortcode form types it through the autocomplete (`:roc` + Tab → `:rocket:`).

Catalog gate:
the parser only tokenizes `:foo:` when `outl_md::emoji::shortcode_to_unicode("foo")` returns a glyph.
Unknown runs (`:notarealemoji:`, `meeting at 14:00 :`) stay plain — there is no "looks-like-a-shortcode" fallback.

Shortcode shape: `[a-z0-9_+-]+`, single token.
Covers `:+1:`, `:-1:`, `:smile_cat:`, `:100:`.
This narrow alphabet is also what makes URL boundaries safe without look-behind.
Every URL fragment that contains `:` (`https://example.com:8080/api`, `mailto:foo@bar.com`, `git@github.com:avelino/outl.git`) either has an invalid char inside the candidate run (`/`, `.`, `@`) or no closing `:` — `try_emoji` bails on its own.

#### Mentions (`[[@name]]`)

`[[@name]]` is **not a new token**.
It is a regular page reference whose target literal happens to start with `@`.
Roundtrip, render, matching, and reconciliation flow through the same code path as `[[ref]]`, no separate parser branch.

What makes it act as a mention is policy on top of the existing primitive:

- **Page identity does not carry the `@`.**
  The page is `pages/<slug>.md` with `title:: <name>` (no `@`).
  The `@` belongs to the link affordance, like the `!` in `!((blk-XXXXXX))` belongs to the embed affordance.
- **Reference resolution strips the `@`.**
  Opening `[[@avelino]]` resolves to the page `avelino` via the same decision tree as any `[[ref]]` (slug match → slugified match → case-insensitive title match → create).
  The `@` is consumed by the resolver before the lookup.
- **Create-on-miss marks the new page as a person.**
  When the target doesn't exist yet, the resolver creates `pages/<slug>.md` and sets `type:: person` on it automatically.
  The next mention surfaces the page in the autocomplete popup without the user editing properties by hand.
- **Backlinks recognise the `@`-aliased form.**
  A person page's backlinks panel surfaces blocks that wrote `[[@name]]` even though the page slug is `name`.
  Plain (non-person) pages do not scan the `@`-aliased form, so `[[@projeto]]` in a block does not accidentally show up under a non-person `projeto` page.

#### Autocomplete trigger

While the user is editing a block, every client opens a person picker on a **word-initial `@`** (the same word-initial rule the `#` tag trigger uses: `@` preceded by start-of-line or whitespace).
The popup lists pages where `type:: person` is set, fuzzy-matched against whatever was typed after the `@`.
Accepting a candidate inserts `[[@<title>]]` at the caret.

- Composite names work.
  `@Thiago Avelino` is a valid query — the autocomplete query allows spaces, unlike `#tag` which terminates on the first non-word character.
- Mid-word `@` is ignored.
  `a@b.com` (an email) is not a mention.
- When no existing person matches the query, the popup offers the query itself as a "create new" candidate so a fresh mention can be minted without leaving the keyboard.

The `type::` page-level property is what scopes the autocomplete candidate list (and marks the page semantically).
The `@`-prefixed link text is what makes the rendered reference visually a mention.

### What is **not** in the file

- ❌ `id::` lines (Logseq-style) — IDs go in the sidecar
- ❌ `<!-- block-uid: ... -->` — no HTML comments for metadata
- ❌ YAML frontmatter (`---`) — page properties use `::` syntax instead
- ❌ `\`\`\`outl` fenced metadata blocks

### Permissive parsing & warnings

A hand-written or imported `.md` may contain lines that don't fit the dialect — typically a leading `# heading`, a paragraph, an HTML snippet, or a table.
The parser is **permissive at every depth, not just the top level**.
Such a line is preserved verbatim as a recovered block — a sibling at depth 0, a child of the block above it when indented with no open continuation to absorb it — and the recovery is recorded in `ParsedPage.warnings: Vec<ParseWarning>`.

Each warning carries:

- `line` — 1-based source line number.
- `raw` — the offending line, verbatim (no trim).
- `kind` — currently `UnrecognizedBlockMarker` (line didn't start with `- ` and isn't a recognized property).

Two consequences worth knowing:

- **No silent data loss, at any depth.**
  Open a file, save it, the content survives — including a line nested inside another block's continuation that the grammar can't place.
  Render is still clean: blocks created from recovered lines render as `- <raw>\n`, so the next save normalizes the file to the dialect.
- **Surfaces show the warnings.**
  The TUI banner, the mobile / desktop overlay, and `outl doctor` all read `ParsedPage.warnings` and present them as actionable hints (line number + first 60 chars of the raw text).
  Users can edit the file at their pace; outl never blocks on a "dirty" page.

---

## The .outl sidecar

For every `pages/foo.md` there is `pages/foo.outl` (sibling file, not a dotfile).
The dotted form was abandoned in v0: iCloud Documents silently skips paths starting with `.` when syncing across devices, so a dotted sidecar never reached peers and "sync" appeared to lose block IDs.
Same rule keeps the op directory at `ops/` rather than `.ops/`.

Format: JSON.

```json
{
  "version": 2,
  "page_id": "01HXY8KJZQ9T8M7VN3P2R6S4A0",
  "last_synced_hash": "sha256:e3b0c44298fc1c14...",
  "last_synced_at": "2026-05-24T11:22:00-03:00",
  "blocks": [
    {
      "id": "01HXY8KJZQ9T8M7VN3P2R6S4A1",
      "line": 7,
      "indent": 0,
      "content_hash": "sha256:abc123...",
      "ref_handle": "blk-r6s4a1",
      "text": "decide the storage backend"
    },
    {
      "id": "01HXY8KJZQ9T8M7VN3P2R6S4A2",
      "line": 8,
      "indent": 1,
      "content_hash": "sha256:def456...",
      "ref_handle": "blk-r6s4a2",
      "text": "JSONL first, ChronDB later"
    }
  ]
}
```

> The sidecar is **structural matching metadata only** — block id, position, content hash, ref handle, last-synced text.
> State that must converge between devices (fold flags, etc.) lives in the op log (`outl-core`), never here. iCloud syncs the sidecar with LWW per-file, which would silently drop concurrent writes.

### Fields

- `version`: always present, integer.
  Future migrations check this.
- `page_id`: ULID of the page itself (the top-level container).
- `last_synced_hash`: SHA-256 of the full `.md` file at last sync.
  Used as a fast "did this change?" check.
- `last_synced_at`: ISO 8601 timestamp with timezone, set on last write.
- `blocks`: array, in tree order (depth-first, preorder).
  Each entry:
  - `id`: ULID of the block.
  - `line`: 1-indexed line number in the `.md` at last sync.
  - `indent`: 0 for top-level outline items, 1 for first child, etc.
  - `content_hash`: SHA-256 of the block's **textual content only**, not including children or property lines that belong to it.
  - `ref_handle`: short, stable, user-typeable handle for `((blk-XXXXXX))` references.
    Default-derived from the id (last 6 chars of the ULID's Crockford base32, lowercased, with the `blk-` prefix).
    Persisted verbatim so future changes to the derivation cannot invalidate references already in the wild.
  - `text`: the block's content **as of the last sync**, verbatim and untruncated.
    Optional — see [Sidecar versioning](#sidecar-versioning) for why an additive field like this one does *not* bump `version`.
    This is the "before" side [level 2](#level-2--medium-confidence) diffs the freshly parsed `.md` against; a hash can only answer "identical or not", so without it a reworded block is indistinguishable from a deleted one.
    Yes, this duplicates the `.md` body inside the sidecar.
    A truncated prefix would be smaller but makes two blocks sharing a long opening look identical, and a level-2 false positive hands one block's id — and its `ref_handle` — to a different block.
    That is the exact corruption matching exists to prevent, so the duplicated bytes are the cheaper side of the trade.
    A sidecar is a rebuildable cache, not a second source of truth: the `.md` still wins, always.

### Content hash

```
content_hash(block) = sha256(block.text_content.trim().normalize())
```

Where `normalize` collapses internal whitespace to single space and strips trailing whitespace.
This makes the hash robust to whitespace-only edits in external editors.

**Same hash function on read and write.**
Diverging hashes silently break matching.

### Sidecar versioning

Current version is **`2`**.

`version` answers exactly one question for a reader that did not write the file: *can I still trust the fields I know?*
Compatibility runs in **both directions**, and the two are not symmetric.

#### Backward — a new binary reading an old sidecar

Always supported, down to `MIN_READABLE_SIDECAR_VERSION` (currently `1`).
A read path is never dropped when a newer one lands; silent format drops are not allowed.

- A v1 sidecar (no `ref_handle`) loads fine; the field is backfilled in memory by deriving it from the block id.
- A sidecar with no `text` loads fine; the field comes back empty and **level 2 simply doesn't fire for those blocks**.
  There is nothing to backfill it from — the point of the field is to hold the text as it was *before* the `.md` on disk changed.
  Matching degrades to hash + position, exactly what shipped before the field existed, never worse.
  The next write records the text, so the page is covered from then on.
- Sidecars below `MIN_READABLE_SIDECAR_VERSION` fail loudly — old workspaces in the wild stay supported until that constant moves.

#### Forward — an already-shipped binary reading a new sidecar

**This is the direction that bites, and it cannot be fixed after the fact.**

Every released binary rejects `version > its own SIDECAR_VERSION`, and that check is frozen in the copies already on users' machines.
Worse, on the paths that consume a sidecar an *unreadable* one used to look exactly like a *missing* one: no old blocks, so every block matched at level 3, took a fresh ULID, and the old id stayed in the tree.
One boot of a stale device against a shared folder duplicated the whole page and rotated every `((blk-…))` handle — and the newer binary did the same in reverse on its next boot.

The devices in one workspace never update at the same instant: the mobile build sits on TestFlight for days while the desktop is already ahead, and a laptop can stay closed for a week.
A version bump is therefore a **break for real users**, not a hypothetical one.

#### The rule

1. **An additive field does NOT bump `version`.**
   Give it `#[serde(default)]` and the format stays readable in both directions: an older reader ignores the unknown JSON key, a newer reader treats "missing" as "feature off for this entry".
   `pipeline_version` and `text` are both this shape, which is why both live at version `2`.
   **Feature detection is per-field presence, never per version number.**
   An empty `text` disables level 2 for that block whatever number the file carries.
   That is also the only correct answer once an old binary rewrites the sidecar and drops the field it never knew about.
2. **Bump only when an older reader would _misread_ the file** — an existing field changes meaning, changes encoding, or goes away.
   There the old binary's `UnsupportedVersion` is the *desired* outcome: a loud refusal beats silent corruption.
3. **A bump is a coordinated release, not a patch.**
   It needs a migration note here and an `outl doctor` path, because every device that has not updated stops reconciling those pages until it does.

Defence in depth for the day rule 2 applies: `reconcile_md` **propagates** `UnsupportedVersion` instead of treating the page as sidecar-less.
A sidecar written by a newer peer is not corruption, and rebuilding "from scratch" over one is what turns a version mismatch into duplicated blocks.
(A sidecar that is genuinely corrupt — unparseable JSON — still rebuilds, per the "never block on a corrupt sidecar" rule.)

---

## Roundtrip

```
parse(render(ast)) == ast
render(parse(md)) ≈ md
```

The second is "semantically equivalent", not byte-equivalent.
We may normalize:

- Tabs → two spaces.
- Trailing whitespace on lines stripped.
- Final newline added if missing.
- Property lines with leading whitespace normalized.

We never:

- Reorder blocks.
- Change content.
- Drop properties.
- Change inline link syntax.

Roundtrip is a **property test** in `crates/outl-md/tests/roundtrip.rs`.
Treat it as part of the spec — if your parse/render changes break it, either the test is wrong (rare) or your change is.

---

## 3-level matching

When a file save lands on disk and the `.md` differs from the sidecar's `last_synced_hash`:

1. **Parse** new `.md` → `new_ast` (no IDs).
2. **Read** sidecar → `old_ast` (with IDs, with hashes).
3. **Match** blocks `new ↔ old` at three confidence levels.

### Level 1 — High confidence

Block matches by:
- `content_hash` exact match between `old_block` and `new_block`, AND
- parent matches (by hash of parent, or both are root-level)

→ Preserve ID.
If position changed, emit `Op::Move`.

### Level 1.5 — Positional fallback

Runs only when the new and old block counts are equal (any insert or delete shifts every following DFS index and makes position meaningless).
A still-unmatched block at DFS index `i` takes the id of `old_blocks[i]` when that entry is unused, sits at the **same indent**, and has the **same parent**.

Indent is not a parent: two blocks can sit at the same depth under different subtrees.
Matching on indent alone teleported one subtree's id — and its `((blk-…))` handle — into another subtree.
A rejection here is not fatal; level 2 below can still recover the id on similarity, and it warns when it does so across parents.

→ Preserve ID.
Emit `Op::Edit` (and `Op::Move` if needed).

### Level 2 — Medium confidence

Block matches by:
- Normalized Levenshtein similarity > 80% against the sidecar's `text`, AND
- a DFS index within ±2 — **unconditional**.
  Parent agreement is not an alternative gate; it only selects which warning gets logged.

Assignment is by **global confidence**, not by the order blocks appear in the file.
Every candidate pair is scored first, then resolved from the highest score down, and a winner must beat the runner-up on **both** sides (the best rival for that new block *and* the best rival for that old block) by a margin.
A pair that fails the margin declines and leaves both of its blocks free to keep competing.

Scoring first is what stops a **newly typed** block from taking the id, and the `((blk-…))` handle, of a block it merely resembles.
Consuming candidates in file order did exactly that: the new block was reached first and claimed the id, while the block the user actually edited fell to level 3.
`orphans` came out empty, so nothing was recorded anywhere.

→ Preserve ID.
Emit `Op::Edit` (and `Op::Move` if needed).
Log a `tracing` warning carrying the block id and the score — and a distinct, louder one when the match crosses parents (the user reparented *and* reworded the block in the same save).

This level is what carries a block's id through the common external edit: **one save that both rewords a block and adds or removes another**.
The count mismatch takes level 1.5 out of play, so before level 2 existed every reworded block in such a save minted a fresh ULID, the old id went to the trash, and every reference to it dangled.

A sidecar entry with no recorded `text` — written before the field existed, or by a peer binary that doesn't know it — doesn't fire level 2 at all; the gate is the empty string, not the version number.
See [Sidecar versioning](#sidecar-versioning).
Blocks longer than 4096 characters are skipped too: the Levenshtein DP is O(n·m), and a block that size is a pasted document, not a sentence someone reworded.

### Level 3 — No match

Block in `new_ast` has no match in `old_ast`:

→ New ULID assigned.
Emit `Op::Create`.

Block in `old_ast` has no match in `new_ast`:

→ Move to TRASH_ROOT (`Op::Move` to trash).
Emit before deletion:

```
2026-05-24T11:22:01-03:00 orphan block=01HXY... content="começava com..."
```

**Hard rule:** every level-3 deletion must hit `orphans.log` before the op is committed.
Silent deletion is a P0 bug.

### Tiebreakers

When two new blocks would match the same old block (or vice versa) at the same confidence:

1. Prefer matches at the same position.
2. Prefer matches with the same parent.
3. Prefer matches where the parent chain matches deepest.
4. If still tied: pick the one that minimizes total moves across the matching as a whole (greedy is fine for now; optimal can come later).

---

## Edge cases

### Duplicated block (Ctrl+D)

User selects a block in VS Code and presses Ctrl+D.
Now there are two blocks with identical content.

- First one matches the old `content_hash` at level 1.
  Keeps ID.
- Second one has the same hash but its **position** differs from the old one.
  After the first match is consumed, no other old block has this hash. → Level 3.
  New ULID.

### Two identical blocks swap parents

A and B both contain "TODO".
A was under page X, B under page Y.
After edit, A is under Y, B is under X.

- Pure hash match alone is ambiguous (both new blocks match both old blocks).
- Tiebreaker: parent matches → A stays under "Y" matches the old A under Y?
  No — the old A was under X.
  The "parent matches" tiebreaker breaks.
- Fall back: minimize total moves.
  Either pairing requires one move.
- Then minimize position diff.
  Pick the assignment that's lexicographically smallest.

This is the case tested in `identical_blocks_swap.rs`.

### Heavy edit (>20% content change)

The content hash is gone.
Similarity may drop below 80%.

- If still > 80%: level 2, log warning.
- If below 80% but the counts are unchanged and the parent + indent still agree: level 1.5 keeps the id (structure says it's the same slot).
- If below 80% AND the counts changed: level 3, treat as a new block — and the old id hits `orphans.log`.

### Rename of header with many children

Heading text changes.
Children unchanged.

- Header block: hash mismatch.
  Probably level 2 (similarity > 80% if rename is partial).
  New ID if rename is total.
- Children: hash matches.
  Stay under the (possibly new-ID) header because they were always under "the block at this position".

The structure tiebreaker handles this: we match parent chains, not parent IDs, when parents are themselves ambiguous.

---

## `outl reconcile` (manual resolution)

If the matching produces orphans or level-2 warnings, the user runs:

```
outl reconcile
```

A TUI opens showing one orphan at a time with candidates.
Keys:

| Key | Action |
|-----|--------|
| `j` / `k` | next/prev candidate |
| `enter` | accept match |
| `d` | confirm delete (orphan stays as `Move` to trash) |
| `s` | skip (revisit later) |
| `q` | quit |

---

## External paste → outl syntax

When the user pastes clipboard markdown from another outliner / note app into outl, `outl_actions::paste_markdown` (in `outl-actions`) normalises the input before parsing it as bullets.
The same pipeline runs in every client — the TUI (bracketed-paste handler), the desktop, and mobile (textarea `onPaste`).
This section is only the **syntax-translation table**; for how paste works as a whole (with / without formatting, rich `text/html` conversion, paragraph splitting) see [Paste](paste.md).

| Input (external) | Output (outl) | Origin |
|------------------|---------------|--------|
| `{{[[TODO]]}} foo` | `TODO foo` | Roam |
| `{{[[DOING]]}} foo` | `DOING foo` | Roam |
| `{{[[DONE]]}} foo` | `DONE foo` | Roam |
| `- [ ] foo` | `- TODO foo` | GitHub / CommonMark task list |
| `- [/] foo` | `- DOING foo` | Logseq / Obsidian tasks |
| `- [x] foo` / `- [X] foo` | `- DONE foo` | GitHub / CommonMark |
| `{{embed: ((blk-XXXXXX))}}` | `!((blk-XXXXXX))` | Roam |
| `{{[[query]]: foo}}` | `{{query: foo}}` | Roam |
| `^^highlight^^` | `==highlight==` | Roam |
| `{{video: url}}` and other unknown `{{…}}` | (stripped) | various |
| `id:: <26-char Crockford ULID>` (alone on a line) | (line dropped) | Logseq |
| `[[June 2nd, 2026]]`, `[[Apr 22nd, 2026]]`, `[[2026/04/22]]` | `[[2026-06-02]]` etc. | Roam / mixed |
| 4-space indent | 2-space indent | Roam / Notion export |

A balanced `^^highlight^^` is rewritten to outl's native `==highlight==`.
Unknown `{{…}}` tokens that aren't outl-native are stripped on purpose so blocks land clean.
A lone unbalanced `^^` has no pair to convert or strip, so it survives verbatim.
Block properties parsed off the source (`key:: value` indented under a bullet) become `Op::SetProp` on the newly-created node so they converge across devices like every other op.

Date refs `[[…]]` whose inner text parses as a date land as the ISO slug outl uses for journals.
Supported forms: long month (`June 2nd, 2026`), short month (`Apr 22nd, 2026`), slashed ISO (`2026/04/22`).
Plain page refs (`[[Avelino]]`) and ambiguous dates (`[[June 2nd]]` without a year) pass through untouched.

The `id::` line strip is strict.
Only 26-character Crockford base32 strings count.
A random 26-character alphanumeric label (e.g.
`id:: IIIILLLLOOOO0000000000000A`) is not a ULID and stays on the page.

Heuristic: when no line is either a bare `-` or starts with `- ` (after leading whitespace), the paste is treated as plain text.
The clipboard payload is spliced into the current block at the caret, no tree conversion.
The bare `-` form matches the parser, which treats a lone `-` on a line as an empty bullet.

Caret offsets in the mobile client are converted from UTF-16 code units (what `textarea.selectionStart` reports) into Unicode codepoints before the Tauri round-trip.
This ensures pasting after an emoji or other supplementary-plane character lands the splice at the right spot.

The orphan log is cleared as items are resolved.
