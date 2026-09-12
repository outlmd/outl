# RFC 0276 — An MCP reply carries its payload once, and an error is the exception

| | |
|---|---|
| **Status** | Accepted |
| **Issue** | none — the problem statement is the PR body; numbered after the PR so `0276-*.md` is findable from it |
| **PR** | [#276](https://github.com/outlmd/outl/pull/276) (original work in [#273](https://github.com/outlmd/outl/pull/273)) |
| **Date** | 2026-09-12 |
| **Reference doc** | [`docs/cli.md` → Commands by domain](../cli.md#commands-by-domain) owns the wire shape; [`crates/outl-cli/CLAUDE.md` → MCP](../../crates/outl-cli/CLAUDE.md#mcp) owns why a field is droppable |
| **Invariant** | none (root); the rule lives in `crates/outl-cli/CLAUDE.md` → MCP |
| **Guarded by** | `success_payload_is_content_only_and_compact`, `error_payload_keeps_structured_content`, `journal_reads_keep_their_ids_and_date`, `pruning_leaves_the_parser_ast_alone` (`crates/outl-cli/src/mcp/tools/payload.rs`); `no_tool_declares_an_output_schema` (`crates/outl-cli/src/mcp/tools/registry.rs`); `daily_today_over_mcp_returns_ids_a_write_tool_can_use`, `export_json_over_mcp_round_trips_into_the_parser_ast`, `frozen_page_update_returns_structured_refusal_not_a_generic_error` (`crates/outl-cli/tests/mcp_smoke.rs`) |

## Why

Every successful MCP `tools/call` sent its payload twice.
`structuredContent` held the handler's `data` inside the CLI's `{ ok, data, error }` envelope, and `content[0].text` held the same data again as pretty-printed JSON.
Four tools were a partial exception, and it is worth stating precisely because the change means something different for them.
`preferred_text_for` already flattened `outl_page_render`, `outl_export_md`, `outl_daily_today` and `outl_daily_get` to their `md` field, so their text was raw markdown rather than a second JSON copy.
The `structuredContent` duplicate was there for all 41 regardless, which is why those four still show a saving below.
On top of that, every outline node carried `tokens`, a pre-tokenized inline AST that exists so the Tauri renderers do not need their own inline tokenizer, and which restates the `text` the model already has.

The consumer of this surface is a language model with a context window, so the cost is paid on every call, in tokens, before the model has read a word of the answer.
Measured against a 2,862-page workspace, before any of this was written:

| call | before | after |
|---|---|---|
| `outl_daily_today` | ~12.2k chars | ~5.9k |
| `outl_page_get` on a journal | ~22k | ~5.9k |
| `outl_page_list` | ~726k | ~303k |

The same handler behind `outl daily today --json` returns the same bytes to a script, where pretty-printing and `tokens` cost nothing that matters.
The problem is specific to the MCP wire, not to the data.

## What we chose

**A success reply is content-only and compact; an error keeps its envelope.**
One owner: `crates/outl-cli/src/mcp/tools/payload.rs`.
The shared `cmd/*` handlers and the CLI's own `--json` output are untouched — this is a projection of the reply, not a change to what the handler computes.

Three decisions, in the order a reader hits them on the wire:

1. **No `structuredContent` on success.**
   `content[0].text` carries the payload as compact JSON (`serde_json::to_string`, not `to_string_pretty`).
   For the two markdown-first tools, `outl_page_render` and `outl_export_md`, it carries the raw `.md` string instead, because their payload is `{slug, md}` and the caller supplied the slug.
   Those two kept the behaviour `preferred_text_for` already gave them; what they lost is the `structuredContent` beside it.
   **`outl_daily_today` and `outl_daily_get` went the other way**, from raw markdown to JSON, and that is the one text change a caller can see rather than just a smaller one (see The opposite direction).
   `tool_success_payload` is the function.
2. **GUI-only outline fields are pruned.**
   On any object that carries `id` **and** `text` **and** `children` — the signature of `outl_actions::outline::OutlineNode` — `tokens` always goes, and `collapsed: false` / `todo: null` / empty `properties` go as default noise.
   Nothing else is touched.
   `prune_gui_fields` is the function; the `id` in the guard is load-bearing (see The opposite direction).
3. **An error keeps `structuredContent: { ok: false, error }` and sets `isError: true`.**
   Its text content is only a `code: message` summary, so the envelope is the sole machine-readable copy of `error.data`.
   [RFC 0255](0255-operation-vocabulary.md)'s `PAGE_MARKDOWN_AHEAD_OF_LOG` carries `path` / `lines` / `sample` / `recovery_command` there, and a caller that can read nothing but prose can report that a page stopped syncing without being able to do anything about it.
   `tool_error_payload` is the function.

**No tool declares an `outputSchema`, and that is part of the decision, not an omission.**
The MCP spec makes `structuredContent` optional only for a tool without an output schema; a tool that declares one is obliged to return `structuredContent` conforming to it on every success.
Taking decision 1 therefore commits this server to *not* declaring output schemas later without reopening this RFC.
`no_tool_declares_an_output_schema` pins it so the schema cannot arrive one tool at a time.

## Why not the alternatives

**Keep `structuredContent` and drop the text copy instead.**
The spec-shaped inverse: `structuredContent` is the typed channel, text is for display.
Rejected because the consumer is a model, and MCP hosts hand the model `content`, not `structuredContent`.
Dropping the text would move the payload into a field the primary reader never sees, to save a duplicate the reader is not charged for either way.

**Declare `outputSchema` per tool and keep both copies.**
The "correct" long-term shape under the spec.
It would have cost 41 hand-written schemas that mirror the handler DTOs and drift from them (the Rust↔TS wire-type pins in `outl-tauri-shared` exist because that drift is real), and it would have kept the duplicate the measurement above is about.
The benefit — a host validating replies against a schema — has no consumer today: desktop and mobile speak Tauri, not MCP, and nothing in the repo read `structuredContent` on success outside the smoke test.

**Trim the handler output in `cmd/*` so both surfaces get smaller.**
Rejected because the CLI's `--json` is a scripting contract and its readers are not paying per token.
`tokens` is also what the Tauri renderers consume through the shared DTOs; removing it upstream would push an inline tokenizer into two TypeScript clients, which is the parallel implementation the root `CLAUDE.md` exists to prevent.

**Flatten every markdown-shaped tool to its `.md`, including the journal reads.**
This was the first pass, and it shipped green.
`outl_daily_today` / `outl_daily_get` return `{date, meta, outline, md}`, and `outline` is the only place a block's id appears — ids live in the sidecar, never in rendered markdown.
Every block-targeting write tool needs one, so flattening them broke "read today's journal, tick a task" with nothing to report it.
`date` goes the same way: `outl_daily_today` takes no argument, so its reply is the only thing naming the journal it opened.
The test for adding a tool to `markdown_field` is therefore not "is markdown the useful part" but **"does the caller already have what I am about to drop"**.

**Key the pruning on `text` + `children` alone.**
Also the first pass, also green.
There are two `OutlineNode` types in this workspace: `outl_actions`' has an `id`, `outl_md::ast`'s is `{text, properties, children}` and does not.
`outl_export_json` returns the second, and its `properties` has no `#[serde(default)]`, so dropping an empty `properties` there made the export fail to deserialize back into the type that produced it — on every block without a property, which is most of them, silently, because the JSON still reads fine.

## The opposite direction

**Required section — what this makes worse.**

**It is a wire break, and the advice being broken is advice this repo published.**
`docs/cli.md` used to say "Clients should read `structuredContent.data` for typed access."
Any client that did is now reading `undefined` on every success.
Nothing in the repo does; nothing outside it is known to.
The `CHANGELOG.md` entry names the break, and the PR template now asks about wire contracts so the next one is named before merge rather than after.

**The journal reads changed the kind of their text content, not just its size.**
`outl_daily_today` and `outl_daily_get` put raw markdown in `content[0].text` before this and put JSON there now.
Every other tool's text either stayed JSON or stayed markdown, so these two are the only place a caller reading `content[0].text` gets a different *type* of thing back.
That is the price of decision 1: their `outline` carries the block ids and used to reach callers through `structuredContent`, so once the envelope goes, the text is the only channel left and it has to carry the whole payload.
A host that fed that text straight to a model as markdown now feeds it JSON with the markdown inside, under `md`.

**Success and error now have different shapes, and a caller has to branch on `isError`.**
Before, `structuredContent.ok` was one field that answered both.
A caller that assumed one shape for both paths will parse a success and miss the `error.data` on a failure, or the reverse.
The asymmetry is deliberate (an error's text is a summary, a success's text is the whole payload), but it is an asymmetry, and it lives in the caller's code.

**A projection can drop the only copy of a field, and this RFC did so twice before it was caught.**
Both mistakes were the same shape: judging a field by what it looked like (markdown-ish, default-ish) instead of by whether the caller could get it elsewhere.
Both passed CI because the unit tests used hand-written payloads rather than anything a handler produces, and the smoke test never called the four tools where the projection changes most.
The rule that came out of it is in `crates/outl-cli/CLAUDE.md` → MCP: **a projection is a place to drop what the reader can re-derive, never a place to drop the only copy.**
When adding a tool to `markdown_field` or widening the prune guard, name what the caller loses and where else they can get it.

**Nothing here changes what is written to disk.**
No `Op`, no `.md`, no sidecar, no handler.
The mirrored case for a projection change is "does the un-projected surface still hold what the projected one dropped", and the answer is yes: `outl daily today --json` still emits `tokens`, `collapsed` and `properties`, checked by hand and pinned by leaving `cmd/*` and `output.rs`'s success path alone.

## How it cannot regress

**The rule.**
`crates/outl-cli/CLAUDE.md` → MCP states the droppability test and names both ways it was got wrong; → "JSON envelope (CLI)" says MCP shares the handlers, not the wire format.
`docs/cli.md` → Commands by domain owns the shape itself.
`docs/development.md` → "Add an MCP tool" points a new tool author at `payload.rs` instead of telling them every tool "uses the same envelope", which is how a success reply grows a `structuredContent` back.

**The tests.**

1. `success_payload_is_content_only_and_compact` — fails if a success reply regains `structuredContent` or goes back to pretty-printing.
2. `error_payload_keeps_structured_content` — fails if the error path is "simplified" to match success and loses `error.data`.
3. `journal_reads_keep_their_ids_and_date` — fails if `outl_daily_today` / `outl_daily_get` are added back to `markdown_field`.
4. `pruning_leaves_the_parser_ast_alone` — fails if the prune guard is widened back to `text` + `children`.
5. `no_tool_declares_an_output_schema` — fails if any `tool_def` grows an `outputSchema`, which would make decision 1 non-compliant.
6. `daily_today_over_mcp_returns_ids_a_write_tool_can_use` and `export_json_over_mcp_round_trips_into_the_parser_ast` drive the real server, so a handler-side shape change that the hand-written payloads in 3 and 4 cannot see still fails.
7. `frozen_page_update_returns_structured_refusal_not_a_generic_error` (RFC 0255) is the consumer of decision 3: it reads `structuredContent.error.data` off an MCP error and would fail if the envelope were dropped there too.

Tests 3, 4 and 6 were run against the projection without their fixes and confirmed to fail, so they pin the behaviour rather than describe it.

## Scope

Not covered here:

- **The CLI `--json` envelope.** Unchanged, and its readers are scripts, not models.
- **The set of MCP tools and their input schemas.** The three description edits in `registry.rs` are wording, not shape.
- **Read-side slicing.** `outl_page_get` and `outl_block_tree` take no `depth`, `outl_page_list` takes no `limit`, so a single call on a large workspace can still swamp a context window after every saving here.
  That is additive to the input schemas and breaks nothing, which is exactly why it does not belong in the PR that breaks a wire format.
- **Declaring `outputSchema` in future.** Permitted only by superseding this RFC, because it reverses decision 1 for every tool that gets one.
