# RFC 0146 — A template is a page with a property, not a new op

| | |
|---|---|
| **Status** | Shipped |
| **Issue** | [#146](https://github.com/outlmd/outl/issues/146) |
| **PR** | — |
| **Date** | 2026-08-06 |
| **Reference doc** | [templates.md](../templates.md) |
| **Invariant** | root `CLAUDE.md` invariant 7 (anything that must converge goes through the op log) |
| **Guarded by** | `sets_from_template_on_root_blocks`, `from_template_only_on_root_not_descendants`, `untraced_variant_skips_from_template` (`template/instantiate.rs`), `inject_call_params_escapes_values`, `inject_call_params_canonicalizes_aliases`, `resolve_call_takes_first_code_fence`, `resolves_with_params` (`template/call.rs`), `duplicate_template_name_is_detectable` (`template/list.rs`) — all under `crates/outl-actions/src/`; plus `structural_instance_shows_in_template_backlinks`, `callable_site_shows_in_template_backlinks` (`src/backlinks.rs`), `open_journal_stamps_template_on_fresh_daily` (`src/page.rs`) |

## Why

outl had exactly one template: `templates/journal.md`, a file read on boot with `{{date}}` hardcoded.
There was no engine.

A user migrating from Roam hits two distinct needs, and neither was reachable.
**Structural** — stamp a known block shape into today's note (a 1:1 agenda, an interview checklist); without an engine every block is retyped by hand.
**Callable** — run a computation and read its result inline; on the measured Roam graph one salary calculator was invoked 53+ times through `{{roam/render}}`, so this is real usage, not a hypothetical.

[`docs/templates.md`](../templates.md) documents *what* templates do.
This RFC records why the definition is a page property and why there are two modes.

## What we chose

**A page with a non-empty `template::` property is a template, and its outline is the body.**
That property is set with the existing `Op::SetProp` — the feature adds **no new `Op` variant**.

Single owner: `outl_actions::template`, split by concern — `list.rs` (discovery), `vars.rs` (`{{date}}` / `{{today}}` / `{{page}}` / `{{time}}` substitution), `instantiate.rs` (structural), `call.rs` + `run.rs` (callable).
Per-client surfaces are mapped in [`docs/templates.md` → Surface scope](../templates.md#surface-scope) and [`docs/client-editing.md`](../client-editing.md#structural-templates).

Two invocation modes, because they return different things:

- **Structural** (`/template <name>`) deep-copies the subtree with fresh `NodeId`s and stamps `from-template:: <slug>` on each **root** clone only.
  The user gets editable blocks.
- **Callable** (a ` ```call:<name> ` fence) resolves the template's first code fence and executes it through `outl-exec`, with the call block's params injected as a `params` binding via `serde_json`.
  The user gets a computed result.

`templates/journal.md` became the page `templates/journal` carrying `template:: journal`, and `JOURNAL_TEMPLATE_NAME` is the reserved name `page::open_journal` auto-instantiates — untraced, so a daily note is not stamped as derived.

## Why not the alternatives

**A dedicated `Op::DefineTemplate` variant.**
The cost is not the enum arm: every variant is forever across replay, undo, the sidecar, MCP and every client's match, which is why the `/new-op` checklist is long.
`Op::SetProp` already replays, undoes and syncs per-actor, and it makes a template searchable and backlinked with no new surface at all.

**Keep templates as files under `templates/*.md`.**
Cheapest to build, and it violates invariant 7 — a file is not an op, so a template authored on the Mac never reaches the phone.

**A special folder with its own parsing rules.**
A second markdown dialect for one feature, and a page invisible to search, backlinks and the tree.

**Substitute callable params by string replacement in the body** (closest to how Roam renders).
A quote or newline in a param value then breaks the generated program, or injects into it.
Serializing the whole params map with `serde_json` removes the class instead of escaping case by case.

**A ClojureScript runtime to match `{{roam/render}}`.**
`outl-exec` already runs Python, JS, Lisp, Lua and Rust blocks; imitating the syntax of the tool being left behind buys nothing.

## The opposite direction

**`template::` is an ordinary property, so nothing reserves the key.**
Any page carrying it becomes a template, and two pages can claim the same name — resolution then picks one.
`duplicate_template_name_is_detectable` exists because that state is reachable, not because it is prevented.

**Structural instantiation is a copy with no live link, deliberately** — an instantiated 1:1 is the user's notes, not a view.
The consequence: fixing a typo in the template does **not** fix pages already stamped from it, there is no "update instances" path, and the user is not told.
`from-template::` gives provenance, and provenance is not propagation.

**The mirrored case runs the other way for callable templates.**
A ` ```call: ` fence is live, so a user who edits a template to fix one page silently changes every other call site's output on the next run.
Both directions surface through one owner — `backlinks::backlinks_for_page` reads `from-template::` **and** the `call:<name>` fence — so someone checking impact sees both kinds of site in one list.

## How it cannot regress

1. **The rules.**
   `crates/outl-actions/CLAUDE.md` → the `template` row is the owner statement, including both modes and the `serde_json` injection reason.
   `crates/outl-cli/CLAUDE.md` → `outl init` states the journal template is seeded as a **page**, so nobody re-adds `templates/journal.md`.
   Root `CLAUDE.md` invariant 7 is what forbids the file-based version returning.
2. **The tests.**
   Those in **Guarded by** cover the stamp landing on roots only, the untraced journal path, param escaping, alias canonicalization, first-fence resolution, and both provenance directions.

## Scope

**Not covered — the follow-ups #146 listed as excluded:** interactive param prompt in the TUI, template rules that auto-instantiate by path or namespace, inheritance and composition, custom user variables.

**Not covered — updating existing instances** when a structural template changes; no issue owns this yet, and the trade-off is recorded above rather than solved.

**Not covered — property-based filtering** a template picker would want at scale; that is query surface, see [RFC 0139](0139-query-language.md).
