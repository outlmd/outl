# RFC 0139 — A line-oriented query DSL in a code fence, not datalog

| | |
|---|---|
| **Status** | Shipped |
| **Issue** | [#139](https://github.com/outlmd/outl/issues/139) |
| **PR** | — |
| **Date** | 2026-08-06 |
| **Reference doc** | [query.md](../query.md) |
| **Invariant** | none |
| **Guarded by** | `parses_status_todo`, `parses_multiple_filters`, `ignores_comments`, `parses_sort`, `parses_since`, `rejects_unknown_key`, `an_empty_tag_is_rejected_rather_than_matching_everything`, `a_dangling_colon_on_prop_is_an_error_not_a_wildcard` (`crates/outl-exec/src/runtimes/query/dsl.rs`), `split_todo_open`, `split_todo_done`, `split_todo_none`, `tag_and_not_tag_on_the_same_name_can_never_both_match`, `prop_and_not_prop_on_the_same_target_can_never_both_match` (`crates/outl-exec/src/runtimes/query/engine.rs`), `crates/outl-exec/tests/query_negative_filters.rs` (the whole file) |

## Why

[#139](https://github.com/outlmd/outl/issues/139) states the pain plainly: tasks are scattered across many notes and get lost, and the workaround is searching for them by hand.

The issue offered two shapes — a built-in tasks note, or "a code snippet which the user can insert on any page of their liking".
Choosing between them, and choosing a *syntax* for the second, is the decision this RFC records.
A query syntax is a **format**, so it joins the op log, the sidecar and the markdown dialect as something users write and we then owe compatibility on.
[`docs/query.md`](../query.md) documents the syntax; it does not say why this syntax.

## What we chose

A ` ```query ` fence holding one `key: value` directive per line, implicitly ANDed.
Directives shipped today: the filters `status`, `tag`, `prop`, `kind`, `since`, `text` — each also spelled `not-<key>` — plus the controls `sort` and `limit`.
Blank lines and `#` comments are ignored.
The `not-` family landed after this RFC — see [Amended by issue 323](#amended-by-issue-323) below.

Results render as **live embeds** (`!((blk-XXXXXX))`), not copies, so toggling a TODO on the original is reflected everywhere it surfaces.
Query fences carry `auto_run() == true` and run on every page load, because the result depends on workspace state and not on the fence body — which makes source-hash caching wrong by construction.

Single owner: `crates/outl-exec/src/runtimes/query/` (split into `mod.rs` / `dsl.rs` / `engine.rs` when the file outgrew the size ratchet; it was one `query.rs` when this RFC was written).
`dsl.rs` parses, `engine.rs` filters and sorts against a `WorkspaceIndex`, and `QueryRuntime` in `mod.rs` returns `OutputFormat::Embeds` for the orchestrator to render.
The **same** engine is reachable structurally as `outl_exec::run_query_structured` and as `outl.query({…})` from JS, so the DSL is a surface over one engine, not a second implementation of filtering.

## Why not the alternatives

**Datalog, which is what Roam uses.**
Strictly more expressive, and that expressiveness is exactly why Roam queries are a power-user-only feature.
Datalog needs the user to know an entity/attribute schema, and outl has an op log plus a materialized tree, not a triple store.
Exposing one means publishing a queryable schema as a permanent public contract, for a problem stated as "I can't find my tasks".

**Inline `{{query: …}}`, also a Roam-ism.**
A magic token requires a new parser token in the markdown dialect, whose job is to stay standard CommonMark; a fence gets comments, multi-line bodies and editor highlighting for free.
The parser keeps `{{query: …}}` as opaque text and there are no plans to implement it — [`docs/query.md`](../query.md#relationship-to-query-) is the owner of that refusal.

**A boolean grammar with `and` / `or` / `not` and parens.**
It needs precedence rules, a real expression parser, and errors that can explain a mis-nested clause.
The AND-only line list has no precedence to get wrong and every line is independently reportable by index (`rejects_unknown_key`), and it grows without a parser rewrite — a new filter is one `enum Filter` variant plus one match arm.

**A built-in `tasks` page**, the issue's first proposal.
One hardcoded query for one shape, on a page the user cannot place or scope.
The name survives as a language alias: `tasks` and `query` resolve to the same runtime.

## The opposite direction

**What this makes worse: the implicit AND is a ceiling, and it is silent.**
"Open tasks *not* tagged `#someday`" was unexpressible, and the user got no error saying so — an unknown *key* is rejected with a line number, but a missing *capability* just reads as a query returning too much.
That is the failure mode this DSL traded for a syntax nobody has to learn.

That specific example is closed; the ceiling is not.
See [Amended by issue 323](#amended-by-issue-323).

**Cost of the live-view choice.**
Because fences auto-run, a page with five query blocks pays five full `WorkspaceIndex` builds from disk on every open; there is no incremental index.
Sub-second under roughly 1,000 pages, and the shards plan in [`docs/sync.md`](../sync.md#per-page-op-log-shards-for-10k-pages) is the fix before 10k.

**The mirrored case.**
The read path is safe: an embed is a reference, so a query can never duplicate or mutate the block it found.
The write path is where the asymmetry sits: the *result block* is real projected content, so a broad query grows the hosting page's `.md` with one `!((…))` line per hit.
A query narrowing from 300 hits to 3 shrinks that page again, and only the fence itself was authored by the user.

## How it cannot regress

1. **The rules.**
   [`docs/query.md`](../query.md) is the single owner of the directive table, the auto-run rule, and the refusal to implement inline `{{query: …}}`.
   No `CLAUDE.md` invariant covers this — it cannot lose data, so it does not earn one.
2. **The tests.**
   The `dsl` tests pin the accepted keys and the rejection of anything else, which is what stops a directive from being silently ignored.
   The `engine` tests pin `TODO` / `DONE` splitting, the one part of the filter that reads the markdown dialect.

## Scope

**Not covered — `or`, `not`, `between`, filter by page slug, filter by block property.**
`prop`, `page` and `group` are named as planned in [`docs/query.md` → Extensibility](../query.md#extensibility), and `prop` additionally needs the block index to expose properties.
Nothing on that list has an issue yet.

> `not` and `prop` shipped later — `not` as a single wrapper over any filter rather than as grammar, per [Amended by issue 323](#amended-by-issue-323).
> `or`, `between`, `page` and `group` remain uncovered.

**Not covered — inline `{{query: …}}`.**
If ever wanted it is a new parser token, never a reuse of this runtime.

**Not covered — an incremental workspace index**, owned by the sharding plan in [`docs/sync.md`](../sync.md#per-page-op-log-shards-for-10k-pages) and [RFC 0137](0137-storage-scale.md).

## Amended by issue 323

[Issue 323](https://github.com/outlmd/outl/issues/323) shipped a negative for **every** filter — `not-status`, `not-tag`, `not-prop`, `not-kind`, `not-since`, `not-text` — plus the `prop:` that `not-prop` is the complement of.
The parts of this RFC that predate it are corrected above; what the amendment does **not** change is the decision recorded under "Why not the alternatives".

**Negation landed as one wrapper, not as a grammar and not as one variant per filter.**
`not-<key>` is parsed by parsing `<key>` and wrapping the result in `Filter::Not`, whose only engine arm is a `!`.
That is a smaller change than this RFC's growth path anticipated: a new directive is negatable the moment its own variant exists, with no second arm to write and none to forget.
There is still no `or`, no parens and no precedence, so the "boolean grammar" rejection stands on its own terms rather than having been quietly reversed.
The remaining half of the ceiling — "tagged `#meeting` **or** `#code-review`" — is still unexpressible and still silent about it, which is issue 323's own open question 3.

**A negative filter is `!` over the positive's matcher, never a second matcher.**
The first cut of this work hand-wrote `NotTag` and `NotProp`: two of six filters negatable, each carrying its own copy of the predicate.
The wrapper replaced both, and the constraint is now structural rather than remembered — root `CLAUDE.md` invariant 14 records it, and `no_filter_and_its_negation_can_both_match` iterates the whole `Filter` set so a new variant with a broken negation fails there.
It is also why `prop:` had to land in the same change: a negation with no positive counterpart is a filter whose complement cannot be written.

**`tag:` narrowed to a boundary match in the same change.**
It matched by substring, so `tag: ops` also hit `#opsec`. Over-inclusive is harmless for a positive filter and is *silent over-exclusion* once negated, so the substring form could not survive `not-tag:`.
`outl_md::tag::text_contains_tag_or_child` is the single owner of the predicate.

**The block index now exposes block properties**, which is the prerequisite this RFC named for `prop` and the reason it was listed as not covered.
`BlockEntry::properties` carries them, lowercased once at index time.
