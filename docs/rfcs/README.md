# RFCs

An RFC records **why** outl changed, what was rejected, and what stops the change from being undone by accident.

It is not a design document written before the work, and not a summary written after it.
It ships **in the same PR as the implementation**, because a decision separated from its diff drifts from the code within a release.

## Why RFCs and not ADRs

The repo already had four places recording evolution — `docs/rfcs/0137`, the "Decisions you don't get to revisit" table in the root `CLAUDE.md`, the `CHANGELOG.md` entries, and `docs/design/`.
Adding a fifth format would have broken the [one owner per fact](../contributing.md#one-owner-per-fact--link-dont-duplicate) rule that keeps the shortcut and CLI tables from diverging.

More to the point: an ADR would not have prevented the bug that prompted this process.
[Issue #166](https://github.com/outlmd/outl/issues/166) was documented — issue, changelog entry, code comment explaining the gate.
What was missing was not a record of the decision taken, it was the question nobody asked: *and in the opposite direction?*
So the template makes that question a required section, and the process ties every RFC to an invariant and a named test.

## The flow

1. **Issue** — expose the problem or request the capability.
   The body states the problem only.
   No proposed solution in the body: a solution written into the issue stops being a proposal and starts being the plan, before anyone has argued with it.
2. **Discussion happens in the comments.**
   Alternatives, trade-offs, objections.
   The issue body stays a problem statement so a newcomer can read what was wrong without reading how three people thought about fixing it.
3. **PR carries the implementation *and* its RFC.**
   One PR, not two.
   An RFC-only PR gets accepted and then the implementation drifts from it; an implementation-only PR gets merged and the reasoning is lost to the diff.
4. **Review and merge.**
   The reviewer checks the RFC against the diff, not only the diff.
   The [Delivering work](../contributing.md) bar applies to both.

## Do I need an RFC?

**Yes** when the change touches an invariant, a data format (op log, sidecar, markdown dialect), the CRDT, sync, or a projection path — or when it has a trade-off a future reader could reasonably want to reverse.

**No** for a localized bug fix with no design decision, a dependency bump, a doc fix, or a perf change with no behavioural consequence.
Those live in `CHANGELOG.md`.

When unsure: if you find yourself writing a paragraph in the PR description explaining *why this way*, that paragraph is an RFC.

Retroactive RFCs exist for decisions the codebase already depends on.
They carry the original issue number and are marked `Shipped`; the point is that the next contributor finds the reasoning where they look for it, not that history is rewritten.

## How RFCs stop regressions

The RFC is the *reasoning*.
It cannot enforce anything on its own — nobody reads `docs/rfcs/` while editing `reconcile.rs`.
So every RFC that can cost a user data must land three things together:

| Layer | Where | What it does |
|---|---|---|
| **Reasoning** | `docs/rfcs/NNNN-*.md` | Why the rule exists, what was rejected, what got worse |
| **Rule** | root or per-crate `CLAUDE.md` | Read on every edit to that crate — this is the enforcing surface |
| **Proof** | a named test | Fails mechanically if someone reverts the behaviour |

The `CLAUDE.md` entry links back to the RFC, and the RFC names the tests in its **Guarded by** row.
That round trip is the point: a rule with no RFC has no rationale and gets argued away, an RFC with no `CLAUDE.md` entry is never read at the moment it matters, and either one with no test is a comment.

**When you change behaviour an RFC pinned, you must update the RFC in the same PR** — either amend it, or supersede it with a new one and set the old `Status` to `Superseded by RFC NNNN`.
Silently changing pinned behaviour is the regression this process exists to catch.

## Index

| RFC | Title | Status | Issues |
|---|---|---|---|
| [0002](0002-tauri-for-every-gui-client.md) | Every GUI client is Tauri 2 over one Rust surface | Shipped | #2, #3, #98 |
| [0008](0008-markdown-dialect-and-sidecar-tokens.md) | What it costs to add a token to the outl dialect | Shipped | #8, #52, #64, #65, #10 |
| [0025](0025-plugin-system.md) | iOS bans JIT, so the plugin runtime is an interpreter | Shipped | #25, #4 |
| [0038](0038-sync-transport-and-workspace-identity.md) | iroh is the default transport, and a workspace is an id the joiner adopts | Shipped | #38, #133, #197, #120 |
| [0044](0044-clipboard-and-paste.md) | Copy-out and paste-in are one pair, and the core speaks exactly one format | Shipped | #114, #44 |
| [0070](0070-keybinding-ownership-and-vim-parity.md) | One catalog owns every chord, and the desktop has no character cursor | Shipped (parity unbuilt) | #70, #80, #184 (+5) |
| [0107](0107-page-identity.md) | A page has three identities: its slug on disk, its title on screen, and the date that decides both | Shipped | #195, #107, #50, #88 |
| [0128](0128-boot-and-memory-at-scale.md) | Boot and memory at scale: the snapshot, the index, and the lazy `Doc` | Shipped | #156, #179, #128, #207 |
| [0129](0129-op-log-durability.md) | An acknowledged op must survive the crash, the reader, and the rebuild | Shipped | #157, #192, #129, #122 |
| [0137](0137-storage-scale.md) | Storage scale: constant RSS, then constant boot/sync | Phase A shipped ⚠️ | #137 |
| [0139](0139-query-language.md) | A line-oriented query DSL in a code fence, not datalog | Shipped | #139 |
| [0146](0146-template-engine.md) | A template is a page with a property, not a new op | Shipped | #146 |
| [0155](0155-peer-trust.md) | A paired peer is not a trusted peer | Accepted (2 of 4 holes closed) | #155, #160, #158, #159 |
| [0169](0169-backlinks.md) | Backlinks: one definition of a mention, one index, four clients | Shipped | #169, #180, #142 |
| [0202](0202-file-assets.md) | Asset bytes are content-addressed blobs, deliberately outside the op log | Shipped | #202, #203 |
| [0210](0210-md-content-outside-op-log.md) | A sidecar hash match is not evidence the `.md` came from the op log | Shipped (partial) | #210, #166, #77 |
| [0211](0211-state-that-leaves-a-boundary.md) | State that leaves a boundary arrives somewhere with different rules | Accepted | #211 |

## How the retroactive set was chosen

78 closed issues were triaged against the "do I need an RFC?" bar above.
**27 carried decision content and collapsed into the 14 retroactive RFCs in the index; the other 51 are changelog-only** — localized fixes with no trade-off to record.

The collapsing is the point.
Eight keybinding issues tell one story about `outl-shortcuts` being the single catalog, so they are [RFC 0070](0070-keybinding-ownership-and-vim-parity.md), not eight documents.
Six dialect issues yield one *rule for adding a token* ([RFC 0008](0008-markdown-dialect-and-sidecar-tokens.md)) rather than six token descriptions.

Where the reasoning already lived somewhere good, the RFC is thin and links.
[RFC 0128](0128-boot-and-memory-at-scale.md) defers to `docs/storage.md` § Snapshot strategy instead of restating it, because two copies of a failure-mode table is exactly what the one-owner rule exists to prevent.

Two borderline cases were **rejected on purpose**.
#99 (TUI word wrapping) has a real non-obvious trade-off, but `outl-tui/CLAUDE.md:175-181` already explains it better than an RFC would, and duplicating it would be worse.
#161 is a pure refactor with no behaviour change.

### Gaps these RFCs surfaced

Writing them turned up behaviour nothing pins.
Each is recorded in its RFC's **Guarded by** row as `none found — gap` rather than papered over with an invented test name:

- **#122** — "don't write an op for what the user didn't do" (the ghost-block policy) has no automated test.
- **Asset bytes never entering the op log** holds by construction only: `Op` has no asset variant, and nothing asserts it.
- **`CaTlsConfig::system()`** and the relay 502 path are untested.
- **#169's proposed random-workspace property test** does not exist — `outl-actions` has no `proptest` dependency, and `bench_backlinks` is `#[ignore]`d, so the 3.8 s regression it was written against would pass CI today.
- **The desktop char-cursor nudge** (`shortcuts.support.test.ts`) has no test, nor does #41's arrow navigation.

Two RFCs also correct the record on what shipped.
[RFC 0128](0128-boot-and-memory-at-scale.md) says outright that #179's Front B never landed — `actor_census` still does a full `all_ops()` — instead of claiming the issue is closed.
[RFC 0025](0025-plugin-system.md) records that the shipped runtime **inverted** #25's own proposal (Boa primary, not QuickJS), which was nowhere in writing.

⚠️ **RFC 0137 predates this template** and is the one document here still on its own structure — it has `## Why` and its measurements, but not **Why not the alternatives**, **The opposite direction**, **How it cannot regress**, or **Scope**.
Its header was normalized; converting the body needs someone who can speak to what was rejected in #137, which is not reconstructable from the file.
Until then it is the exception, not the pattern to copy.

## Numbering

The RFC number **is** the issue number it resolves — no separate sequence to keep in sync, and `0210-*.md` is findable from issue #210 without opening a file.
A change that resolves several issues takes the lowest number and lists the rest in its **Issue** row.
`0000-template.md` is the template.
