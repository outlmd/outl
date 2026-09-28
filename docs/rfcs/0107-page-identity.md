# RFC 0107 — A page has three identities: its slug on disk, its title on screen, and the date that decides both

| | |
|---|---|
| **Status** | Shipped |
| **Issue** | [#107](https://github.com/outlmd/outl/issues/107), [#195](https://github.com/outlmd/outl/issues/195), [#50](https://github.com/outlmd/outl/issues/50), [#88](https://github.com/outlmd/outl/issues/88) |
| **PR** | — |
| **Date** | 2026-08-06 |
| **Reference doc** | [concepts.md § Slugs](../concepts.md#slugs), [config.md](../config.md) |
| **Invariant** | root `CLAUDE.md` invariant 1 (identity changes are `Op`s), invariant 7 (the title converges through the op log, not a shared file) |
| **Guarded by** | `open_by_slug_uses_the_literal_stem` (`crates/outl-tui/src/actions/nav/open.rs`), `concurrent_create_does_not_double_a_regular_page_title` (`crates/outl-actions/src/page_repair_titles.rs`) |

## Why

A page in outl answers to three different names, and nothing in the repo wrote down that they are three.

- The **slug** is the filesystem identity: one path component under `pages/<slug>.md`, so it cannot contain `/`, `\`, control characters, or `..`.
- The **title** is what a human reads and types: `avelino/outl`, `São Paulo`, `Meu Projeto`.
- The **date** is the journal's identity, and it silently decides the slug: today's journal is `journals/YYYY-MM-DD.md`, so "what day is it" is an identity question, not a display question.

Treating any two of those as one thing has cost data four separate times.

**#195 minted duplicate empty pages.**
Opening a page from the TUI quick switcher landed on an **empty** page — just a `title::` line and one empty bullet — while the switcher's own preview pane showed the real content.
Every open also created a new empty page on disk.
The cause is that preview and Enter resolved the page through two different identities.
Preview used the candidate's `key`, the literal `pages/*.md` file stem, and looked it up in `WorkspaceIndex::by_slug`, which is keyed by that raw stem.
Enter called `open_page_by_name`, which **re-ran `slugify()`** on the same stem.
`slugify` is idempotent on its own output, but an on-disk slug is not always its own output: the MCP and CLI `page create` do not slugify by default, so real filenames carry `~`, `%`, and uppercase.
Re-slugifying `ai-memory~019f603c-…~sessions%2F74174d99-….md` produced a path that did not exist, the "not found, create empty page" branch fired, and the user's content stayed in a file nothing would open again.

**#107 looked like a clock bug and was an identity bug.**
The report is one line: it is 21:24 in the UK and outl says 20:24.
The status-line clock was the visible symptom; the real exposure is that the same code decides which journal slug "today" means.
`chrono::Local` reads the OS zone, and containers and Chrome OS Crostini run in UTC regardless of where the user is.
So a user in London near midnight writes into yesterday's journal, and a user syncing a Crostini device against a macOS device has two machines that disagree about which page today's notes belong on.
That is convergence between devices being decided by an environment variable.

**#50 refused the most common way to create a page.**
Tapping a `[[ref]]` or `#tag` on mobile whose text contained `/`, an accent, or a space surfaced a red `invalid page slug` toast and navigated nowhere.
`[[avelino/outl]]`, `[[São Paulo]]`, and `[[meu projeto]]` are routine in any workspace imported from Roam or Logseq, and in any pt-BR workspace.
The mobile command passed raw user input into `is_valid_slug`.
The validator was right to reject `/` — the slug becomes a single path component, so anything that could escape its directory must be refused there.
The bug was feeding a human-typed title to a filesystem validator.

**#88 showed the machine name where the human name belonged.**
Mobile's `[[…` autocomplete rendered `avelino-outl` instead of `avelino/outl`.
The frontend was already correct.
`page_meta` derived the title from the page root node's **text**, falling back to the slug, and never read the `title::` property.
Pages created in-app set the root text, so they looked fine; pages arriving from disk or from an import park the human name in the `title::` property and leave the root text empty.
Every one of those displayed its slug, on every client, in every page list.

## What we chose

Three identities, three owners, and one explicit rule about which direction converts into which.

**Slug resolution is split by provenance, not by convenience.**
`open_page_by_slug` takes an identifier that is already on disk and opens it **verbatim**.
`open_page_by_name` takes a human-typed name and slugifies it.
The TUI quick switcher's `accept_quick_switch` calls the first, because its candidate key *is* the file stem; following a `[[ref]]` or running `/open <name>` calls the second.
`outl_md::slug::slugify` stays the single owner of "human name to filesystem slug", and `outl_actions::page::is_valid_slug` stays the single owner of "may this string be a path component".
Neither is allowed to run on the other's input.

**Human-typed names go through one resolution ladder.**
`outl_actions::resolve::open_or_create_by_name` slugifies for disk and keeps the typed string as the title, so `[[avelino/outl]]` creates `pages/avelino-outl.md` titled `avelino/outl` and opens it in the same tap.
`resolve_or_create_by_name` is the shared `pub(crate)` ladder underneath: literal slug, then slugified form, then case-insensitive title, then create.
`open_or_create_by_ref` is the one owner of the full "user tapped a ref" decision tree, including the date and `@`-mention branches.
`person::ensure_person_by_name` reuses the same ladder rather than keeping a second opinion.

**The title lives in a property, not in the root's text.**
This is the least obvious decision in this RFC and the one that prevents a convergence bug rather than a display bug.
`page_id_from_slug` derives a page's `NodeId` deterministically, so two peers creating the same page offline mint the **same** root.
When the title was the root's Yrs text, both devices ran `edit_text(root, title)` on that shared root, and the two concurrent inserts **concatenated** on merge: `"2026-06-252026-06-25"`.
Yrs is doing exactly its job — two concurrent inserts into one text both survive — which is right for prose and wrong for a name.
So `page::open_or_create` now creates the root with **no text** and stores the title in the `title::` property via `Op::SetProp`, which is last-write-wins by HLC and converges to one value.
A journal never gets a `title::` property at all, because `journal_title == slug`, so its `.md` stays free of a redundant `title:: 2026-06-25`.
`page_meta` reads the title through a most-specific-first ladder: the `title::` property, then the root node's text, then the slug as a last resort so a title is never blank.
That ladder is what fixed #88 for every client at once, since `page_meta` is the single owner of `PageMeta`.

**Two repair passes, because prevention does not reach workspaces already broken.**
`page_repair_titles::repair_doubled_journal_titles` clears any journal root whose text is its own slug repeated two or more times, through `edit_text` — an `Op`, so the fix converges to every device instead of being applied per machine.
`page::merge_duplicate_slug_roots` handles the split-brain case where more than one root claims a slug: it re-parents every child under the canonical root and trashes the emptied duplicates, all through `Op`s, and is idempotent.
Both run on the clients' **background** reconcile pass, never the synchronous boot path, because both scale with page count.

**"Today" is a configured value, not an ambient one.**
`outl_actions::clock` is the process-wide owner of "now" and "today": `init(tz)` once at boot, then `now_local` and `today`.
It resolves `[calendar] timezone` (an IANA name, DST-aware through `chrono-tz`) and falls back to OS local when unset, so the default behaviour is unchanged for users whose OS is honest.
`page::today` delegates here, so the journal slug and the status-line clock cannot disagree.
`dates` stays deliberately clock-free — every function takes the anchor date as a parameter — which is what keeps date parsing testable and keeps "what does today mean" in exactly one place.

## Why not the alternatives

**Make `slugify` idempotent over arbitrary input, then keep re-slugifying everywhere.**
This was the tempting one-line reading of #195.
It cannot work: `slugify` is deliberately many-to-one and lossy, and the on-disk stems in question were never produced by it.
No amount of idempotence makes `slugify("foo~bar%2Fbaz")` equal `"foo~bar%2Fbaz"`, so the duplicate-minting branch would still fire.

**Make the MCP and CLI `page create` always slugify, so every on-disk slug is canonical.**
Correct for the future and useless for the present.
Every workspace already holding non-canonical slugs would keep minting duplicates, and the underlying confusion — a file stem and a typed name resolved by the same function — would still be there for the next caller.
It is also a real capability loss: writing a verbatim slug is how an external tool keeps its own identifiers.

**Relax `is_valid_slug` to accept `/` and accents, so #50 stops rejecting refs.**
This is the fix the bug report's shape suggests, and it opens a path-traversal hole.
The slug becomes a single path component under `pages/`, so accepting `/` means accepting `../../etc` by the same rule.
The validator is the last line before the filesystem; the fix belongs one layer up, in not handing it a title.

**Store the title in the page root's Yrs text and let the CRDT sort it out.**
This is what shipped first, and it is the origin of `"2026-06-252026-06-25"`.
Yrs converges correctly and the result is still wrong, because two concurrent inserts into one string is the right answer for a paragraph and the wrong answer for a name.
The failure is not in the CRDT, it is in modelling a single-valued field as collaborative text.

**Store the title in the `.outl` sidecar instead of the op log.**
Cheaper than an op, and it violates root `CLAUDE.md` invariant 7.
The sidecar is a per-page shared file with last-write-wins semantics under every file transport, so two devices renaming a page concurrently would lose one rename with no record.
`Op::SetProp` also loses one rename, but it does so deterministically by HLC on every device, and the losing write is still in the log.

**Give each page a random `NodeId` instead of deriving it from the slug.**
This removes the shared-root collision that caused the doubling, and replaces it with a worse problem.
Two devices creating today's journal offline would mint two unrelated roots for the same slug, so the day's notes would split into two pages that nothing can merge automatically.
Determinism is what makes offline-first journals converge; `merge_duplicate_slug_roots` exists for the residue.

**Fix #107 by calling `chrono::Utc` plus a stored offset, or by trusting the OS harder.**
A fixed offset is wrong twice a year in every DST zone, which turns a one-hour display bug into a one-hour journal-slug bug on two specific nights.
Trusting the OS is what was already happening, and Crostini is the counter-example.
An IANA zone name through `chrono-tz` is the only option that gets both the offset and the DST transitions right.

## The opposite direction

**Slugification is many-to-one, so #50's fix silently merges distinct names.**
`avelino/outl`, `avelino-outl`, and `Avelino Outl` all resolve to `pages/avelino-outl.md`.
Before this change the second name was refused loudly; now it quietly joins whatever page already holds that slug, and the user who meant a new page gets someone else's.
The resolution ladder makes this more likely, not less, because it also matches on case-insensitive title.
Nothing surfaces the collision, and this is the sharpest unowned risk in this RFC.

**#195's mirror: the two addresses still disagree, the switcher just picked the right one.**
`open_page_by_slug` opens the literal stem and `open_page_by_name` slugifies, which means a page whose on-disk slug is not canonical now has **two** addresses that resolve to two different files.
The quick switcher reaches the real one.
Following a typed `[[ai-memory~abc]]` ref still slugifies and still lands on the canonical-slug file, which may be one of the empty duplicates #195 already created.
Both tests in `crates/outl-tui/src/actions/nav/open.rs` pin this on purpose: `open_by_slug_uses_the_literal_stem` and `open_by_name_still_slugifies` assert the divergence rather than hide it.
The duplicates already on disk are not cleaned up either — see Scope.

**#107's mirror is not fixed, and cannot be fixed here.**
Making "today" configurable removes the container-lies-about-UTC failure and introduces a genuine one: two devices with different `[calendar] timezone` values disagree about which journal slug "today" is, at the same instant, and **both are right**.
The op log converges, so no op is lost — but the day's content splits across two journal pages, and nothing merges them.
A user who travels and does not update the config writes into their home-zone journal.
There is no correct answer available to the software here; a journal is inherently a local-wall-clock concept, and outl has chosen to make the choice explicit rather than ambient.

**A concurrent rename loses one title with no conflict surface.**
`Op::SetProp` is last-write-wins by HLC, so renaming a page on the laptop and the phone while offline keeps exactly one name.
That is the intended trade against text concatenation, and it is a real regression against "no write is ever silently dropped".
The losing `Op::SetProp` is still in the log, so the information is recoverable in principle; nothing in any client shows it.

**`repair_doubled_journal_titles` is a heuristic that deletes text without asking.**
It runs on a background pass and clears root text it judges corrupted.
The judgement is narrow: journal roots only, and only when the text is the slug repeated whole, two or more times.
`leaves_single_and_empty_and_unrelated_alone` pins that a partial repetition like `2026-06-252026`, and any unrelated text, are left alone.
It is still a destructive repair driven by pattern matching, and the user is not asked.

**The read path degrades cosmetically, not destructively.**
`page_meta`'s ladder ends at the slug, so an ingested page whose `title::` op has not arrived yet displays its slug and then renames itself when sync lands.
That is visible churn in page lists and autocomplete, and it loses nothing.

## How it cannot regress

1. **Invariants.**
   Root `CLAUDE.md` invariant 1 covers the repairs: `repair_doubled_journal_titles` and `merge_duplicate_slug_roots` mutate through `Op`s, never by editing `.md` to fix state, which is what makes a repair on one device reach the others.
   Invariant 7 is the one that matters most, and this RFC is one of its worked examples: the title is state two devices can disagree about, so it goes through the op log as `Op::SetProp` rather than into the sidecar.
   The `page` row of `crates/outl-actions/CLAUDE.md` carries the concatenation story inline, including the literal `"2026-06-252026-06-25"`.
   So the next contributor to wonder why a title is not just the root's text finds the answer in the file they are already editing.
   The `page_repair_titles`, `resolve`, `dates`, and `clock` rows of the same file state their halves.
   The `clock` rule is repeated where its consumers live: `crates/outl-config/CLAUDE.md`, `docs/config.md`, and `docs/primitives-actions.md` all say to use `clock::now_local` / `today` instead of `chrono::Local::now()`, and all three name issue #107.
   `crates/outl-frontend-shared/CLAUDE.md` carries the frontend half.
   `refReplacement` inserts a journal's ISO slug and everything else's **title**, naming #88.
   The same file records the matching JS trap: `new Date("YYYY-MM-DD")` is midnight UTC and renders the previous day in negative-offset zones.

2. **Tests.**
   Slug provenance: `open_by_slug_uses_the_literal_stem` and `open_by_name_still_slugifies` (`crates/outl-tui/src/actions/nav/open.rs`) pin both directions, and the first asserts that no slugified duplicate file appears.
   `slug_is_idempotent` (`crates/outl-md/src/slug.rs`) pins that `slugify` is stable on its own output, which is the narrow guarantee the split relies on.
   `rejects_path_traversal_slug` (`crates/outl-actions/src/deeplink.rs`) pins that the validator still refuses what it must.
   Human-typed names: `open_or_create_by_name_slugifies_filesystem_hostile_input` and `open_or_create_by_ref_resolves_existing_via_slugified_form` (`crates/outl-actions/src/resolve.rs`) pin the #50 fix.
   The first also asserts that a second call with the same typed name returns the same node.
   Title identity: `page_meta_prefers_title_property_over_node_text` and `page_meta_falls_back_to_slug_when_no_title_or_text` (`crates/outl-actions/src/page.rs`) pin the #88 ladder in both the found and the missing case.
   Convergence: `concurrent_create_does_not_double_a_regular_page_title` and `concurrent_journal_open_does_not_double_the_title` live in `crates/outl-actions/src/page_repair_titles.rs`.
   Each builds two workspaces, creates the same page on both offline, syncs in both directions, and asserts a single title.
   These two are the tests that fail if someone "simplifies" the title back into the root's text, and they must not be relaxed.
   Repair: `repairs_a_doubled_journal_title`, `detects_doubled_and_tripled_slug`, `leaves_single_and_empty_and_unrelated_alone`, and `is_a_noop_on_a_clean_workspace_and_idempotent` (same file) pin the heuristic's boundaries.
   `merge_duplicate_slug_roots_consolidates_two_journal_roots`, `merge_duplicate_slug_roots_is_idempotent_and_noop_when_clean`, and `find_by_slug_is_deterministic_with_duplicate_roots` (`crates/outl-actions/src/page.rs`) pin the split-brain path.
   Date identity: `london_zone_is_dst_aware` (`crates/outl-actions/src/clock.rs`) encodes the #107 repro literally — 20:24 UTC in summer reads 21:24 in London, and the same instant in winter reads 20:24.
   `date_can_differ_from_utc_across_midnight` is the one that guards the identity half rather than the display half: 23:30 in São Paulo is already the next day in UTC, and `today` must be the user's local date.
   `resolve_unknown_name_falls_back_to_none` pins that a typo in the config degrades to OS local instead of failing the boot.

## Scope

**Not covered — collision warnings on slugification.**
Nothing tells a user that the name they just typed resolved to an existing page rather than creating one.
This is the residual risk of the #50 fix and it has no owner.

**Not covered — cleaning up the duplicates #195 already created.**
The empty `pages/<slugified>.md` files, their sidecars, and their ops are still on disk in any workspace that hit the bug.
They are regular pages, so removal is `outl page delete <slugified-slug>` per duplicate, and no pass finds them for the user.
`merge_duplicate_slug_roots` handles multiple roots sharing **one** slug, which is a different shape from two slugs naming one intended page.

**Not covered — surfacing a lost rename.**
The losing `Op::SetProp` from a concurrent rename stays in the log with nothing reading it back.
Any general "show me writes the merge discarded" capability is unowned across the whole op log, not just for titles.

**Not covered — per-device journal timezone reconciliation.**
Two devices configured to different zones will keep writing to different journal slugs for the same instant.
Whether outl should stamp an authoring zone on a journal, or normalise, or keep the current explicit-config behaviour, is an open design question and belongs in its own RFC.

**Not covered — renaming a page's slug.**
This RFC covers how a page acquires its three identities, not how the slug changes afterwards, which touches `.md` paths, sidecars, and every inbound `[[ref]]`.
