# RFC 0255 — Three surfaces, three names, and one refusal that reaches two of them

| | |
|---|---|
| **Status** | Accepted |
| **Issue** | [#255](https://github.com/outlmd/outl/issues/255) |
| **PR** | none yet |
| **Date** | 2026-09-03 |
| **Reference doc** | [`docs/clients.md`](../clients.md) |
| **Invariant** | root `CLAUDE.md` invariants 8 and 12 |
| **Guarded by** | `crates/outl-actions/src/refusal.rs::tests::{every_surface_declares_every_refusal, every_declared_state_explains_itself, the_clients_doc_matches_the_refusal_matrix}`; `crates/outl-cli/src/output.rs::tests::ahead_of_log_wording_is_the_actionerrors_display_not_a_second_sentence`; `crates/outl-cli/tests/mcp_smoke.rs::frozen_page_update_returns_structured_refusal_not_a_generic_error`. |

Sub-project **D** of the client convergence effort (A → B → C → D), and the last.
A unified the design tokens ([RFC 0022](0022-unified-design-tokens.md)).
B made feature parity a compile error ([RFC 0253](0253-client-capability-catalog.md)).
C closed the mobile gaps ([RFC 0254](0254-mobile-capability-gaps.md)).
D is about the two surfaces with no UI to converge.

## Why

outl exposes the same workspace operations through three vocabularies that were named independently.

| Surface | Convention | Example |
|---|---|---|
| MCP tools (41) | `noun_verb` | `outl_page_delete` |
| CLI subcommands (28) | `noun verb` | `outl page delete` |
| `outl_shortcuts::Action` (78) | `VerbNoun` | `DeletePage` |

Each is internally consistent. None agrees with the others, and nothing notices when they drift.

That is the shallow half of the problem, and on its own it would not justify an RFC — `page delete` → `outl_page_delete` is guessable, and a rename sweep would cost more than it returns.

### The half that matters

[Invariant 8](../../CLAUDE.md) requires that a page which stopped syncing — its `.md` holding content that exists in no op — be **surfaced**, not swallowed. `docs/clients.md` § "Surfacing a page that stopped syncing" is explicit that a refusal reaching only a log line is the defect:

> Before this banner existed the refusal only reached a backend log line, so the page appeared to freeze with nothing said.

It then carries a per-surface table: the CLI names the page in `outl doctor`, the desktop and mobile render `<PageAheadOfLogBanner />`, the TUI is recorded as not surfacing it yet.

**The MCP server is not in that table, and does not handle the refusal.**
`ActionError::PageMarkdownAheadOfLog` is constructed in `outl-actions` and handled in `outl-cli` and `outl-tauri-shared`; `crates/outl-cli/src/mcp/` never names it.

This is not hypothetical. `outl_page_get`, `outl_page_update` and `outl_block_append` are exactly the operations that touch a frozen page. An agent driving them gets a generic failure or an apparent success, with nothing saying the page has stopped converging — which is the precise silence invariant 8 exists to end, on the one surface whose caller is a program that will retry rather than a human who will notice.

### And the table itself is hand-maintained

The per-surface table in `docs/clients.md` is prose. Nothing regenerates it and no test pins it.

Sub-project B removed exactly this defect for capabilities: `docs/client-parity.md` used to be hand-written and was wrong in three places at once, so the fact moved into an exhaustive `match` and the doc became a projection of it. The refusal table is the same shape of fact, one document over, still hand-maintained — and it is already incomplete, because the MCP is missing from it.

## What we chose

**Make the refusal a declared surface matrix, and put the MCP in it.** Two parts, in this order.

**Part 1 — the MCP surfaces the refusal.**
`crates/outl-cli/src/mcp/` maps errors to JSON-RPC responses. `PageMarkdownAheadOfLog` gets a distinct, structured response rather than a generic failure: the page, the line count, the sample, and the recovery command (`outl reconcile --ahead-of-log`). The wording comes from the existing owner — `@outl/shared/warnings::aheadOfLogNotice` is the TypeScript owner for the GUI clients, and the Rust side must not grow a second copy of the sentence. A caller reading the response must be able to tell "this page stopped syncing" from "that operation failed".

**Part 2 — the surface table is generated.**
An exhaustive `match` over (refusal, surface), in the shape `outl_shortcuts::support` established: a new surface does not compile until every refusal declares what it does there, and the `docs/clients.md` table becomes a projection pinned by a test.

The surfaces are **five**: TUI, Desktop, Mobile, CLI, MCP. Note that this is a different axis from `Client`, which has three members and deliberately excludes the CLI and MCP because they render no outline. Refusals are not capabilities: a surface that performs an operation owes the user an explanation when it declines, whether or not it draws anything.

**The vocabulary alignment is explicitly NOT part of this RFC.** See Scope.

## Why not the alternatives

**Rename the three vocabularies to match.**
The obvious reading of the issue, and rejected. Renaming `Action::DeletePage` to `PageDelete` touches every client's dispatch, every chord doc, and the generated parity table, to make a name guessable that already was. Renaming CLI subcommands or MCP tools breaks every script and agent configuration in the wild. The cost is real, the benefit is cosmetic, and it does nothing about the refusal.

**Reuse `Client` for the refusal matrix.**
Tempting — it exists and it already has a `Support` vocabulary. Rejected because `Client` means "a thing that renders an outline", which is why it has three members. Adding the CLI and MCP to it would give every one of the 78 `Action` rows two columns that can only ever say "not applicable", to serve a different question.

**Document the MCP refusal in `docs/mcp.md` and stop there.**
That is what `docs/clients.md` already does for the other four surfaces, and it is exactly why the MCP is missing: a hand-maintained table does not fail when a surface is added.

## The opposite direction

**Required section — what this makes worse.**

**A fifth surface makes every refusal five declarations.**
Today a new refusal is an `ActionError` variant and whatever handling its author remembers. After this, it does not compile until five surfaces answer — including two whose honest answer is often "returns it as a structured error, no UI". That is the mechanism working, and it is a real tax.

**The MCP response shape becomes a compatibility surface.**
A structured refusal is something callers will parse. Once agents depend on it, changing the shape breaks them, in a way a prose log line never could.

**Nothing here changes what is written to disk.**
No `Op`, no `.md`, no sidecar. This RFC changes what a surface *says* when it declines a write that was already declining. The write path is untouched — deliberately, because invariant 8's guard is the thing working correctly here and only its reporting is incomplete.

## Who does not have this

Per [invariant 12](../../CLAUDE.md).

**Update, post-RFC:** a later audit found five more production write paths (outside CLI/MCP) still calling the unconditional writer — `outl-actions::sync::reproject_page`, `outl-actions::exec::run_code_block`'s `call:` branch, and three TUI sites (`template.rs`, `exec.rs`, `autocomplete.rs`).
Fixing them gave the TUI *partial* coverage: every TUI-initiated write that re-projects a page now reports a refusal on the status line (or a toast, for the peer-sync reload path), so the blanket "the TUI does not surface the refusal" above is no longer accurate.
What is still true, unchanged: **the TUI does not call `apply_page_md_with_sidecar_if_stale` on its own page-open path**, so a page that drifted ahead of the log with no local write attempt in between (e.g. an external edit synced in by a peer) stays silent until the next write touches it.
`crates/outl-actions/src/refusal.rs`'s TUI row now reflects this split instead of a flat "not surfaced yet"; `docs/clients.md`'s generated table is the current source, this line is history.

## How it cannot regress

**The rule.** Invariant 8 already requires the refusal to reach the user. This RFC widens "the user" to include a program calling the MCP, and invariant 12's enumeration requirement extends to the refusal matrix.

**The tests:**
1. **`every_surface_declares_every_refusal`** — the exhaustive-match pin, mirroring `every_client_declares_support_for_every_action`.
2. **`the_clients_doc_matches_the_refusal_matrix`** — the doc becomes a projection, pinned like `the_parity_doc_matches_the_code`.
3. **An MCP test that a frozen page returns the structured refusal**, not a generic error — the behaviour Part 1 adds, pinned so a later refactor of the error path cannot silently flatten it.
4. **A test that the refusal wording has one owner** — the Rust MCP response and `@outl/shared/warnings::aheadOfLogNotice` must not drift into two sentences for one condition.

## Scope

Not covered here:

- **Renaming anything.** The three vocabularies keep their conventions; see Why not the alternatives.
- **Making the TUI surface the refusal.** Declared, not closed.
- **Other `ActionError` variants.** The matrix is built with `PageMarkdownAheadOfLog` as its first and only member; adding the rest is mechanical once the mechanism exists, and doing it in one change would bury the mechanism under forty rows of declarations.
