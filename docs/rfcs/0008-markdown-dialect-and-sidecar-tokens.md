# RFC 0008 — What it costs to add a token to the outl dialect

| | |
|---|---|
| **Status** | Shipped |
| **Issue** | [#8](https://github.com/outlmd/outl/issues/8) (anchor), [#52](https://github.com/outlmd/outl/issues/52), [#64](https://github.com/outlmd/outl/issues/64), [#65](https://github.com/outlmd/outl/issues/65), [#10](https://github.com/outlmd/outl/issues/10), [#147](https://github.com/outlmd/outl/issues/147), [#116](https://github.com/outlmd/outl/issues/116) |
| **PR** | — |
| **Date** | 2026-08-06 |
| **Reference doc** | [markdown-format.md](../markdown-format.md) |
| **Invariant** | root `CLAUDE.md` invariant 2; `outl-md/CLAUDE.md` invariants 1, 6, 7 |
| **Guarded by** | `roundtrip_preserves_semantics` (`crates/outl-md/tests/roundtrip.rs`), `derive_ref_handle_is_deterministic`, `derive_ref_handle_uses_last_six_chars_lowercased`, `v1_sidecar_loads_and_backfills_ref_handle` (`crates/outl-md/src/sidecar.rs`), `collision_expands_handle_for_loser_and_keeps_winner_resolvable`, `forget_page_does_not_unresolve_winner_on_collision_removal` (`crates/outl-md/tests/block_index.rs`), `block_ref_handle_is_stable_across_repeated_index_builds`, `editing_citing_page_leaves_cited_handle_valid` (`crates/outl-md/tests/block_ref_roundtrip.rs`), `level1_match_preserves_custom_ref_handle_verbatim` (`crates/outl-md/src/diff.rs`), `reword_one_block_and_delete_another_keeps_the_id_and_the_ref_handle` (`crates/outl-md/tests/edit_and_delete.rs`), `alternating_binaries_never_rotate_an_id_or_a_ref_handle` (`crates/outl-md/tests/mixed_version_sidecar.rs`), `comparison_operator_is_not_a_highlight`, `empty_highlight_stays_plain`, `newline_inside_is_not_a_highlight` (`crates/outl-md/tests/highlight.rs`), `emoji_unknown_shortcode_stays_plain`, `emoji_url_https_with_port_no_token`, `emoji_git_ssh_url_no_token`, `emoji_inside_inline_code_stays_literal`, `bang_before_a_link_is_an_image_not_a_stranded_bang`, `obsidian_wiki_embed_is_not_a_markdown_image` (`crates/outl-md/tests/inline.rs`), `quote_prefix_is_not_tokenized_as_an_inline`, `image_carries_alt_and_href` (`crates/outl-md/tests/tokenize_owned.rs`), `single_line_quote_roundtrips`, `multi_line_quote_roundtrips_with_marker_on_each_line` (`crates/outl-md/src/render.rs`), `"renders the resolved text, not the raw handle"`, `"renders the raw handle chip when the ref is orphan (no embeds)"` (`crates/outl-frontend-shared/src/markdown/MarkdownInline.test.tsx`) |

## Why

Six issues in a year asked for the same shape of thing.
`==highlight==` ([#52](https://github.com/outlmd/outl/issues/52)),
`> quote` ([#64](https://github.com/outlmd/outl/issues/64)),
`:shortcode:` ([#65](https://github.com/outlmd/outl/issues/65)),
`@mention` ([#10](https://github.com/outlmd/outl/issues/10)),
and `((blk-XXXXXX))` / `!((blk-XXXXXX))` ([#8](https://github.com/outlmd/outl/issues/8)).
Every one of them arrives as "every other note-taker speaks this, outl renders it as literal text", and every one of them reads like a small parser patch.

The parser is not the expensive part.
Three costs are, and none of them are visible from inside the diff that adds a matcher:

**A new token retroactively changes the meaning of files already on disk.**
There is no per-file dialect version.
The day `==` became a token, every `.md` in every workspace on every device was re-read by a binary that knew one more form than the binary that wrote it.
A user who typed `a == b` in a note last year had that line reinterpreted, everywhere, at once, with no migration and no undo.

**A token that carries an identity is a convergence problem, not a rendering problem.**
`((blk-r6s4a1))` in one file names a block whose ULID appears in no `.md` anywhere — invariant 2 forbids writing it there.
Two devices that have never spoken must independently agree on what that handle means, from the sidecar and the tree alone.
Disagreeing does not render badly; it resolves the citation to the *wrong block*, which is indistinguishable from correct until the user reads it.

**A token with two owners is a bug report, twice.**
[#116](https://github.com/outlmd/outl/issues/116) ("Block refs not working in Desktop but working in CLI") and [#147](https://github.com/outlmd/outl/issues/147) ("rendered as raw handle chips") are the same defect filed by two people.
The desktop tokenized `((blk))` and `!((blk))` correctly — it even drew distinct chips for the two forms, and its `((` autocomplete found targets workspace-wide.
The TUI resolved and expanded the same bytes from the same workspace.
The parser was never wrong; the *resolution* existed once instead of once-per-client-wrapping-one-owner.

So the useful artifact is not documentation of six tokens.
It is the rule that decides what adding the seventh costs.

## What we chose

The per-token reference lives in [`docs/markdown-format.md` → Inline syntax](../markdown-format.md#inline-syntax), and the crate's responsibilities live in [`outl-md/CLAUDE.md` → Outl markdown dialect](../../crates/outl-md/CLAUDE.md).
This section is the decision rule those two do not state.

**1. The disk form is the source form, never the rendered result.**
Owner: `inline_to_source` (`crates/outl-md/src/inline.rs`) and `outl_md::render`.
The `.md` stores `:tada:`, not 🎉; `((blk-r6s4a1))`, not the resolved text; `assets/<hash>.png`, not the bytes; `==foo==`, not a colour.
This is the same discipline as invariant 2 applied to content instead of ids, and it is what keeps a file greppable, diffable and font-independent across devices.
It is also non-negotiable in the reverse direction because rendering is frequently **not injective**: `:+1:` and `:thumbsup:` both resolve to 👍, so a renderer-to-source translation has to guess, and it guesses wrong for one of the two users.

**2. No new field on the AST, no new field on the wire.**
`OutlineNode` keeps `text`, `properties`, `children`.
An inline form is an `InlineTok` variant plus the matching `InlineToken` variant plus its `InlineToken::from_borrowed` arm, all three in the same change.
A block-level form is a **text prefix** consumed by a helper — `outl_actions::todo` (`TODO ` / `DOING ` / `DONE `) and `outl_actions::quote` (`> `) are the two that exist.
`DOING ` ([#235](https://github.com/outlmd/outl/issues/235)) is the cheapest possible instance of this clause, and the reference case for it.
It took a third arm on `TodoState`, one more stop in `cycle_todo`, and **zero** changes to the parser, the renderer, the sidecar or any DTO.
What it did cost was every place that had quietly assumed the state was binary — an `Option<bool>` in the query engine and in the TUI's view layer, a `match` on two arms in `quote.rs`, and cursor math assuming all markers are five characters wide.
The lesson generalises: a block-level prefix is free to *parse* and is priced by how many consumers encoded its cardinality in a type.
No `Op` variant, no sidecar version bump, no DTO migration on mobile and desktop.
Issue [#8](https://github.com/outlmd/outl/issues/8) put this in its own body: references are *content* of a block, not *structure* of the tree.

**3. Parse permissively; every ambiguity resolves to `Plain`.**
The tokenizer never errors and never warns.
`==` adjacent to a space stays plain, so `a == b` survives.
`:foo:` stays plain unless `outl_md::emoji::shortcode_to_unicode` finds it, so `meeting at 14:00` and `git@github.com:avelino/outl.git` survive.
`![[img]]` is left to `wikilink::convert_image_links` rather than claimed by `try_image`.
The direction is chosen: a false negative leaves the user's characters exactly as typed, and a false positive eats prose that cannot be recovered by re-reading the file.

**4. A round trip through an external tool is the acceptance test.**
`roundtrip_preserves_semantics` runs 200 generated pages through `render(parse(md))`.
The file must stay editable in plain VS Code with outl absent, and the matching pipeline must reconstruct ids afterwards.
That is why the quote marker is re-emitted on every continuation line, keeping the block a valid CommonMark blockquote instead of an outl-only shape.

**5. A token needs a sidecar bridge only when it names something whose identity is not its own text — and then the derivation must be a pure function of that identity.**
This is the clause that separates a cosmetic token from a convergence-critical one, and it is the whole reason #8 anchors this RFC.
Ask what the token names:

| The token names | Machinery it needs | Example |
|---|---|---|
| Text | None — `slugify` already resolves it | `[[page]]`, `#tag`, `[[@name]]` |
| A node in the tree | Sidecar handle, deterministic derivation, collision policy, orphan reporting | `((blk-XXXXXX))`, `!((blk-XXXXXX))` |
| Nothing (presentation) | Tokenizer only, and it must not be indexed | `==highlight==`, `> quote`, `:shortcode:` |

`derive_ref_handle(NodeId) -> String` (`crates/outl-md/src/sidecar.rs`) is the single owner of the middle row: `blk-` plus the last six characters of the ULID's Crockford base32, lowercased.
Same input, same output, on every device, forever — `outl-md/CLAUDE.md` invariant 7 states it as "two devices building the sidecar independently must agree on what `((blk-XXXXXX))` means".
A ULID tail is also stable under *any* text edit, which is what makes a citation survive a reword; the sidecar carries the handle across level-1 and level-2 matches verbatim (invariant 6) so it survives an external save too.
An orphaned handle degrades visibly — dimmed in the TUI, a neutral chip in the GUI clients — and `outl doctor` lists it via `check_orphan_block_refs` (`crates/outl-cli/src/cmd/doctor/files.rs`).

**6. One owner, every client wraps.**
`outl_md::tokenize_owned` produces `InlineToken`, and each client maps it to its own primitive: `ratatui::Span` in the TUI, JSX in `<MarkdownInline />` (`@outl/shared`).
The fix for #116 and #147 was not a parser change — it was the desktop consuming the resolution path (`resolve_embeds` → `EmbedMap` → `<MarkdownInline embeds= />`) that the TUI already used.
A client that tokenizes a form and then decides for itself what it means has forked clause 5 without saying so.

## Why not the alternatives

**Make the `@` part of the person page's identity** — `pages/@avelino.md`, `title:: @avelino`, and a slugifier taught to preserve a leading `@`.
This is what issue [#10](https://github.com/outlmd/outl/issues/10) proposed in its own body, down to the note that `crates/outl-md/src/slug.rs` "has to keep the leading `@` instead of stripping it as punctuation".
The cost is that `slugify` gains a punctuation exception every caller inherits, and `avelino` and `@avelino` become two pages.
A user who writes `[[avelino]]` then misses every mention, and a user who writes both has silently split one person in half.
Shipped instead: the `@` is a link affordance the resolver strips before lookup, exactly like the `!` in `!((blk-XXXXXX))` is an affordance and not part of the handle.
Zero new machinery, and mentions *are* backlinks for free.

**Give highlight and emoji a semantic field so they are queryable.**
A `kind` on `OutlineNode`, or a token index alongside the text index.
Cost: a wire-format migration across the mobile and desktop DTOs, plus a second definition of "what is a highlight" for FTS to drift away from.
It also invites a `SIDECAR_VERSION` bump, which every already-shipped binary then refuses — see `outl-md/CLAUDE.md` → Sidecar versioning.
Declined in #52's non-goals, and the composition argument holds: `==important [[topic]]==` still yields the `[[topic]]` backlink because the wrapper is transparent to `extract_refs`.

**Give quote a block-level AST variant instead of the `> ` text prefix.**
Cost: a fourth field on `OutlineNode`, a change in every DTO and every client render path, and a `.md` that stops being per-block CommonMark unless the renderer re-invents the prefix anyway.
The prefix already had a working precedent in `TODO ` / `DONE `, and a prefix has a property a struct field cannot have: the user can delete it in a plain editor.
That property is why the historical "auto-preserve the TODO prefix" behaviour was removed — it made the marker impossible to erase.

**Store the glyph on disk and translate back to the shortcode on display.**
The inverse of clause 1, and lossy by construction: the reverse map has to pick one of `:+1:` / `:thumbsup:` for 👍.
It also throws away the ASCII-on-disk property that survives an iCloud encoding round trip, which #65 called out as a sync-safety reason rather than a cosmetic one.

**Anchor-form block references — `[[page#human-slug]]`, the slug derived from the block's text and stabilized in the sidecar.**
Issue #8 listed this first among the options.
The handle then changes whenever the text changes, so a reword breaks every citation — the exact case level-2 matching exists to survive, re-created inside the citation format itself.
A ULID tail changes only when the block's identity changes, which is never, short of a level-3 fall-through that `orphans.log` already records.

**Write the ULID into the `.md`.**
Refused by invariant 2 before any code existed, and it is the Logseq `id::` shape the project rejected explicitly.

**Mint a random short handle at citation time and record it only in the sidecar.**
Cost: two devices that both cite the same block mint different handles, and the sidecar is synced as one last-write-wins blob per page, so one of the two citations loses its bridge.
Determinism from the `NodeId` is what removes the need for the devices to coordinate at all.

**Resolve handles client-side, in TypeScript, from a locally built map** (the shortest fix for #116 / #147).
Cost: a second implementation of handle-to-block, which is precisely the drift that produced two independent bug reports about one feature.

## The opposite direction

**Permissive parsing makes silence the failure mode.**
`=highlight=`, `:notarealemoj:`, `> `without the space, a mistyped `((blk-r6s4a2))` — none of them error and none of them warn.
`ParseWarning` covers block-level oddities (an unrecognized marker at depth 0), and **inline ambiguity produces no warning at all**.
Three of those four typos degrade to plain text, which the user sees and fixes.
The fourth degrades to an orphan chip, and that one has a report path (`outl doctor`, `check_orphan_block_refs`) because a dangling citation is not visibly a typo.
A user who mistypes `==` learns from nothing.
That asymmetry is deliberate — eating `a == b` or `https://host:8080/` is unrecoverable from the file, and leaving characters alone is not — but it is a real gap and it is stated here so nobody has to rediscover it.

**What this now permits that it did not before: retroactive reinterpretation, on every device, at once.**
A `.md` written by an older binary is re-read by a newer one that knows more forms, and text that was plain becomes a span with no migration step and no per-file opt-out beyond wrapping the run in `` ` ``.
The alternative is a dialect version recorded in the file, which is metadata in the `.md` and therefore invariant 2's problem.
Nothing tracks this today.

**The mirrored case for the handle, stated explicitly.**
Clause 5 secures "two devices agree what a handle means".
The mirror is "two blocks claim one handle", and it happens: six lowercase base32 characters is roughly 30 bits, about 5×10⁻⁶ birthday probability at 100k blocks.
Expansion resolves it: the losing block's handle grows one character at a time from its own ULID tail until unique.
The choice of loser is by ULID order, so it is the same on every device regardless of traversal order (`crates/outl-md/src/block_index.rs`).
Both blocks stay independently resolvable.
The honest cost: the sidecar still records the deterministic 6-character form, so the expansion lives in memory until a later reconcile persists it.
A `.md` citing an expanded 7-character handle therefore resolves only on a device whose index has already seen *both* colliding blocks.
Until the colliding page syncs, that citation is an orphan chip.
Orphan-then-resolves was chosen over resolves-to-the-wrong-block, because the second one is invisible.

**If a token is refused, what does the user do instead?**
Nothing here refuses — that is the point of clause 3 — so the answer is that the characters stay as typed and the user retypes them.
The one path that *can* refuse is the sidecar reader: `reconcile_md` propagates `UnsupportedVersion` rather than treating a newer peer's file as sidecar-less, because rebuilding over it rotates every handle in the page.
That refusal is deliberate and pinned by `alternating_binaries_never_rotate_an_id_or_a_ref_handle`.

## How it cannot regress

1. **Invariants.**
   Root `CLAUDE.md` invariant 2 is the enforcing surface for clauses 1 and 5 — markdown stays 100% clean, ids live only in the `.outl` sidecar — and its anti-patterns name "writing IDs into the `.md` file (just for now)" outright.
   `outl-md/CLAUDE.md` carries the three that matter per-token.
   Invariant 1 is roundtrip stability (clause 4).
   Invariant 6 is `ref_handle` preserved across level-1 and level-2 matches (clause 5).
   Invariant 7 is `derive_ref_handle` determinism, spelled out as the two-device agreement.
   Its "Things to never do here" list is where an editor of `inline.rs` reads them.
   The token table itself has one owner, [`docs/markdown-format.md` → Inline syntax](../markdown-format.md#inline-syntax); a token that ships without a row there has already forked clause 6.

2. **Tests.**
   Grouped by the clause they defend, and every one of them fails if the clause is "simplified" away:
   - Clause 1 and 4 — `roundtrip_preserves_semantics` (200 generated cases) fails the moment a token renders to anything other than its source form.
     `image_round_trips_through_inline_to_source`, `emoji_inline_to_source_round_trip`, `highlight_round_trips_to_source`, and the four quote roundtrips in `render.rs` pin the individual forms.
   - Clause 3, the "ambiguity resolves to `Plain`" battery.
     Highlight: `comparison_operator_is_not_a_highlight`, `empty_highlight_stays_plain`, `newline_inside_is_not_a_highlight`.
     Emoji: `emoji_unknown_shortcode_stays_plain`, `emoji_url_https_with_port_no_token`, `emoji_git_ssh_url_no_token`, `emoji_inside_inline_code_stays_literal`.
     Image and quote: `obsidian_wiki_embed_is_not_a_markdown_image`, `bang_before_a_link_is_an_image_not_a_stranded_bang`, `quote_prefix_is_not_tokenized_as_an_inline`.
     This battery is the record of every ambiguity someone already tried to resolve the other way.
   - Clause 5 — `derive_ref_handle_is_deterministic` and `derive_ref_handle_uses_last_six_chars_lowercased` fail if the handle ever becomes random or index-order dependent.
     `collision_expands_handle_for_loser_and_keeps_winner_resolvable` and `forget_page_does_not_unresolve_winner_on_collision_removal` pin the collision policy in both directions.
     `level1_match_preserves_custom_ref_handle_verbatim` and `reword_one_block_and_delete_another_keeps_the_id_and_the_ref_handle` fail if matching stops carrying the handle across an external edit.
     That is the failure the anchor-form alternative would have had by construction.
   - Clause 6 — the `MarkdownInline.test.tsx` cases pin GUI-side resolution (#147): the resolved text rather than the raw handle, the neutral chip when orphan, and the same behaviour on both the `inline` and `pill` variants.

   **Named gap.**
   Nothing mechanically checks that a new `InlineTok` variant reached the owned `InlineToken`, the TS union, and every client renderer.
   The compiler catches a missing `from_borrowed` arm; it does not catch a variant mapped to `Plain` on the owned side, and it cannot see TypeScript at all.
   That pair is convention plus review today, which is exactly the shape #116 and #147 took.

## Scope

**Not covered — where asset bytes live.**
`![alt](url)` obeys this rule like any other token.
The file it points at is a deliberate exception to invariant 7 and lives in [RFC 0202](0202-file-assets.md), issues [#202](https://github.com/outlmd/outl/issues/202) and [#203](https://github.com/outlmd/outl/issues/203).

**Not covered — autocomplete and triggers.**
The `[[`, `#`, `@`, `((`, `:` popups and their word-initial rules are client wiring, together with the shared `detectRefContext` / `applySuggestion` helpers.
Owners: [`docs/shortcuts.md`](../shortcuts.md), [`docs/clients.md`](../clients.md), `crates/outl-frontend-shared/CLAUDE.md`.

**Not covered — query tokens.**
`{{query: …}}` is parsed as opaque text and the ` ```query ` fence is the supported path: [`docs/query.md`](../query.md), issue [#139](https://github.com/outlmd/outl/issues/139).

**Not covered — forms declined in the source issues.**
Nested quotes (`>> `), multi-color highlight, and a keybinding per marker were all non-goals in #64 and #52.
No issue tracks them.

**Not covered — a per-file dialect version.**
See The opposite direction.
Nothing tracks it, and it would need an issue before anyone attempts it, because it puts metadata back in the `.md`.
