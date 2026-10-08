# Paste

outl has two paste modes, and both do more than drop raw text into a block.
The goal is that whatever you copied — a Slack message, a Google Doc paragraph, an outline from another app, a chat reply — lands as a tidy outline in outl, not as a wall of unformatted text or a mess of stray characters.

- **Paste with formatting** converts the clipboard into outl markdown: rich formatting is kept, bullet structure becomes a real outline, and plain prose is split into one block per paragraph.
- **Paste without formatting** splices the raw clipboard text into the current block verbatim — no conversion, no splitting.

The chords are per-client; the full table lives in [Shortcuts](shortcuts.md).
In short: desktop `Cmd/Ctrl+V` (with) and `Cmd/Ctrl+Shift+V` (without); TUI `p` (with) and `Shift+P` (without); mobile is always with formatting.

> **Why copy-out and paste-in are one pair, and why the core speaks exactly one format:** [RFC 0044](rfcs/0044-clipboard-and-paste.md).

## Paste with formatting

This is the default paste (`Cmd/Ctrl+V`, TUI `p`, every mobile paste).
It picks one of three routes, in order:

1. **Rich clipboard (`text/html`).**
   When you copy from an app that carries formatting — Slack, Google Docs, Notion, Gmail — the bold/italic/links/lists live in the clipboard's `text/html` flavour, while `text/plain` is stripped of them.
   outl reads the HTML and converts it to outl markdown, so the formatting survives:
   `**bold**`, `*italic*`, `[text](url)`, `- ` bullets, `~~strikethrough~~`, and inline code all come across.
   Custom emoji pasted as an image (Slack renders `:bus:` as `<img alt=":bus:">`) keep their `:shortcode:`.
   Editors that encode weight as inline CSS instead of `<b>` (Google Docs above all) are handled too: a `font-weight:700` span becomes `**bold**`, and the non-bold `<b>` wrapper Docs wraps the whole message in does not bold the entire block.
   A `<table>` becomes a markdown table, with each column's alignment read off the header's `text-align` — see [Tabular data](#tabular-data) below.

2. **Structured plain text.**
   With no richer HTML, if the clipboard is already an outline (lines starting with `- `) or has multiple paragraphs, it is routed through the conversion pipeline.
   An outline keeps its hierarchy; multi-paragraph prose (a pasted chat reply, an email) becomes one block per paragraph instead of a single wall-of-text block.
   Markdown copied from another outliner (Roam, Logseq, a GitHub task list) is normalised to the outl dialect on the way in.
   The full syntax-translation table is in [Markdown dialect → External paste](markdown-format.md#external-paste--outl-syntax).

3. **Tabular plain text.**
   Tab-separated lines — what a spreadsheet, a SQL client or `column -t` puts on the clipboard — become one markdown table.
   See [Tabular data](#tabular-data).

4. **Trivial text.**
   A single word, a URL, one line — spliced into the block in place, no round-trip, so a routine paste stays instant.

The routing decision is one shared function, `choosePasteRoute`, so the desktop and mobile clients can never disagree about what a given clipboard should do.

## Tabular data

Copy a range out of Excel, Google Sheets, Numbers, a Notion database, a `psql` result or an HTML table on a web page, and it lands as an outl table — **one block**, rendered as a grid on every client ([issue #329](https://github.com/outlmd/outl/issues/329)).

Three sources, one destination:

| What you copied | How it arrives | What happens |
|---|---|---|
| A spreadsheet range, a database result | `text/html` with a `<table>`, plus tab-separated `text/plain` | The HTML wins: `<table>` → markdown table, per-column alignment kept |
| A terminal's columns, a `.tsv` snippet | tab-separated `text/plain` only | Converted to a markdown table |
| A markdown table (a README, an assistant's reply) | `text/plain` | Recognised as a table, kept as one block |

All three end up as the same markdown, so the [dialect's table rules](markdown-format.md#tables) apply from there on: one block, cells carrying inline markdown, nothing truncated.

What is deliberately **not** tabular:

- **Comma-separated text.**
  Prose carries commas, and `a, b` on two lines is far more often two sentences than a 2×2 grid.
  A `.csv` file is an import, not a paste.
- **Tab-indented code or outlines.**
  They have a consistent field count per line, which is most of what identifies a grid — so the gate also requires that no line *starts* with a tab.
  This costs the rare table whose first column is empty on every row; it re-pastes fine with that column filled.
- **A single line.**
  One row has no header to rule off.
- **Paste without formatting.**
  `Cmd/Ctrl+Shift+V` / `Shift+P` means without formatting, tabs included.

A `|` inside a pasted cell is escaped to `\|` so it cannot open a column that was never there.
A cell holding a line break (a spreadsheet allows one) is flattened to a space — the one lossy step, and a visible one.

## Paste without formatting

`Cmd/Ctrl+Shift+V` (desktop) and `Shift+P` (TUI) paste the raw clipboard text with **no** conversion:
the text is spliced into the current block as-is, and outline-looking or multi-paragraph content is **not** split.
Use it when you want the literal characters — a code snippet, a block of text whose line breaks matter, markdown you want to keep as source rather than render.

Mobile has no without-formatting chord; every mobile paste is with formatting.

## Pasting inside a code block

When the block you are editing is a fenced code block (a `` ``` `` block), **every** paste is literal — the same as paste-without-formatting — regardless of the chord you use.
A multi-line or outline-shaped clipboard is spliced in verbatim, with its line breaks intact, instead of being converted into sibling blocks (which would tear the fence apart and strand the closing `` ``` `` on its own line).
This holds on both GUI clients (desktop and mobile), because the guard lives next to the shared `choosePasteRoute` decision.

## Under the hood

The behaviour is shared across clients so it stays identical everywhere:

- **`choosePasteRoute(html, plain)`** (`@outl/shared/paste`) — the rich / structured / native decision, shared by desktop and mobile.
- **`htmlToOutlMarkdown(html)`** (`@outl/shared/paste`) — the `text/html` → outl markdown conversion, built on [Turndown](https://github.com/mixmark-io/turndown) tuned for the outl dialect.
- **`tableElementToMarkdown(table, convert)`** (`@outl/shared/paste`) — the `<table>` → markdown table rule Turndown does not ship. TypeScript-only, because HTML never reaches the Rust side.
- **`outl_actions::looks_structured`** — the one predicate a client asks before paying for a round trip: an outline, a markdown table, or tabular data. Mirrored by `choosePasteRoute`'s `structured` arm.
- **`outl_md::tsv_to_markdown`** — tab-separated text → markdown table, and the owner of the "is this tabular" gate (`from_delimited`) both Rust and TypeScript answer from.
- **`outl_actions::paste_markdown`** — with-formatting: normalises external syntax, detects outline shape, splits paragraphs, and grafts the result as blocks through the op log.
- **`outl_actions::paste_plain`** — without-formatting: raw text as one block, no normalisation or splitting.

The TUI reads the OS clipboard directly (`arboard`) and runs the same `outl_actions` pipeline; it has no `text/html` flavour to convert, so rich-clipboard conversion is a GUI-only capability.
Tab-separated and markdown tables reach it through `text/plain` like everything else, so a spreadsheet paste works there too.
On the desktop, paste-without-formatting (`Cmd/Ctrl+Shift+V`) reads the clipboard through the Tauri clipboard-manager plugin (backend `arboard`), not `navigator.clipboard.readText()`.
The macOS webview gates that web API behind a native "Paste" permission button when it is called outside a real paste gesture, so the plugin read is what makes the chord work.
See the [Shared primitives catalog](shared-primitives.md) — specifically [Markdown pipeline](primitives-markdown.md) — for where each piece lives.
