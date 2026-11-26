# Paste

outl has two paste modes, and both do more than drop raw text into a block.
The goal is that whatever you copied — a Slack message, a Google Doc paragraph, an outline from another app, a chat reply — lands as a tidy outline in outl, not as a wall of unformatted text or a mess of stray characters.

- **Paste with formatting** converts the clipboard into outl markdown: rich formatting is kept, bullet structure becomes a real outline, and plain prose is split into one block per paragraph.
- **Paste without formatting** inserts the raw clipboard text verbatim — no conversion, no splitting.
  On the desktop it is spliced into the block you are editing.
  In the TUI it becomes one new block ([details](#in-the-tui)).

The chords are per-client; the full table lives in [Shortcuts](shortcuts.md).
In short: desktop `Cmd/Ctrl+V` and, in Normal mode, `p` (with) and `Cmd/Ctrl+Shift+V` (without); TUI `p` (with) and `Shift+P` (without); mobile is always with formatting.

The sections below describe the desktop and mobile behaviour, which share one code path.
The TUI reads the clipboard itself and differs in a few places, so it has its own section: [In the TUI](#in-the-tui).

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
   (The TUI has no caret to splice at in Normal mode, so there a single line becomes a new block.)

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
- **Tab-separated text whose first line is a bullet.**
  `- a<TAB>b` is an outline with tabs in it, so it stays an outline.
- **Tab-indented code or outlines.**
  They have a consistent field count per line, which is most of what identifies a grid — so the gate also requires that no line *starts* with a tab.
  This costs the rare table whose first column is empty on every row; it re-pastes fine with that column filled.
- **A single line.**
  One row has no header to rule off.
- **Paste without formatting.**
  `Cmd/Ctrl+Shift+V` / `Shift+P` means without formatting, tabs included.

A `|` inside a pasted cell is escaped to `\|` so it cannot open a column that was never there.
A cell holding a line break (a spreadsheet allows one) is flattened to a space — the one lossy step, and a visible one.

## Paste after the selected block (desktop Normal mode)

With a block selected and nothing focused, `p` pastes the OS clipboard as blocks **after** the selected block, converted the same way as paste with formatting.
`Cmd/Ctrl+V` does exactly the same, except that when a block cut with `Cmd/Ctrl+X` is pending it moves that block instead.
Copying (`y y`, `Y`, `Cmd/Ctrl+C`, Visual `y`) disarms a pending cut, so the next `Cmd/Ctrl+V` pastes what you just copied.

Where the paste lands:

| Situation | Landing |
|---|---|
| A block is selected | Right after it, as its next sibling |
| Zoomed into the selected block | As the last child of the zoom root |
| The page is empty | At the end of the page |
| The selected block is a backlink row | Refused, with a status message: a backlink row belongs to another page |

The selection moves to the **last pasted root block**, so `p` again stacks the next paste after the first, and a pasted subtree is never nested inside the copy before it.
The backend resolves the position once: the parent if it is still on the page, else the block to paste after, else the end of the page, else it refuses.
A block deleted in the meantime therefore never sends the paste to the wrong place.

A copy and a paste round-trip: `y y` then `p` duplicates the block and its subtree with fresh ids, and copying out of outl and pasting into outl reconstructs the same tree.
After a Visual `y`, one extra `y` is swallowed, so the habit of typing `y y` then `p` works from Visual mode too.

Plain text pasted this way (a single line, or a clipboard that is not an outline) has trailing newlines and CRLF trimmed, and a clipboard holding only whitespace is ignored.

## Where the caret lands, and what happens to the block you were in

This section is the desktop and mobile behaviour.
The TUI never merges into the block you were in, see [In the TUI](#in-the-tui).

A paste with the caret inside a block is the one paste that rewrites the block you are **typing in**.
With formatting, the first pasted bullet joins the text left of the caret and becomes that block's new text; the rest become siblings below it, and whatever was right of the caret becomes one more sibling so nothing you typed is lost.
Without formatting, the raw text is spliced in and your tail stays where it was — which is why the caret lands between the two, not at the end of the block.

Both clients put that new text back into the editor before you keep typing, and both take it from the backend rather than working it out themselves: the merge can insert a separating space, so "where I pasted plus what I sent" is not the answer.

This used to be where the paste went missing ([issue #488](https://github.com/outlmd/outl/issues/488)).
Each client holds the block being edited as a draft seeded once, so refreshing the outline was not enough — the editor stayed on the pre-paste text and wrote it straight back over the paste on the next Esc, blur or tap away.
A link copied out of an email vanished from the block and from the `.md`, with nothing on screen to say so.
A plain-text URL was unaffected, because a trivial paste never leaves the browser.

## Paste without formatting

`Cmd/Ctrl+Shift+V` (desktop) and `Shift+P` (TUI) paste the raw clipboard text with **no** conversion.
On the desktop the text is spliced into the current block as-is, and outline-looking or multi-paragraph content is **not** split.
In the TUI the raw text becomes one new block after the selected one.
Use it when you want the literal characters — a code snippet, a block of text whose line breaks matter, markdown you want to keep as source rather than render.

Mobile has no without-formatting chord; every mobile paste is with formatting.

## Pasting inside a code block

When the block you are editing is a fenced code block (a `` ``` `` block), **every** paste is literal — the same as paste-without-formatting — regardless of the chord you use.
A multi-line or outline-shaped clipboard is spliced in verbatim, with its line breaks intact, instead of being converted into sibling blocks (which would tear the fence apart and strand the closing `` ``` `` on its own line).
This holds on both GUI clients (desktop and mobile), because the guard lives next to the shared `choosePasteRoute` decision.

## Known losses

A paste is converted before it is parsed, and two conversions drop text without saying so:

- A first line `title:: x` is lifted as a page property and is not kept as a block.
- An unknown `{{…}}` token outside a fenced code block is stripped (see [External paste](markdown-format.md#external-paste--outl-syntax)).
  Inside a fence nothing is rewritten, so a code block pasted from anywhere keeps its `{{…}}` and its `- [ ]` lines.

## In the TUI

The TUI has no caret inside a block while you are in Normal mode, so a paste there always adds blocks **after the selected block**.
It never merges into the text of a block.

| You do | Where the text lands |
|---|---|
| `p` in Normal | After the selected block, converted: an outline keeps its hierarchy, a markdown table or tab-separated rows become one table block, several lines become one block per non-blank line, a single line becomes one new block |
| `P` in Normal | After the selected block, as **one** block of raw text, nothing converted or split |
| Your terminal's paste (`Cmd+V`, `Ctrl+Shift+V`) in Normal | Same as `p` |
| Your terminal's paste while editing a block (Insert) | Plain text is spliced into the block at the caret, line breaks included. An outline, a table or tab-separated rows are different: the edit is committed first, then they land as new blocks after the current one, like `p` |
| Dragging a file into the terminal while editing | The file is imported into `assets/` and its link is inserted at the caret, see below |

After a paste the selection moves to the last pasted block, and the footer says `pasted 3 blocks` (or `pasted text` when the clipboard held one plain line).
`clipboard unavailable` means outl could not read the system clipboard, which is common over SSH.
Paste from your terminal instead, or copy inside outl and paste in the same session.
`paste failed: <reason>` leaves the page as it was.

**What is different from the desktop.**

- There is no rich-text conversion, because a terminal clipboard carries plain text only.
  Copy from a web page or Slack and the formatting is already gone before outl sees it.
- There is no guard for code blocks.
  The desktop and mobile paste literally inside a fenced code block.
  In the TUI a plain-text paste into an open fence is spliced literally, but an outline-shaped one (lines starting with `- `) is still converted into blocks.
  Press `Esc` and use `P` to put the raw text in its own block, then move the text where you need it.
- There is no caret merge.
  The first pasted bullet does not join the text left of the caret, so there is no "tail" to put back.

**Terminals without bracketed paste.**
outl asks the terminal to wrap pasted text in markers so it can tell a paste from typing.
A terminal that ignores the request sends the paste as keystrokes, and every line break becomes `Enter`, which commits the block and starts a new one.
If a multi-line paste in Insert mode splits into blocks you did not expect, use `p` from Normal mode instead, which reads the clipboard itself and does not depend on the terminal.

**Dropping files.**
Terminals deliver a dragged file as its path pasted as text.
In Insert mode, when the paste is exactly the path of one existing file, or several lines that are each an existing file, outl imports each into `<workspace>/assets/<hash>.<ext>` and inserts the markdown links separated by spaces.
Backslash-escaped spaces (`/My\ Files/report.pdf`, the macOS drag convention) are understood.
One line that is not an existing file, or a list where any line is not, pastes as ordinary text.
A file over `[assets] max_bytes` is refused with `import failed: …` and nothing is inserted.
In Normal mode a pasted path is just text.
Link format: [markdown-format.md → Asset links](markdown-format.md#asset-links-nameassetshashext).

The rest of the TUI's clipboard behaviour (copying out, OSC 52 over SSH) is in the [TUI manual](tui.md#clipboard).

## Under the hood

The behaviour is shared across clients so it stays identical everywhere:

- **`choosePasteRoute(html, plain)`** (`@outl/shared/paste`) — the rich / structured / native decision, shared by desktop and mobile.
- **`htmlToOutlMarkdown(html)`** (`@outl/shared/paste`) — the `text/html` → outl markdown conversion, built on [Turndown](https://github.com/mixmark-io/turndown) tuned for the outl dialect.
- **`tableElementToMarkdown(table, convert)`** (`@outl/shared/paste`) — the `<table>` → markdown table rule Turndown does not ship. TypeScript-only, because HTML never reaches the Rust side.
- **`outl_actions::looks_structured`** — the one predicate a client asks before paying for a round trip: an outline, a markdown table, or tabular data. Mirrored by `choosePasteRoute`'s `structured` arm.
- **`outl_md::tsv_to_markdown`** — tab-separated text → markdown table, and the owner of the "is this tabular" gate (`from_delimited`) both Rust and TypeScript answer from.
- **`outl_actions::paste_markdown`** — with-formatting: normalises external syntax, detects outline shape, splits paragraphs, and grafts the result as blocks through the op log.
- **`outl_actions::paste_plain`** — without-formatting: raw text as one block, no normalisation or splitting.
- **`PasteOutcome.host_text` / `host_caret`** (`outl_actions`) — the host block's new text and where the caret belongs in it, in codepoints. The one owner of both, carried to the GUI clients as `PasteReply` and folded back into the editor by **`pasteEditorUpdate`** (`@outl/shared/paste`), which also converts the offset to the UTF-16 units a textarea counts in.

The TUI reads the OS clipboard directly (`arboard`) and runs the same `outl_actions` pipeline; it has no `text/html` flavour to convert, so rich-clipboard conversion is a GUI-only capability.
Tab-separated and markdown tables reach it through `text/plain` like everything else, so a spreadsheet paste works there too.
What the TUI does with the result is in [In the TUI](#in-the-tui).
On the desktop every OS clipboard read and write goes through `lib/os-clipboard.ts`, the Tauri clipboard-manager plugin (backend `arboard`), never `navigator.clipboard`: `p`, `y y`, `Cmd/Ctrl+C` and paste-without-formatting all use it.
The macOS webview gates the read behind a native "Paste" permission button when it is called outside a real paste gesture, and refuses the write outside a user gesture without saying so, so the plugin is what makes the chords work.
The desktop command is `paste_block_after(page_id, after_id?, parent_id?, text)`; its reply carries `new_id`, the last pasted root.
The paste code lives in `outl_tauri_shared::commands::paste`, and the TUI lands on the same block through `PasteOutcome::last_root`.
See the [Shared primitives catalog](shared-primitives.md) — specifically [Markdown pipeline](primitives-markdown.md) — for where each piece lives.
