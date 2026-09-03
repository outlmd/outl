# RFC 0253 — Chord parity is a compile error; feature parity is a rumour

| | |
|---|---|
| **Status** | Accepted |
| **Issue** | [#253](https://github.com/outlmd/outl/issues/253) |
| **PR** | none yet |
| **Date** | 2026-09-02 |
| **Reference doc** | [`docs/client-parity.md`](../client-parity.md) |
| **Invariant** | root `CLAUDE.md` invariant 12 |
| **Guarded by** | `crates/outl-shortcuts/src/capability.rs::tests::all_matches_the_enum`; `crates/outl-shortcuts/src/capability_support.rs::tests::{every_client_declares_support_for_every_capability, every_degraded_capability_explains_itself, capability_nudges_are_written_for_the_user_not_the_developer, no_capability_is_out_of_reach_on_every_client}`; `crates/outl-shortcuts/src/support.rs::tests::the_parity_doc_matches_the_code` (extended to both generated tables); `crates/outl-mobile/src-tauri/src/capability_parity.rs::tests::mobile_capability_column_is_backed_by_real_commands`. |

Sub-project **B** of the client convergence effort (A → B → C → D).
A unified design tokens ([RFC 0022](0022-unified-design-tokens.md), shipped).
B makes feature gaps a compile error, C closes the mobile gaps B enumerates, D aligns the CLI / MCP vocabulary.

## Why

`outl_shortcuts::support` works.
It is an exhaustive `match` over `Action`, so a new variant does not compile until all three clients declare what they do with it, and [`docs/client-parity.md`](../client-parity.md) is generated from it and pinned by `the_parity_doc_matches_the_code`.
Invariant 12 exists because of it.

It covers chords and nothing else.
The parity doc says so, in its own "What this table does not cover" section:

> Only actions in the chord catalog.
> Features with no `Action` — page history, assets, the plugin marketplace, the calendar — diverge across clients the same way and are not yet tracked anywhere.
> That is the next gap, not a closed one.

That sentence has been true for months.
Surveying the component trees:

| Feature | Desktop | Mobile | TUI |
|---|---|---|---|
| Timeline / page history | `TimelinePanel.tsx` | — | — |
| Plugin marketplace | `PluginMarketplace.tsx` | — | — |
| Calendar | — | `Calendar.tsx` | — |
| Templates | — | `TemplateSheet.tsx` | — |
| Properties, reminders | yes | yes | — |
| Backlinks | yes | yes | `ui/backlinks.rs` |

Not one of those rows is written anywhere a build can check.
So nothing fails when a client falls further behind, and the user is the one who finds out — by looking for something that is not there, with no way to tell a gap from a bug.

### The narrower defect underneath

`outl-mobile/src-tauri` does not depend on `outl-shortcuts` at all.
Verified against its `Cargo.toml`: the consumers are `outl-plugins`, `outl-tauri-shared`, `outl-tui` and `outl-desktop/src-tauri`.

So mobile's column in the generated parity table is declared **on mobile's behalf**, by a crate mobile does not build against.
Nothing in mobile's own build breaks when mobile diverges, which makes its column a claim rather than a constraint.

## What we chose

**A second exhaustive `match`, over a new `Capability` enum, reusing the existing `Support` vocabulary unchanged.**

`Support` already models exactly the five states a feature needs, and it was designed for precisely this distinction:

- `Full` — the client does it.
- `Native(why)` — the platform does it, so there is no handler to test for.
- `Partial(why)` — reachable, with less than the full semantics.
- `Missing(why)` — not built here yet, and it should be. The string says what to do instead.
- `NotApplicable(why)` — cannot exist here by construction.

None of that is chord-specific.
`ClientSupport`, `Client::ALL`, `is_reachable()`, `nudge()` and `kind()` are reused as they are.

```rust
/// A user-facing capability that is NOT reachable through a chord,
/// and therefore invisible to `Action`'s parity match.
pub enum Capability {
    PageHistory,
    PluginMarketplace,
    Calendar,
    Templates,
    WorkspaceSearch,
    Assets,
    PeerPairing,
    // …
}

/// What each client does with a capability. Exhaustive: a new
/// variant does not compile until all three clients declare.
pub fn capability_support(cap: Capability) -> ClientSupport { … }
```

**Post-implementation note.**
The shipped enum has six variants, not the seven sketched above: `PageHistory`, `PluginMarketplace`, `Calendar`, `Templates`, `Assets`, `PeerPairing`.
`WorkspaceSearch` did not survive the survey.
The desktop's `Picker.tsx` and the mobile quick switcher are the same fuzzy page-title search already tracked as `Action::OpenPicker` (`Full` on all three clients); in-document find is already `Action::SearchWordForward` / `SearchWordBackward`.
The only search feature with no client surface at all — full-text content search over every page's blocks — lives solely in `outl search --in blocks|pages|all` (CLI) and the MCP `outl_search` tool.
Both are out of scope per "Who does not have this" below.
Adding `WorkspaceSearch` here would have been a second catalog entry for a fact `Action` already owns (the failure mode "Reuse-first" in root `CLAUDE.md` names), so it was dropped rather than forced in to match this sketch.
The TUI's `ui/pages.rs`, cited in the original survey as its `WorkspaceSearch` evidence, turned out to be the sidebar's static alphabetical page listing — not a search feature at all.

[`docs/client-parity.md`](../client-parity.md) gains a second generated table below the first, from the same generator, pinned by the same kind of test.
Its "What this table does not cover" section — the one quoting the gap — becomes false and is replaced by what is genuinely still uncovered.

**`outl-mobile/src-tauri` gains a dependency on `outl-shortcuts`**, so mobile's declarations are checked by mobile's own build.

## Why not the alternatives

**Widen `Action` to include features.**
Rejected because `Action` means "a thing a chord can fire", and `lookup()` resolves chords to it.
Adding `PluginMarketplace` there would put a non-chord in the chord catalog and force every keybinding surface to carry rows that can never have a key.
The two catalogs share a *vocabulary*, not a namespace.

**One generic `parity!` macro over both enums.**
Tempting, and it is the Rule-of-Three trap: two instances is not a pattern.
The two `match` bodies are the *content* — the per-client sentences shown to users — not boilerplate, so a macro would abstract the wrong half and make the sentences harder to read and review.

**A hand-maintained table in `docs/`, since that is what a capability list looks like.**
That is what `docs/client-parity.md` was before it was generated, and it was wrong in three places at once.
The whole reason the chord catalog works is that nothing hand-maintained sits between the code and the doc.

**Leave it, and enumerate gaps in each client's `CLAUDE.md`.**
That is three hand-maintained copies of one fact, which is the state invariant 12 was written to end.

## The opposite direction

**Required section — what this makes worse.**

**A new enum variant now blocks three clients, not one.**
Adding a capability means writing three sentences before the workspace compiles, including for clients whose answer is "not here, and that is fine".
That cost is the mechanism, not a side effect — but it is a real tax on the person adding a feature, and it lands on them rather than on whoever later discovers the gap.

**`Missing(why)` strings will rot.**
Each says what the user should do instead, and "use the desktop for this" stops being true the day the mobile client gets it.
The chord catalog has the same exposure and has held, because the string is compiled and reviewed rather than living in a doc — but this doubles the surface.

**Nothing here touches reconciliation, sync, projection, or the op log.**
No `Op` is added, no `.md` is written, no sidecar is read.
A wrong parity row is a wrong sentence shown to a user; it cannot cost them content.
Recording that plainly, so the next reader knows the question was asked.

## Who was standing on the old behaviour

Per [invariant 10](../../CLAUDE.md) — who consumed the winner?

- **`docs/client-parity.md`'s "What this table does not cover" section** is quoted by this RFC and by issue #253 as the statement of the gap. Closing the gap makes it false; it must be rewritten in the same change, not deleted.
- **`the_parity_doc_matches_the_code`** regenerates the file wholesale. A second table must be inside its markers, or the regeneration silently deletes it.
- **Mobile's build** currently cannot fail on a parity declaration, because it does not depend on the crate. Adding the dependency means a mobile-only change can now be blocked by a declaration it never had to satisfy before.

## Who does not have this

Per [invariant 12](../../CLAUDE.md) — the rule this RFC is an instance of.

The catalog covers the three clients that render an outline.
**The CLI and the MCP server are deliberately out of scope**, and that is sub-project D's subject ([#255](https://github.com/outlmd/outl/issues/255)): they expose operations, not capabilities, and their gap is a vocabulary mismatch rather than a missing feature.

## How it cannot regress

**The rule.** Invariant 12 already states it — *when you add a capability, enumerate who does not have it*. This RFC widens its reach from `Action` to `Capability`; the invariant text gains a sentence naming the second catalog.

**The tests**, all landing with the implementation:

1. **`every_client_declares_support_for_every_capability`** — mirrors its `Action` sibling, and pins `Capability::ALL`'s length so a variant cannot be dropped from the iteration list while its `match` arm survives.
2. **`every_degraded_capability_explains_itself`** — no empty reason strings.
3. **`capability_nudges_are_written_for_the_user_not_the_developer`** — reuses the existing `DEV_WORDS` list, so "unimplemented" cannot reach a user through the new catalog either.
4. **`the_parity_doc_matches_the_code`** extended to cover both tables.
5. **A test that mobile's own crate builds against `outl-shortcuts`**, so the dependency cannot be dropped as unused — nothing else in mobile imports it yet.

## Scope

Not covered here:

- **Closing any of the gaps the catalog records.** Enumerating is not implementing; that is sub-project C ([#254](https://github.com/outlmd/outl/issues/254)).
- **CLI and MCP vocabulary** — sub-project D ([#255](https://github.com/outlmd/outl/issues/255)).
- **Finishing the TUI's migration onto `lookup()`.** The TUI still dispatches Normal-mode keys from its own `match` in `input/normal.rs`; that is open work predating this RFC and is not made better or worse by it.
