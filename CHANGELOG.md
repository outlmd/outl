# Changelog

All notable changes to outl are documented here.
Format inspired by [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- **`outl recover` — brings back block text that an `Op::Edit` truncated, reading the op log rather than the `.md`.**
  Everything else that recovers unlogged content (`outl reconcile --ahead-of-log`, the `doctor` listing) works by reading the `.md`, which only helps while the `.md` still holds it.
  A page overwritten before the re-projection guard existed was assumed lost.
  It is not: the log is append-only, so the truncating edit did not replace the earlier one, it followed it.

  The signature is deliberately narrow — not "the text shrank" (deleting text is something users do) but "the current text is a **prefix**, character for character, of an earlier revision", which is what dropping everything after a point produces.
  On a real 2,560-page workspace the whole-graph scan returns 8 blocks out of 67,213, four of them the multi-line briefings the original report was about.
  Restoring is strictly additive by construction, and `restore_truncated_block` re-checks that instead of trusting it.
  Read-only by default; `--apply` writes, and writes go through `block::edit_text` — a new op, never a rewrite of the log.

- **`outl doctor --repair --force`, and a volume count before anything is written.**
  The run that removed 1,426 lines from 233 pages printed `708 fixed` and no line count at all.
  `doctor` now measures, per page, how many content lines the new projection would not reproduce, and reports it before writing — in `--repair` and in the default read-only mode alike.
  Past 100 lines or 20 pages that lose content, page repairs stand down and `--force` is required.
  A page the write only adds to counts as zero, so a device that just paired with the whole graph unprojected never trips it.

### Fixed

- **A machine running only `outl mcp serve` never synced with anything.**
  It wrote its ops to disk and no peer ever saw them; peers' ops never arrived either, and `outl peer status` on the other device just said "offline" with nothing to explain it.
  Running the TUI once made the device appear, which is what made it look like the MCP server was broken rather than deliberately silent (issue #220).

  It was deliberate, and the reasoning was sound as far as it went: iroh routes one endpoint per device identity, and every outl process here shares one, so a second endpoint steals the relay route and breaks the holder's sync **in both directions**.
  The fix for that was to name the MCP server the loser — the GUI binds, everyone else writes to disk and lets it push.
  That works right up until there is no GUI, and an agent driving `outl mcp serve` on a dedicated machine is a normal way to run outl.

  The constraint was never "only the GUI" — it is "one live endpoint per device", which is a question about who got there first, so it is now answered by a lease (an advisory lock next to `identity.key`) instead of by client type.
  Every long-lived client asks `outl_sync_iroh::build_transport`; the winner binds the endpoint and announces its writes, and the losers run the file poller and converge through the shared `ops/` dir exactly as before.
  A GUI opened at login still keeps the endpoint, so nothing changes on a desktop machine.
  With no GUI, the MCP server is the peer.
  `outl sync` joins the same election and now stands down when it loses, instead of taking the route from a running process for 25 seconds — the process it would be taking it from was already pushing those ops out.
  `outl peer status` takes the same lease for its probe and reports whoever already holds it, instead of racing a live transport for the endpoint.

  **This moves the iroh identity under `$OUTL_DEVICE_DIR`, and that rotates the device's node id for anyone already setting that variable.**
  `~/.outl/identity.key` now lives at `$OUTL_DEVICE_DIR/iroh/identity.key` when the variable is set — a container or sandboxed CI job that exports it comes back up under a new node id on first run and reads as offline to every existing peer until it is re-paired.
  See [`docs/storage.md`](docs/storage.md) → `$OUTL_DEVICE_DIR`.

- **Locking your phone no longer cuts the sync off mid-exchange.**
  iOS suspends an app's sockets within seconds of backgrounding, so the delta sync in flight when the screen locked simply died.
  Nothing was lost — the responder confirms durable ingest by closing cleanly, so an unconfirmed push is re-sent — but the desktop reported `peer did not confirm durable ingest` every single time, and the ops waited for the next window.

  The two `BGTaskScheduler` windows could never have covered this: both are requests for a window *later*, at the scheduler's discretion.
  The missing piece was a `beginBackgroundTask` assertion taken on `didEnterBackground`, which keeps the process resident for one last pass — and, just as importantly, lets an **inbound** sync a peer is mid-way through reach its confirmation instead of dying with the frame unsent.

  Getting the release condition right turned out to be the hard part, and the first version had it wrong.
  It waited for the forced-pass counter to *advance*, but `sync_now()` was fire-and-forget with no identity and the counter is global, so the flush read the **foreground** pass (mobile fires one every 3s) as its own completing, released the window ~250ms in, and let iOS suspend with its own request still queued — ending the sync it existed to finish.
  `sync_now_seq()` now returns the sequence number of the request it enqueued, and the caller waits for `completed_sync_passes() >= seq` **plus** `peers_in_flight() == 0`, since a forced pass skips peers that already have a dial running and can therefore complete having dialed nobody.
  The window is sized from `backgroundTimeRemaining` rather than a constant: one unreachable peer costs 5s direct + 10s relay, so a fixed 20s cap blew past a real budget with two peers and guaranteed the tear-down it was meant to prevent.

- **Android gets background sync too, and it is the first Android-specific feature in this changelog.**
  Android doesn't suspend sockets — it *freezes* the cached process (cgroup freezer, API 30+, ~10s after caching on 34+) — which produces the identical torn-down exchange.
  Going to the background now requests an expedited `OneTimeWorkRequest` to finish the pass in flight, and keeps a 15-minute `PeriodicWorkRequest` for catch-up; both are skipped with zero paired peers.

  **It is weaker than the iOS path and the docs say so rather than implying parity.**
  The handover is a *request*, not an instant grant: between `ON_STOP` and the job starting the process can already be frozen, so the pass gets restarted rather than finished and the peer may still log one timeout.
  Expedited quota is finite (~30 min/24h on Active), below API 31 there is no expedited path at all (a foreground service for a 20s sync would need a permanent notification), and force-stop drops everything.
  The periodic worker has a sharper limit worth knowing: unlike iOS's `BGProcessingTask`, WorkManager starts the *process* but no Activity, so Tauri never boots and no transport is registered — it is a catch-up for a **frozen** app, not a killed one.

  `bg_sync.rs` is now one platform-agnostic core with `cfg`-gated exports (iOS C ABI, Android JNI), so the shared bodies stay covered by the host test suite where neither platform's exports compile.
  `androidx.work` is pinned to **≤ 2.10.5**: 2.11.x pulls `kotlin-stdlib:2.1.20`, whose metadata the pinned Kotlin 1.9.25 compiler cannot read, and that breaks every file in the module including Tauri's generated ones.

- **A peer that went away mid-sync is amber, not red — and a peer that *refused* you is no longer silent.**
  The Sync panel's only red row was, in practice, the one a locked phone produced on every screen lock: a working sync, reported as a failure.
  It now renders as `interrupted` ("went away — will retry"), stays out of the activity feed, and does not pretend to be success — the pass is still an error internally and the peer is still re-pushed.

  The inverse was worse.
  A peer that **refuses** the dial (you were removed from its `peers.json`, or the workspaces differ) closes before writing a byte, so the initiator failed on a *read* and never reached the close-reason check — emitting nothing at all.
  The one failure a user has to act on was the only one with no line in the panel.

  Also moved the responder's `peers.json` refresh to **after** it confirms ingest, and onto a blocking thread.
  It was blocking file I/O sitting between the fsync and the confirmation, widening the exact window in which a suspending peer ingests durably but dies before saying so.

- **A redirected `$OUTL_DEVICE_DIR` now says so, once, at startup.**
  The rotation above is intended, and it was also completely silent: the node id is only announced when it is *generated*, and `iroh endpoint bound node_id=…` never says which key it came from.
  The one place it bites hardest is this repo — `.cargo/config.toml` exports the variable so the suite stays off the developer's real store, and cargo exports it to `cargo run` too, so a desktop built from source is a **separate device** whose key lives under `target/` and dies with `cargo clean`.
  Pairing a phone against that build works, then stops working after the next clean, with the phone reporting only "offline".
  `default_device_dir()` now logs one `WARN` naming the directory and how to opt out (`OUTL_DEVICE_DIR=`).
  See [`docs/development.md`](docs/development.md) → Testing P2P sync from a source build.

- **The first boot after an upgrade froze the app for 24 seconds.**
  outl's premise is that it opens fast and is ready for input, and the `CURRENT_PIPELINE_VERSION` bump broke it: every sidecar goes stale by pipeline, so the first boot re-reconciles the whole graph.
  Measured on a 2,827-file workspace: **24.7 seconds at 8% CPU** — not computation, but `write_atomic`'s two `fsync`s per sidecar, 5,656 of them back to back, for 44 ops of actual content.

  Being on a worker thread was not enough. The pass reacquired the lock immediately after each page and kept the disk saturated for the whole batch, and the UI reads that same disk to paint, so the app felt frozen regardless of which thread the work was on.

  Dropping the sidecar `fsync` would take it to 0.3 seconds and is the wrong trade: a rename landing before its data leaves a sidecar of garbage, which reads as a *missing* one, which mints a fresh ULID per block — the page duplicates and every `((blk-…))` handle breaks.
  Skipping the migration is worse, since the parser fix then never reaches pages nobody opens.

  So the pass yields instead. `BackgroundPace` sleeps in proportion to the work each page cost, holding a bounded share of the device, and the loop takes the workspace lock with `try_lock` so a click never waits behind a migration.
  A slow disk makes it yield more rather than stutter more, because the ratio is the constant, not the delay.
  Nobody needed the migration to be fast; it needed to be invisible.


- **Markdown containing a blank line inside a block, or a block whose text carried its own indentation, lost everything after that point — silently, and then permanently.**
  This is the producer behind [issue #210](https://github.com/avelino/outl/issues/210), and it was not where [RFC 0210](docs/rfcs/0210-md-content-outside-op-log.md) guessed.
  `render → parse` was not a roundtrip:

  ```text
  input:    a block whose text carries a blank line and its own indentation
  render:   correct — every line emitted
  parse:    one block, first line only
  warnings: 0
  ```

  The `warnings: 0` is the part that mattered.
  The parser's contract is that nothing is dropped in silence, and three separate arms of `parse_block_list` broke it: an over-indented line was recovered only at depth 0 and skipped mutely below it, a blank line inside a block's text was read as a separator (the renderer writes it indented; a real separator is empty — the indent tells them apart), and a continuation line's own indentation pushed it out of reach of the level that could have claimed it.
  A fourth arm warned about an unplaceable line but dropped it from the AST anyway, on the reasoning that a guard in another crate would keep the bytes on disk.

  The reconcile that followed then wrote the truncation into the op log as an `Op::Edit`, so the loss reached the one place the RFC had described as still holding the content.
  Measured on a real 2,827-file workspace, running the same normalisation over both branches: pages holding unlogged content went from **41 to 8**, lines from **387 to 49**.

  **The first version of this fix traded the bug for a worse one, three times over**, and none of the 237 green tests saw it — a five-minute probe against the real workspace saw all three.
  `render → parse` has to be a **fixpoint**, not merely lossless, and the original bug at least converged while these mutated the document on every save:
  a bullet at an irregular indent (`    - child`) was recovered as verbatim *text* carrying its own `- `, so each pass read one more level of nesting (`- parent\n  -     - child`, then `- parent\n  - - child`);
  a whitespace-only line before a sibling appended a trailing `\n` to the previous block, which is invisible to the reader and a different `content_hash` to the log, so every page with that shape emitted an `Op::Edit` forever;
  and a recovered block kept the leading indent the renderer then wrote *after* its own marker, so the file settled on the second save instead of the first.

  The property is now stated as one — `render_then_parse_is_a_fixpoint_not_just_lossless` — and verified against all 2,827 `.md` files of a real workspace: **0 unstable, 0 losing a line**.

- **Known cost, stated rather than filed away: withholding the hash invites an older peer in.**
  A binary from before this release reads `last_synced_hash: ""` as "needs reconciling", reconciles with its own lossy parser, and emits an `Op::Edit` truncating a block the log held correctly.
  Measured against a worktree of v0.11.0-beta.151, the build users actually have.

  No value of the field avoids it: the old binary's two gates are complementary (`reconcile_md` when the hash mismatches, its re-projection when it matches), and a real hash is worse — it authorises deleting the unlogged content outright.
  Two things bound the damage: no old write path preserves a `pipeline_version` it does not understand, so the page stays queued and a current binary heals it from the `.md`; and the truncation is an `Op::Edit`, so `outl recover` can read the revision before it.
  Calibration on 2,827 real files: **0 pages trigger the withholding today** — this is a guard for the next parser gap, not for a live condition.

- **The volume guard reported `0` lines removed in the exact scenario it was built for.**
  `outl doctor` builds its reference from a **render**, and `content_lines_missing_from` carried a stand-down for "this reference cannot answer" — written for a pre-0.11 sidecar, where it is right.
  A render of empty blocks hit the same branch, but it is a definitive answer ("the tree holds no text"), not an absence of one.
  So a repair that would empty a page printed *"removes nothing"* and sailed past the `--force` threshold — the run that guard exists to stop.
  Measured: 4 content lines on disk, tree rendering `-\n-\n`, reported 0.

  The condition now lives at each call site, next to the knowledge of where the blocks came from, which is the only place that can tell a sidecar from a render.

- **The post-mutation guard wrote anyway when the sidecar could not be read.**
  Missing, corrupt, and written-by-a-newer-binary are three states where "does the log know this line" has one honest answer: *I cannot tell*.
  The first version used `if let Ok(sidecar)` and fell through to the write on all three, so the page the read path protects was overwritten on the next keystroke commit — the door this guard was added to close, standing open on a different hinge.
  Liveness is not at risk: `needs_reconcile` maps an unreadable sidecar to `true`, so the orphan pass rebuilds it and the page projects on the pass after.

- **Withholding the hash made the page invisible to every warning the same release added.**
  `reconcile_md` writes `last_synced_hash = ""` when it read content it could not log, and every gate downstream tests hash-equality — so `apply_page_md_with_sidecar_if_stale` returned `Ok(None)`, no `PageMarkdownAheadOfLog` was raised, and the banner never appeared.
  `doctor` counted the page as an ordinary pending edit and never named it.
  The producer fix was erasing the signal built to report it.
  Both now treat the empty hash as what it is — withheld, not stale — and ask the content question anyway.

- **A local edit deleted the unlogged lines the open path had just refused to touch.**
  The re-projection guard covers the paths that *read* a page.
  The background projection writer runs after a real mutation and wrote unconditionally, so the very deletion `apply_page_md_with_sidecar_if_stale` declines happened anyway on the next keystroke commit — same invariant 8, a door nobody had checked.
  Found while wiring the warning banner for the first door.

  `apply_page_md_with_sidecar_guarded` is the post-mutation counterpart, and it cannot just call `_if_stale`: that one declines whenever the `.md` carries an unreconciled external edit, which is the state a page is in *while the user types into it*.
  So it asks the one question that matters — does the file hold content the log cannot account for — and skips the projection if it does.
  The edit is never at risk either way: it went through `Workspace::apply` and lives in the op log, and only the on-disk projection lags, which is the recoverable direction.
  Every GUI write path now routes through it (`ProjectionWriter`, block move, template instantiate).

- **The unlogged-content check reported content the log already had, and that verdict freezes a healthy page.**
  A bullet inside a code fence lives in the block's text **with its marker**, because the renderer only adds one for a block's first line.
  The disk side stripped the marker anyway, so `- endpoint:` on disk never matched the `- endpoint:` the log held, and 8 pages of a real workspace were told they carried 49 lines of unlogged content they did not carry.

  That is not an advisory verdict: it withholds `last_synced_hash`, refuses the page's re-projection, and reconciles it on every boot forever — the exact failure mode [RFC 0210](docs/rfcs/0210-md-content-outside-op-log.md) names as the worst one available.
  Each disk line is now tried in both shapes, stripped and verbatim, which widens *how* a known line may match and never *which* lines are known — a line the log genuinely lacks still fails both lookups.
  Residue on the reporting workspace: **0 pages, 0 lines**.

- **`crates/outl-md/tests/corpus_gate.rs` — the throwaway probe became a test.**
  Every defect this issue produced, the original and all four regressions introduced while fixing it, was found by running code over 2,827 real `.md` files; none by the unit suite, which was green at every one of those moments.
  Three properties now run in CI over `tests/corpus/`, a set of files reduced from the real shapes: no line is lost, `render → parse` is a fixpoint, and the unlogged-content check does not cry wolf.
  The maintenance rule is one line — when a `.md` bug is found in the wild, its shape becomes a file in that directory.

- **`CURRENT_PIPELINE_VERSION` 3 → 4, so the parser fix actually reaches existing workspaces.**
  A page that the old parser truncated is hash-faithful, and `reconcile_md`'s short-circuit consults only the hash — so a fixed parser never looks at it again and the content stays outside the log forever.
  The bump makes every sidecar stale by pipeline and turns the first boot into a one-shot migration, which is the same mechanism version 3 used for the same reason.

  It was missed in the first version of this change and caught in review.
  Worth naming because of *where* the failure is invisible: on the author's machine the recovery commands get run by hand, so nothing looks wrong, while every other user keeps content outside the log with no symptom at all.

- **`reconcile_md` claimed the op log held content it had never emitted an op for.**
  Writing `last_synced_hash` is the claim "the log holds what is in this file", and every consumer downstream believes it.
  `outl-md`'s invariant 8 already said the hash may only advance over content the same call emitted ops for — as prose, with no code checking it and no test pinning it.
  It is now enforced: the hash is withheld when anything is unaccounted for, leaving the page dirty so the next pass looks at it again, and `ReconcileReport.unlogged_lines` carries the count.
  A page that reconciles twice is a nuisance; a page that lies about its own state is a data-loss bug.

  With the parser fixed this fires on nothing, which makes a **false positive** its real failure mode — a permanently dirty page, across 2,560 of them.
  So the test that matters asserts the hash still advances for 17 shapes taken from the pages that actually held unlogged content: Roam's tab-indented ordinals and `*` bullets, a `U+2029` pasted from a PDF, non-breaking spaces, fences, properties followed by prose.

- **Matching level 3 deleted one block and five thousand the same way.**
  A `.md` that arrived truncated — an undownloaded iCloud placeholder, a half-flushed write — emptied a page as quietly as deleting a bullet.
  `reconcile_md` now goes through `match_blocks_guarded`, which refuses past 500 orphans or 75% of a page (once it has at least 20 blocks).
  The refusal covers the whole pass rather than shortening the orphan list, and is safe by construction because `match_blocks` is pure — refusing after it ran is refusing before anything exists to apply.

  `outl reconcile --allow-bulk-delete` is the way to say the deletion was meant.
  It needs its own mode rather than riding on `--ahead-of-log`, and the reason is that the two select **opposite** sets: `--ahead-of-log` visits pages whose `.md` holds *more* than the log, while a page the guard refused holds *less*, so `content_lines_missing_from` is zero and it never appeared in that list.
  Wiring the flag only there — which is how it first shipped — left it unreachable for every page it existed to unblock, the "guard with no escape hatch is a wall" failure invariant 9 names, arrived at through the door marked "fixed".
  `the_escape_hatch_applies_a_deletion_the_guard_refused` fabricates the refusal and resolves it, instead of asserting that a flag parses.

- **`apply_page_md_with_sidecar_if_stale` read the sidecar twice with no lock between the reads**, so the hash that authorised the write and the blocks that validated it could describe different revisions.
  This is the same defect `reconcile_md` had already fixed and documented.
  It also now declines to write when the sidecar *cannot* answer the question at all (every one written before 0.11 carries `text: ""`): an empty verdict from a reference that could not check is not permission to overwrite, and reading it as one is how the page gets emptied.
  `sidecar_can_answer` names that condition next to the stand-down it mirrors, so the two cannot drift.

- **`desync::recover_desynced_projection` re-projected over text the op log had never seen.**
  "Strictly additive" covered structure, not text: a block whose id the tree already knows kept the tree's text, so recovery overwrote what the `.md` said for it — and the ordinary offline session produces exactly that pair.
  It keeps the recovered ops and no longer rewrites the file.

- **The re-projection guard refused to update any page a peer had edited or deleted from, and the recovery command reverted the peer.**
  Found in code review of the commit that introduced it, by an executable probe, after 1,687 tests and a green `/check` had not.
  `content_lines_missing_from` compared the `.md` against a fresh render of the tree, which answers _"do disk and tree disagree"_ — and every remote edit answers yes to that, since the pre-edit line is on disk and absent from the render.
  So did every remote delete and every reorder.
  The page then froze showing pre-edit text with nothing surfaced to the user, which is [issue #166](https://github.com/avelino/outl/issues/166) reintroduced for the most ordinary sync case there is.
  Worse, `outl reconcile --ahead-of-log` (which the error message and `doctor` both recommended) wrote the pre-edit text back as ops on such a page, reverting the peer permanently, since the log is append-only.

  The reference is now the **sidecar's blocks**, which are what the log held at the last agreement, so the question asked is the one intended: _does the op log know this line_.
  The bullet marker, indent and trailing whitespace are normalised away — they are the renderer's layout, not content, so a pure indent no longer reads as new text — and `key:: value` lines are skipped, since a property is never part of a block's `text`.

  One consequence worth stating: a sidecar whose blocks carry `text: ""` cannot answer the question at all.
  That is **every sidecar written before 0.11**, when the field was added — measured on a real workspace, 7,400 blocks with not one populated.
  Answering anyway flagged 615 pages holding 35,261 lines against the 233 / 1,426 that are genuinely unlogged, which would have frozen most of the graph instead of guarding it, so such a sidecar does not get to veto the write.
  The `CURRENT_PIPELINE_VERSION` bump rewrites them on first boot and the guard arms itself from there.
  On the measured workspace the post-migration count is **29 pages / 261 lines**, down from the 79 / 909 the render-based comparison reported.

## [0.12.0]

### Changed

- **BREAKING — `ActionError` gains a third variant, `PageMarkdownAheadOfLog { path, lines, sample }`.**
  The enum is `pub` and not `#[non_exhaustive]`, and `outl-actions` ships to crates.io, so a downstream `match` that enumerates every variant needs updating.
  Same shape as 0.11.0's two-variant addition.

- **BREAKING (behaviour) — `apply_page_md_with_sidecar_if_stale` now returns `Err` on a page it used to rewrite successfully.**
  Louder than the type change and easier to miss: the signature is unchanged, so an embedder treating `Ok(None)` as "nothing to do" and `Err` as "something is broken" will start seeing errors on pages that previously re-projected silently.
  The error is the correct outcome — see the fix below — but code that logs and continues will now log where it used to be quiet, and code that aborts on any `Err` will stop where it used to proceed.

- **BREAKING (behaviour) — `DeviceStore::machine_id()` no longer overwrites a machine id minted by a racing process.**
  It mints through a compare-and-swap and adopts the winner on `AlreadyExists`.
  An embedder that relied on the last writer winning gets the first writer instead.
  The deliberate remint-a-clone path still overwrites, because that is a replacement, not a race.

### Fixed

- **`outl init` racing `outl serve` on a fresh machine could permanently break that workspace's actor claim.**
  `DeviceStore::machine_id()` minted with a plain write while `bind()` used a compare-and-swap.
  Two processes reaching a fresh store both minted an id and the last writer won; `init` then stamped `actor_claimed_by = <machine>` into `config.toml`, the next `open` read a machine id someone else had reminted, the claim read as foreign, and the workspace forked an actor.
  Reproducible 6 times out of 6 from a cold store, not intermittent — which is why it read as three flaky `doctor` tests for as long as it did.

  Two further defects surfaced underneath it, and the second was the dominant cause:
  1. **The compare-and-swap failed open.**
     `create_new_record` used a bare `O_EXCL` open, which creates an **empty** file and fills it a moment later.
     Every reader maps a blank record to `None` — "absent" — which is precisely the answer that licenses overwriting.
     It now composes the content in a temp file and publishes with `link(2)`, so the record appears complete or not at all, falling back to the old open where the filesystem cannot link rather than breaking on FAT media.
  2. **Concurrent writers shared one temp file.**
     `scratch_path` keyed the scratch name on pid alone, and test threads share a pid, so two writers used the same path: one `fs::write` truncated into the other and the publish step shipped a record neither writer composed.
     The name is now unique per call.

  Pinned by `concurrent_first_opens_converge_on_one_machine_id`, verified non-vacuous against both old code paths (16 distinct ids with the old mint, 2 with the old scratch path), plus `outl-ws`'s `device_isolation` test as its complement rather than its copy.

- **Opening a page could delete content that had reached the `.md` but never the op log.**
  Re-projection was gated on one question — does the sidecar's `last_synced_hash` match the bytes on disk? — and a `.md` that answers yes was treated as a merely _stale_ projection, safe to overwrite with a fresh render of the tree.
  But that hash proves the sidecar agrees with those bytes; it never proves the bytes came from the log.
  A `reconcile_md` that rewrites the sidecar without emitting ops for everything it read leaves a page in exactly that state, and re-rendering over it destroys the difference _and_ rebuilds the sidecar from the same render — so no later scan, and no `doctor` check, could tell the page had ever held more.
  Silent by construction, which is the shape this project treats as the worst kind of bug.
  Measured on a real 2,560-page workspace: **233 pages holding 1,426 lines** in that state.
  `outl doctor --repair` deleted all of it and reported `708 fixed`.
  Worse, `--repair` was not the only path — `apply_page_md_with_sidecar_if_stale` runs on **every** GUI open (`open_page_by_slug`, `open_journal_for`, `open_today_journal`, `open_ref`), so a plain page open in the desktop or mobile app was enough.
  `outl_actions::content_lines_missing_from` is now the single owner of the verdict, and `apply_page_md_with_sidecar_if_stale` returns the new `ActionError::PageMarkdownAheadOfLog { path, lines, sample }` instead of writing.
  It compares a **multiset** of trimmed non-blank lines rather than running a diff, because a line the renderer merely _moved_ is not at risk — on that same workspace an LCS diff flags 616 pages where only 233 genuinely hold unlogged content.
  Whitespace-only drift is ignored on purpose: the renderer's trailing-newline behaviour changed across releases, and treating that as content would strand every genuine re-projection behind noise.
  `outl doctor` calls the same function in its read-only listing, so it can no longer offer a repair the `--repair` pass then refuses (the "announced before they run" invariant), and names the count plus one of the lines at risk instead of only counting them.
  Content in this state also never reached your other devices — peers exchange ops, not files — so the report says that too.
  `outl reconcile` owns the `.md → tree` direction and remains what brings it into the log.

- **`docs/query.md` rendered 46 lines as one code block, and four headings produced no anchors** (issue #214).
  A three-backtick outer fence wrapping an inner ` ```query ` example closed at the _indented_ inner closer instead of its own, so everything from line 115 on was inside a code block.
  `## Relationship to {{query: ...}}`, `## Extensibility`, `## Architecture` and `## Plugin SDK API` were swallowed, which broke four links — two of them pre-existing in `docs/plugin-api.md` and `docs/plugins.md`, dead long enough that nobody had noticed.
  The file already used a four-backtick fence for the same pattern 90 lines earlier.

### Added

- **`outl reconcile --ahead-of-log` — the `.md → tree` direction on pages the ordinary reconcile cannot see.**
  A page holding content the op log never saw is _hash-faithful_: its sidecar agrees with the bytes on disk, so `needs_reconcile` reads it as in-sync and skips it.
  Fixing the parser that produced that state does not recover anything by itself, which took measuring to notice — `serve --once` applied **0 ops** to all 233 such pages on the measured workspace.
  The flag clears the recorded hash on exactly the pages `doctor` names and reconciles them, which emits ops for that content.
  Opt-in on purpose, and it prints the page count and line count before it writes: this is a deliberate write, not a repair.
  Detection is `outl_actions::content_lines_missing_from` against the sidecar's blocks, the same owner `doctor` and the write-side guard use, so the three cannot disagree about which pages qualify.
  Run it only on a build whose parser preserves the content — reconciling with one that still drops prose after a block property writes the truncated text into the log, which is the one place the loss currently is not.

- **`CURRENT_PIPELINE_VERSION` goes 2 → 3, which recovers the existing content without a command.**
  The constant exists to force a re-reconcile when the pipeline could produce a different op log for the same `.md`, and a parser that now reads prose it used to discard is exactly that.
  Every existing sidecar becomes stale by pipeline, so the first boot per device is a one-shot migration: **323 ops applied, 233 pages down to 79** on the measured workspace, and 29 after the guard was corrected to compare against the sidecar.
  One slower boot per device, additive (the change only ever captures _more_ text), and crash-safe, since the stamp is per page so partial progress is kept.

- **RFCs: `docs/rfcs/`, 16 documents, and a process that ties reasoning to an enforceable rule.**
  outl already recorded evolution in four places (a lone RFC, the decision table in the root `CLAUDE.md`, `CHANGELOG.md`, `docs/design/`), so a fifth format would have broken the one-owner-per-fact rule that keeps the shortcut and CLI tables from diverging.
  ADRs were considered and rejected for a sharper reason: **an ADR would not have prevented the bug that prompted this.**
  Issue #166 was documented — issue, changelog entry, code comment explaining the gate.
  What was missing was not a record of the decision taken; it was the question nobody asked, _and in the opposite direction?_

  So the template makes that a **required, non-deletable section**, and the process ties every RFC to an invariant and a named test:

  | Layer     | Where                         | Role                                                     |
  | --------- | ----------------------------- | -------------------------------------------------------- |
  | Reasoning | `docs/rfcs/NNNN-*.md`         | Why the rule exists, what was rejected, what got worse   |
  | Rule      | root or per-crate `CLAUDE.md` | Read on every edit to that crate — the enforcing surface |
  | Proof     | a named test                  | Fails mechanically when someone reverts the behaviour    |

  A rule with no RFC has no rationale and gets argued away in review; an RFC with no `CLAUDE.md` entry is never read at the moment it matters; either one without a test is a comment.
  **Changing behaviour an RFC pinned means updating that RFC in the same PR** — amend, or supersede and mark the old one.
  The RFC number **is** the issue number, so there is no second sequence to keep in sync.
  Flow, "do I need an RFC?", and the retroactive triage live in [`docs/rfcs/README.md`](docs/rfcs/README.md); the four-step issue → discussion → PR-with-RFC → review path is in [`docs/contributing.md`](docs/contributing.md).

  Of 78 closed issues, **27 carried decision content and collapsed into 14 retroactive RFCs**; the other 51 are changelog-only.
  The collapsing is the point — eight keybinding issues tell one story about `outl-shortcuts` being the single catalog, so they are one document, not eight.

  Writing them also corrected the record twice: **#179's Front B never shipped** (`actor_census` still does a full `all_ops()`, so the ~35 s delta was only half improved), and **#25 shipped inverted** (Boa primary, not the QuickJS-primary its own proposal described).
  Five behaviours nothing pins are recorded as `none found — gap` rather than papered over with invented test names (issue #213).

- **Invariants 8 and 9 in the root `CLAUDE.md`, each with the incident that produced it.**
  Invariant 8: a sidecar hash match proves outl wrote those bytes last, never that the op log holds them.
  Invariant 9: when state crosses a boundary, the RFC that moves it must say what the new home requires — **who writes it, who reads it, how a test gets its own copy, and what cleans it up.**
  0.11.0 moved the write actor out of the workspace correctly and left question three unanswered; question four is still open (nothing prunes a workspace the user deleted).

  Both generalize to one rule, stated in the root file: **a fix relocates a problem far more often than it removes one.**
  After "did I fix it?" comes "where does the problem live now, and what does that place require that the old one did not?"
  Three separate defects in this codebase came from skipping it.

### Changed (tooling and docs)

- **`.github/copilot-instructions.md`: 39,980 → 10,548 chars, split into path-scoped instruction files** (issue #215).
  It sat ~20 chars under the size ceiling and roughly six times over GitHub's own "no longer than 2 pages" guidance, which made the two hooks contradict each other: adding a required catalog mirror row overflowed `markdown-size-guard`, while `catalog-sync-guard` required that row to exist.
  Now four `.github/instructions/*.instructions.md` files carry `applyTo` globs and load **in addition** to the repo-wide file, so a Solid PR no longer pays for the Rust bar plus 17k of Rust primitives catalog.
  `markdown-size-guard` already covers the new files: `*` matches `/` in a bash `case`, so `.github/*.md` reaches into `.github/instructions/` on its own.
  Note for anyone adding a rule: **there is no include mechanism** — each file loads whole or not at all, so anything that must always apply belongs in the repo-wide file.

- **Three per-crate `CLAUDE.md` files were at or over the 40k ceiling; extracted to reference docs** (issue #216).
  `outl-mobile` 42,345 → 32,265, `outl-desktop` 41,435 → 34,560, `outl-sync-iroh` 39,998 → 32,955.
  New docs with real owners: `docs/iroh-internals.md`, `docs/ios-platform.md`, `docs/deep-links.md`; the rest went into `development.md`, `reminders.md`, `plugin-architecture.md`, `theming.md`.
  Movement, not rewriting — of 350 lines removed, 338 exist verbatim in the destination, and each stub keeps whatever in it was genuinely an invariant.
  The extraction surfaced seven documentation contradictions, filed as issue #212, including desktop settings documented at a path the code no longer reads.

- **43 links now connect RFCs and docs in both directions**, including a `Reference doc` row in every RFC header (and in the template, so new RFCs inherit it).

- **Qodo's code review is now configured by `.pr_agent.toml` in the repo, not in its web portal.**
  A repo-local `.pr_agent.toml` outranks portal settings, so a knob changed in the UI would have silently disagreed with what is committed — and nobody reading a PR could tell which one won.
  Five keys, no copied defaults: `repo_context_files = ["CLAUDE.md"]` (the default `AGENTS.md` does not exist here), `ignore_pr_title` extended with WIP/DRAFT while preserving the upstream `^Auto` entries the key would otherwise replace, `expand_evidence` so the file:line citations open by default, and two guideline blocks that name the failures this repo has actually shipped — the mirrored-divergence rule from RFC 0210, the hash-is-not-membership rule, guards a sentinel disarms, convergent state outside an `Op` from RFC 0211, and the four questions invariant 9 asks of state that moves.
  Automation (`pr_commands`, `push_commands`) is deliberately absent: that key replaces the default list rather than extending it, so writing it out would quietly drop whatever the platform adds later.

  No `best_practices.md` was added, though Qodo imports one if present.
  Its rule import already reads root and per-crate `CLAUDE.md` and scopes each file's rules to the directory holding it at any depth, so `crates/outl-md/CLAUDE.md` is stricter inside that crate and silent elsewhere for free.
  A `best_practices.md` would be a second copy of rules `CLAUDE.md` owns, and the copy is the one that goes stale.
  One gap worth knowing: Qodo does **not** read `.github/instructions/*.instructions.md`, so anything that must reach both it and Copilot belongs in a `CLAUDE.md`.
  Documented in `docs/contributing.md` → "The automated reviewers", which now maps each of the three review bots to the file that configures it.

## [0.11.0]

### Changed

- **BREAKING — `DeviceStore::actor_for_workspace` / `set_actor_for_workspace` become `actor_for_instance`.**
  The write actor is now keyed by `WorkspaceId` **plus the canonical root path**, so a copied workspace directory forks its own actor instead of sharing an op log, while a moved or renamed one keeps the actor it had.

- **BREAKING — a workspace created before the claim marker forks a new actor on every device, once.**
  One extra `ops-<actor>.jsonl` per device; the legacy log stays on disk and is still merged on read, so nothing is lost.
  This is the cost of closing the shared-actor hole below, and it is paid once.

- **BREAKING — reads that used to return a short or empty result now return an error.**
  `read_page_outline`, `Storage::ops_since`, `ops_for_actor` and `ops_since_per_actor`.
  A short read is indistinguishable from a healthy one, which is exactly how ops went missing without anything surfacing.

- **BREAKING — `SidecarBlock` gains a `text` field and `ActionError` gains two variants.**
  Both are `pub` and neither type is `#[non_exhaustive]`, so a literal `SidecarBlock { .. }` or an exhaustive `match` on `ActionError` downstream needs updating.
  Build sidecar entries through `SidecarBlock::from_text`, which keeps hash, handle and stored text derived from one revision.

### Fixed

- **Four paths that could lose your content without saying so.**
  All four were silent by construction: nothing errored, nothing appeared in a log, and the state left on disk looked healthy to every consistency check outl had.
  That is the specific thing that makes a notes app untrustworthy — you find out weeks later, from a page that is now blank.
  1. **A failed read could overwrite a page with nothing.**
     Every read-parse-mutate-write path opened the `.md` with `read_to_string(..).unwrap_or_default()`.
     A read that failed for any reason other than "the file isn't there" — a permissions change, a raw `EIO`, invalid UTF-8, or an **iCloud placeholder whose bytes hadn't downloaded to this device yet** — parsed as an empty document, rendered, and atomically replaced the page with nothing.
     The sidecar was then rebuilt from that same empty parse, so its hashes agreed with the file and no later scan could tell the page had ever had content.
     Reads on a rewrite path now go through `outl_md::read_for_rewrite`, where a missing file is empty and **every other I/O error propagates**.
     The TUI additionally refuses to save a page it could not read, and says so.
  2. **The desktop, mobile and undo paths reconciled with the orphan log switched off.**
     `outl-md`'s hard rule is that a block dropping to matching level 3 is recorded in `orphans.log` _before_ it is moved to the trash.
     Four call sites passed `None` for that log, so on those clients the deletion happened with no record anywhere — the exact case a half-synced `.md` from iCloud produces at boot.
     `outl_actions::sync::orphans_log_path` is now the one owner of that path — every caller passes it, and `outl_ws`'s `Paths::at` derives its `orphans` field from it instead of re-joining `.outl/orphans.log` itself.
  3. **One unreadable line truncated the whole op log.**
     The sequential replay hit an I/O error and `break`, discarding every op _after_ it.
     A transient failure on line 5,000 of 200,000 booted a workspace containing the first 5,000 ops and carried on as if that were everything.
     It now skips the damaged line and continues (with a cap on consecutive failures), matching how a corrupt or non-UTF8 line was already handled two lines below.
  4. **A snapshot could record an op it never applied.**
     The index-driven reads dropped an op the offset index listed but disk wouldn't return.
     Because the next snapshot's cutoff is derived from that same index, the omission was written down as "already folded in" — and no later boot would ever replay it again.
     Permanent loss, with the bytes still on disk.
     Those reads now fail with `MissingOp` and snapshot boot degrades to a full replay, which re-reads the file and recovers everything around the damage.

  Pinned by regression tests, including fault injection — there were previously **no** tests that forced an I/O read to fail, so all four passed a green `/check`.

- **`.md` and sidecar writes now fsync the parent directory after the rename.**
  `rename` is atomic for readers, but the directory entry it creates is only durable once the directory itself is synced.
  APFS and ext4's default `data=ordered` usually paper over this; other filesystems don't.

- **External edits no longer break block references in the common case.**
  Editing a `.md` outside outl in a way that both **rewords one block and adds or removes another** made the block counts disagree, which disabled the positional fallback and sent every reworded block to matching level 3: fresh ULID, old id trashed, and every `((blk-…))` pointing at it dangling.
  That is the _ordinary_ external edit, not an exotic one.
  Level 2 of the documented matching algorithm is now implemented (normalized Levenshtein > 0.8 against the previous text, which the sidecar keeps in the `text` field — additive, still version 2), so the id **and** its ref handle survive.
  The positional fallback also compares real parents instead of indent depth — same depth in a different subtree used to hand one subtree's id to another block.

- **The actor id no longer lives inside the workspace.**
  "One `ops-<actor>.jsonl` per device, never shared" is what makes last-write-wins-per-file harmless, and it was only true by accident: the id sat in `<root>/.outl/config.toml`, and the only transport that didn't replicate it is iCloud (which drops dot-prefixed paths).
  On Syncthing, Dropbox, NFS, a shared volume or a `git clone`, both devices read the same actor and appended to one file — `flock(2)` is advisory and machine-local, so each acquired its lock successfully and ops vanished with nothing raised.
  The write actor is now resolved from a device-local store outside the workspace, keyed by `WorkspaceId`, with a migration that keeps an existing device on its existing op file.

### Added

- **Local backups (`outl backup init` / `now` / `list` / `restore` / `status`).**
  The op log is append-only and every write is atomic, but that only covers the failures outl was _designed_ for.
  It had no answer for a bug in a projection path, an `outl import` aimed at a workspace that already had pages, a sync tool resolving a conflict the wrong way, or a page deleted with the app then closed (undo is in-memory and dies with the process).
  For all of those, the recovery story was "reconstruct it by hand from `ops-*.jsonl`".
  Git-backed — the `git` binary, not `libgit2`, so nothing new reaches a dependent's `cargo deny`.
  Captures `ops/`, `pages/`, `journals/`, `templates/`, `assets/` and `.outl/config.toml`; excludes the caches the next boot rebuilds.
  **`restore` never writes in place.**
  It extracts to a directory you name — and refuses one inside the workspace — so you diff and choose what to bring back; a recovery tool that overwrites the live op log is the last thing you want when something has already gone wrong.
  `[backup] enabled` defaults **on**, for the same reason reminders do: the failures it catches are ones you discover _after_ the moment you could have turned it on.

  **The repository is device-local and lives outside the workspace**, at `<device-dir>/backups/<slug>-<hash>.git` with the workspace passed as `--work-tree`, so **nothing is written inside the workspace** — no `.git/`, no pointer file.
  That single decision closes two problems at once: a `.git` inside the workspace would ride Syncthing / Dropbox / NFS (object store, `index`, `HEAD`, `index.lock` over eventual sync — the same class of bug the actor-id move exists to fix), and a workspace you already keep in _your own_ git repo would have had outl staging over your index, committing to your branch, running your hooks, and tripping your `commit.gpgsign` into a Touch ID prompt per snapshot.
  Now outl's snapshots use their own git dir, own branch, own identity (`outl backup <backup@outl.app>`), hooks disabled, signing off.
  Your repo is never touched, and backups keep working for the people who version their notes — the ones with the most to lose.
  **A `.gitignore` cannot silently drop your data**: `ops/`, `pages/`, `journals/`, `templates/`, `assets/` and `.outl/config.toml` are force-staged, and every snapshot is **verified** afterwards — an op log missing from the commit is an error, not a green checkmark.
  **The automatic pass is real**, not just a config key: a background thread snapshots on the `[backup] interval_minutes` floor (derived from git itself, no state file), wired into the TUI today.
  `docs/config.md` names exactly which clients run it, rather than implying all of them do.

- **`outl doctor` now checks what it couldn't, and `--repair` fixes what is safe to fix.**
  New checks: corrupt `.jsonl` lines (named by line number, byte offset and reason — previously the one gap the code itself admitted to), snapshot integrity, offset-index coherence, blocks sitting in the trash (previously invisible to the user entirely), sync-conflict copies from iCloud / Syncthing / Dropbox, and ops the materialized tree never applied.
  `--repair` re-projects a stale `.md` from the op log, rebuilds a missing sidecar, drops a corrupt snapshot, and prunes stale `.outl/repair-backup/` generations — nothing else.
  It never deletes a `.md`, never writes into `ops/`, never trashes a block, and copies every file it touches to `.outl/repair-backup/<timestamp>/` first.
  All four are announced before they run, so the read-only listing and the repair pass can't disagree.
  A run whose only work is a prune still happens — a workspace with nothing wrong with it is the one that would otherwise hoard backups forever.

- **The Roam importer now tells you what it didn't bring over.**
  It reports `pages: N/M` and `blocks: N/M` against counts taken from the source JSON, subtracting only the reductions it can name (blocks lifted into page properties, journals merged, pages skipped) and shouting when the books don't balance.
  Four silent losses are now counted or refused: a `{{[[TODO]]}}` in the middle of a block (which becomes literal text and loses its task state — it is _counted_, aggregated into one warning rather than thousands), a page with an empty title (which used to be dropped along with its entire subtree, with no record), and re-importing into a populated workspace, which **overwrote the `.md` files and reconciled the result — destroying anything written in outl since the last import**.
  That now aborts and asks for `--force`.

- **`remind::` — a block-level reminder rule that turns a TODO into an OS notification, on desktop and mobile (issue #63).**
  A `[[2026-12-12]]` was only ever good for **recall**: it put a backlink on that day's journal and then waited for you to open the app.
  If you didn't, the reminder was silent.
  Now a block can carry a rule that reads as English — `remind:: 3pm every 1h until DONE` — and the OS tells you.
  **Opt-in on purpose:** a `[[date]]` alone still schedules nothing, because plenty of people use dates purely for backlinking, and the moment a link becomes a buzz the linking stops.
  The grammar is `TIME ("every" INTERVAL)? ("until" STOP)? ("max" N)?` — `10am`, `15:00`, `now`, `every 30min`/`1h`/`2d`, `until DONE`/`until 6pm`/`until 2026-12-20`, `max 5`.
  Caps are a 1-minute interval floor (a sub-minute nag is never what you meant, so it's rejected rather than silently rewritten) and a 10-fire ceiling (clamped, with a warning).
  A rule the parser can't read **never costs you the property or the block** — it stays on disk verbatim and just doesn't schedule, surfacing in the parse banner and `outl doctor` like every other dialect recovery.
  **The schedule math has exactly one owner**, `outl_actions::reminders::next_fire_at` — pure, clock-free, `now` passed in.
  The TUI overlay, the desktop panel, the mobile sheet and every OS bridge call it; a second opinion in TS or Swift about when a reminder fires is drift that reaches the user at 3am on one device before it reaches a test.
  Two behaviours worth knowing: a device that was asleep owes you **one** banner, not a backlog (close the laptop at 10:00 on an hourly rule, open it at 18:00, get one reminder), and a block with two `[[date]]`s schedules on **both**, because you wrote both on purpose.
  **Snooze converges** via the new `Op::SnoozeRemind` — silencing a nag on the phone silences the same block on the laptop.
  The device-local half ("this device already buzzed you") deliberately does _not_: it lives in `<root>/.outl/reminders-fired.json`, a dotfile iCloud drops and iroh never ships, pruned at 7 days.
  **Quiet hours** (`[reminders] quiet_hours = "22:00-07:00"`, device-local, unset by default) push a fire to the window's end rather than dropping it — you asked for it, you get it, just not at 3am.
  Delivery itself (`[reminders] enabled`) is **on** by default: writing `remind::` is already the opt-in, and a device that never gets a rule never fires, so defaulting off only bought you a rule that silently did nothing.
  A fire pushed past its own `until` is genuinely over.
  Surfaces: **TUI** `g r` / `g R` to author and `g n` for the overlay (`Ctrl+R` was the obvious chord and is already Redo — a terminal can't tell it from `Ctrl+Shift+R`); **desktop** `Cmd+R` to author, `Cmd/Ctrl+Shift+R` for the panel; **mobile** long-press → _Remind me…_, bell icon for the list.
  **All three deliver**, including the TUI, which fires an OSC 9 desktop notification plus a toast on its event-loop tick (OSC 9 is the sibling of the OSC 52 the yank path already uses, honoured by iTerm2 / kitty / WezTerm / ghostty, and the toast covers the emulators that ignore it).
  That is why `take_due` and the fired log sit in `outl-actions` rather than behind the Tauri layer, which the TUI can't reach.
  `g s` snoozes the block under the cursor an hour without opening the list, on both keyboard clients.
  On **mobile** the Reminders sheet carries the two device-local settings itself (a delivery switch and quiet hours as native time pickers), because the app has no settings screen and `config.toml` lives inside the iOS sandbox — without them the sheet could say "notifications are off" and offer nothing to do about it.
  Its rows also mark a task DONE, matching the desktop panel.
  **Scope, stated plainly:** notifications fire whenever the app is running, foreground or backgrounded, on macOS / Linux / Windows / iOS, and while the TUI is open.
  **Delivery with the app fully closed does not ship in this change** — the iOS `UNCalendarNotificationTrigger` pre-registration (64-request cap, `BGAppRefreshTask` refill), the macOS launch agent, the Windows scheduled toast and the systemd user timer are tracked as follow-ups on issue #63.
  A reminder for a day you never open outl will not reach you yet; worth knowing before relying on it for something that matters.
  Full spec: [`docs/reminders.md`](docs/reminders.md).

- **Embedded assets now render: `![alt](url)` shows an inline image on desktop/mobile and a placeholder in the TUI, and imported images stop being dead links (issue #203).**
  Uploading or importing a file already copied it into `assets/<hash>.<ext>`, but nothing rendered it — every asset, images included, landed as a plain `[name](assets/…)` link, so an imported graph showed clickable text where you expected to _see_ the picture.
  `![alt](url)` is now a first-class inline token (`InlineTok::Image` / owned `InlineToken::Image { alt, href }`), parsed by `try_image` right after the embed matcher so the leading `!` is never stranded before the bare-`[` link — one parser in `outl-md`, consumed by every client (no parallel TS/Swift tokenizer).
  **Desktop and mobile** render an image inline through the shared `<MarkdownInline />`: a local `assets/…` asset loads its bytes through a new `read_asset_data_url` backend command (resolved with the existing traversal-safe `resolve_asset_path`, capped at 25 MB, returned as a `data:` URL — no Tauri asset-protocol config, identical on both clients), and a remote `http(s)` image loads directly.
  A non-image `![…]` (e.g. `![notes](assets/x.pdf)`) degrades to a clickable file chip (`📄 name`) that opens in the OS app, so nothing is ever left unrendered.
  **The TUI** paints a `🖼 alt` / `📄 name` placeholder (a terminal can't show pixels), keeping the raw `![alt](url)` verbatim in edit mode so cursor alignment stays exact.
  **The importers** (`roam` / `logseq` / `obsidian`) now emit the embed form `![…]` for image assets and keep the plain link for everything else, so a migrated graph's images render on first open.
  Client-side image-vs-file classification reuses `wikilink::is_image_target` / the mirrored `assetKind` helper — no second extension list.

- **`==highlight==` is now a native inline token, so Roam's `^^highlight^^` renders as a highlight everywhere instead of vanishing.**
  The importer already rewrote `^^…^^` to `==…==` on disk, but nothing rendered `==…==` — the marker just sat there as literal text.
  `outl_md::tokenize` now emits an `InlineTok::Highlight`, the shared `<MarkdownInline />` renderer wraps it in `<mark>`, and the TUI paints it with a yellow background.
  Pasting Roam content converts `^^…^^` to `==…==` too (previously it was stripped), trimming any space next to the markers so the result renders.
  The matcher rejects a space next to either marker, so a spaced comparison (`a == b`) stays plain text — unlike `~~strike~~`.

### Changed

- **The snapshot boot cache is encoded with `postcard` instead of `bincode`, so `outl-core` can be embedded in a project with a dependency-policy gate (issue #207).**
  `outl-core` is on crates.io specifically so other projects can [embed outl](docs/embedding.md) as a storage layer.
  That story was blocked in practice: a maintainer auditing a PR that embedded outl rejected it because the graph failed their policy gate.
  The cause is [RUSTSEC-2025-0141](https://rustsec.org/advisories/RUSTSEC-2025-0141) — the bincode team ceased development permanently, and the advisory carries `patched = []`, meaning **every** version is flagged, 1.x and 2.x alike (the 3.0.0 "release" is a tombstone whose entire `lib.rs` is a `compile_error!`).
  So moving from bincode 1 to bincode 2 would have changed the version number and left every downstream `cargo deny` / `cargo audit` failing exactly as before.
  postcard is serde-native, actively maintained, `MIT OR Apache-2.0`, and its varint encoding is smaller on the wire — which also helps the peer snapshot transfer over iroh.
  **Nothing about this can lose data.**
  A snapshot has never been source of truth; the op log is.
  `SCHEMA_VERSION` goes `3` → `4`, an old snapshot fails `decode`, and that lands on the same path a corrupt snapshot always took: full op-log replay, nothing surfaced to the user.
  The cost is one slower boot per device, once.
  Cross-version pairing degrades the same way — a peer still on the old build ships a snapshot this build skips while it keeps scanning for a readable one, so it replays instead of erroring.
  The regression tests decode a **real** schema-3 snapshot captured from the old encoder rather than a synthetic corruption, on both the local-boot and the peer-adoption path.
  `bincode` is gone from `outl-core`'s dependency graph, direct and transitive. One flagged crate remains across the whole embedding contract, `smallstr` (see the entry below), so a gate that fails on `unmaintained` is not clean yet.
  Note the workspace `Cargo.lock` still resolves bincode 1.3.3 through `steel-core`, the Lisp runtime behind `outl-exec`'s default features — that's a separate graph an embedder only opts into by taking `outl-exec`.

- **Swept the rest of the published crates for the same class of problem, and cleared every advisory that a version bump could clear.**
  #207 was one symptom; the audit was the point.
  `wasmtime` 46.0.1 → **47.0.3** closes two real vulnerabilities (RUSTSEC-2026-0222 / -0223 — cross-engine type-index confusion and VM-state corruption on preempted bulk operations). 46.0.2 was never published, so the fix required the major bump; it needed no code change.
  `crossbeam-epoch` 0.9.18 → 0.9.20 (RUSTSEC-2026-0204, invalid pointer dereference), `anyhow` 1.0.102 → 1.0.104 (RUSTSEC-2026-0190, unsoundness in `Error::downcast_mut`), `event-listener` 5.4.1 → 5.4.2 (RUSTSEC-2026-0221, `!Send` tags crossing threads).
  **The embedding contract — `outl-core`, `outl-md`, `outl-actions`, `outl-ws` — now carries exactly one flagged crate between all four**: `smallstr 0.3.1` (unmaintained, no patched release), transitive through `yrs`. `yrs` 0.27.3 still depends on it, so that one needs upstream and is tracked separately.
  Its licenses are fully permissive.
  The sweep also clarified something [Embedding](docs/embedding.md) had wrong: **`outl-exec` is not embedding surface.** It sits on crates.io only because `outl-actions` references it and cargo won't publish a crate whose dependencies aren't in the registry. Running code fences is an app concern, not a storage-layer one, which is why the workspace has always pinned it at `default-features = false` — and why the LGPL-3.0 / MPL-2.0 crates its language runtimes pull in never reach an embedder. The doc now says so instead of listing it as a fifth option.

### Fixed

- **`outl doctor` no longer claims every `.md` parses cleanly right after listing the ones that don't.**
  The parse-warning check ran once per directory and printed its all-clear from inside that loop, so a dirty `pages/` followed by a clean `journals/` produced "2 line(s) outside outl dialect" and "every `.md` parses cleanly in the outl dialect" three lines apart.
  The tally is now accumulated across every scanned directory and the verdict printed once — a workspace-wide claim needs a workspace-wide count.
  Pre-existing, but the new `remind::` validation makes it far easier to hit.

- **The Roam importer now carries `key:: value` attributes over as real properties instead of dropping them into bullet text.**
  A Roam page's attribute blocks (`icon::`, `page-type::`, `work::`, `related::`, `oura-date::`, and every `key:: value` a graph accumulates) used to import as plain text bullets — the adapter set `props: Vec::new()` unconditionally, so a contact page's `page-type:: contact` never reached outl's index and the sidebar icon, the `page-type` filter, and the `@` mention autocomplete all came up empty on imported graphs.
  Blog-post frontmatter (`url::`, `draft::`, `public::`, `status::`, `tags::`) landed as text too.
  The adapter now lifts attribute lines out of each `:block/string`: the leading run of pure-attribute blocks (only `key:: value` lines) at the head of a page is promoted to page properties in the `.md` header, while a pure-prop block that appears after real content stays in place so promotion never reorders the outline.
  Structural lines are normalized everywhere: `collapsed:: true` becomes the fold flag (`Op::SetCollapsed`) instead of a literal property, and `id::` (Logseq residue) is dropped and counted as an artifact.
  A block that still has prose keeps its `key:: value` lines in the text on purpose: outl's own parser lifts trailing continuation properties and resolves any `((uid))` in their values through the placeholder pass, so the block-ref-in-a-property-value shape (Omnivore's `note:: ((uid))`) still degrades correctly instead of being silently bypassed.
  `parse_prop_line` moved to the shared `adapters/scan.rs` so the Roam and Logseq adapters classify a property line identically.

- **Pairing a device from the CLI (`outl peer pair --ticket`) now joins the host's workspace instead of leaving the two devices unable to sync (issue #197).**
  A machine that paired via the CLI kept its own freshly-generated workspace identity, so every later sync was refused with `rejecting sync from peer on a different workspace` — the paired devices looked like two unrelated graphs and nothing ever converged.
  The CLI joiner now **adopts the host's `WorkspaceId`** during the handshake (persist-first: it writes the id to `<workspace>/.outl/workspace-id` before returning), exactly like the desktop/mobile GUI already did, and the host advertises its own id so a CLI host works too.
  `outl peer pair --ticket` now prints what happened — `Joined the host's workspace (…)`, `Already on the host's workspace`, or a warning if the host is on a build too old to advertise an id.
  The sync onboarding walkthrough in [`docs/sync.md`](docs/sync.md) was rewritten to hand-hold a new user step by step (which folder to pair from, that the joiner adopts the host's workspace, how to pull the notes) and gained a Troubleshooting section for the `workspace-mismatch` message.

- **The desktop app now resolves `((blk))` block refs and expands `!((blk))` embeds instead of rendering the raw handle (issue #147).**
  An inline `((blk-XXXXXX))` used to show its literal handle on the desktop; it now renders the source block's text (Roam-style), and a `!((blk-XXXXXX))` embed expands the source block as `↳ text` plus its full subtree — nested and read-only — matching what the TUI already did.
  The rendering lives in the shared frontend so mobile can adopt it without a parallel implementation: `<MarkdownInline />` resolves the `blockref` token against an `embeds` map (orphan handle = raw chip), and a new `<EmbeddedSubtree />` component in `@outl/shared` renders the embed's subtree depth-capped at 4, mirroring the TUI's `emit_embedded_children`.
  The desktop only wires the loop — `OutlineView` collects every ref + embed handle on the page with `collectBlockRefHandles` and resolves them in one round-trip through the `resolve_embeds` backend command, which was extended to carry each source block's subtree (`EmbedContent.children`, projected with tokens via the new `outl_actions::project_parsed_subtree`).
  The resolution logic itself is shared (`@outl/shared` + `outl-tauri-shared`), so the client holds no client-owned ref/embed rendering path.

- **TUI: opening a page from the quick switcher (`Ctrl+P`) no longer lands on an empty duplicate when the page's on-disk slug contains slug-unsafe characters (issue #195).**
  The switcher's preview resolved the page by its literal `pages/*.md` file stem, but Enter round-tripped that same stem through `slugify()` before opening — for a slug that isn't slugify-idempotent (`~`, `%`, uppercase; e.g. slugs written verbatim by the MCP's `page create`), the re-slugified path missed the real file and the "not found" branch silently created a fresh empty page (`title::` plus one empty bullet) next to the real one.
  Enter now opens the candidate's literal on-disk slug, the same identifier the preview already used; opening by a user-visible name (following a `[[ref]]`, `/open`) still slugifies as before.

- **Re-rendering a page from the op log no longer deletes its block-level `key:: value` property lines from the `.md`.**
  `reconcile` correctly turned block properties into `Op::SetProp` on the block node, but the reverse projection (`render_page_md` / `render_block_md` via `build_outline`) never wrote them back — so any mutation that re-rendered the page (a GUI edit, the importer's ref-resolution pass) silently dropped every `priority:: high`-style line from disk, and the next external-edit reconcile would emit property-removal ops: convergent data loss.
  Block properties now project back alpha-sorted, matching the page-property behavior, and the importer's end-to-end suite pins the round trip.

- **Pressing Enter in the middle of a block now splits it instead of leaving the text untouched and opening an empty block below (issue #184).**
  Every client — TUI, desktop, mobile — used to treat Enter as "commit this block, then create an empty sibling below and start typing there," no matter where the cursor sat, so splitting a sentence into two blocks meant a manual cut-and-paste.
  Enter now behaves the way every other outliner does: the text before the cursor stays in the current block, the text from the cursor onward moves into a brand-new sibling created right below, and the cursor lands at the start of that new block.
  The two edge cases keep their old, familiar shape — Enter at the end of the text still just opens an empty block below, and Enter at the very start pushes the whole line down and leaves an empty block above it.
  Children of the split block stay with the head, so indenting is never disturbed.
  The desktop and mobile apps share one backend operation for this (`outl_actions::split_block`, wired through a new `split_block` Tauri command); the TUI has its own equivalent that slices the in-flight edit buffer before it's ever written back to disk, since the TUI edits an AST that hasn't round-tripped through the workspace yet.

### Added

- **The embedder lib crates now publish to crates.io on every release.**
  `outl-core`, `outl-md`, `outl-exec`, `outl-actions`, and `outl-ws` — the closure an external tool needs to open and mutate an outl workspace through the op log — are published by CI at the same version the binaries report, beta and GA alike.
  Embedding outl no longer requires a git checkout side by side: a plain `outl-core = "0.8"` requirement resolves GA versions, and a `"0.8.0-beta"`-style requirement rides the betas cut from `main`.
  Every other crate in the workspace is explicitly `publish = false`.

- **`outl import roam` now preserves block refs, embeds, and folded state — powered by a new adapter-based import pipeline (`outl-import` crate).**
  The old importer flattened every `((uid))` block ref into a whole-page link and mangled `{{embed}}`s into leftover syntax; for a graph that leans on refs and embeds, that was silent data degradation at migration time.
  The new pipeline parses the source into a typed IR, writes markdown with inert placeholders, reconciles to mint sidecar handles, and then resolves every source UID into a real `((blk-XXXXXX))` reference / `!((blk-XXXXXX))` embed — through the op log, so block identities never shift under the rewrite.
  Roam dialect now translates properly on the way in: `__italic__` → `*italic*`, `^^highlight^^` → `==highlight==`, `#[[Multi Word]]` → `[[Multi Word]]`, `{{[[TODO]]}}`/`{{[[DONE]]}}` → outl task prefixes, org-style `DEADLINE:/SCHEDULED: <date>` stamps → `[[YYYY-MM-DD]]` links (the issue #63 model), and flat `{{[[query]]: {and: …}}}` queries become live ` ```query ` fences.
  Roam's folded blocks (`open: false`) survive as `Op::SetCollapsed`, components with no outl equivalent (`{{table}}`, `$$latex$$`, …) are preserved verbatim and counted, and slug collisions are disambiguated instead of silently overwriting.
  Three new flags: `--dry-run` (parse + report, write nothing — measure fidelity against a real backup before migrating), `--json` (full machine-readable report), and `--preserve-timestamps` (keep Roam create/edit times as `created::`/`edited::` properties).
  A large import (a 4.5k-page graph reconciles for several minutes) now paints a live progress line on stderr — phase, page counter, percentage bar, current page, elapsed — instead of sitting silent; it only renders on a TTY, so piped and CI runs stay clean.
  Anything unresolvable stays greppable (`((unresolved:uid))`) and every degradation is counted in the import report — silent loss is treated as a bug.
  **Logseq and Obsidian moved to the same adapter pipeline** (the legacy string pipeline is deleted), so every source accepts the same flags and produces the same rich report.
  The Logseq adapter now truly parses the outline: `id::` UIDs become real block-ref handles instead of page-link fallbacks, `collapsed:: true` survives as folded state, `DOING`/`NOW`/`LATER`/`WAITING`/`CANCELED` map to `TODO`/`DONE` with a `state::` property preserving the nuance, `[#A]` priorities become `priority::` properties, `SCHEDULED:`/`DEADLINE:` timestamps become `[[date]]` links, `:LOGBOOK:` drawers are dropped and counted, and `#+` directives plus leading `key:: value` lines become page properties.
  The Obsidian adapter keeps its full behavior (frontmatter policy, wiki-link collapsing, image-embed conversion, H1/title resolution, path-derived slug-collision suffixes, `path::` folder hints) on the new engine.
  `outl import auto <src> <dst>` detects the source from its shape (JSON file = Roam, graph dir = Logseq, `.obsidian/` vault = Obsidian).

- **The TUI's `g x` now opens the markdown link under the cursor when the block isn't code (issue #183).**
  Links `[text](url)` already rendered in the TUI (blue, underlined) but there was no way to follow one — the desktop and mobile apps let you click, the terminal had nothing.
  `g x` keeps running fenced code blocks exactly as before (code always wins), and only when the current block isn't code does it look for a markdown link under the cursor and open it in your system browser.
  The cursor can sit on the link's text or its URL — both open the same link, matching vim's `gx` on `.md` files.
  Only `http` / `https` / `mailto` links are opened; anything else is refused, the same guard the desktop uses.
  Following a `[[page]]` / `#tag` / `((block ref))` is still `Enter`, unchanged.

- **The pairing screen now shows live sync progress instead of a frozen "Loading…".**
  Pairing a device to a large workspace transfers a ~15 MB snapshot plus a couple hundred thousand ops, which takes the better part of a minute — and until now the screen gave no sign anything was happening.
  The Sync section (desktop) and the Devices sheet (mobile) now show, as a pass runs: a **real progress bar** for the snapshot download (the only phase with an honest percentage — the total is known from the frame's length prefix before the body arrives), a **live count** of ops received / sent (op totals are only known once a batch finishes, so they surface as a number, not a bar), and an **activity feed** of "device → what synced".
  For an incremental live sync the feed names the pages that changed (`MacBook Pro → journals/2026-07-18`); on the initial bulk pair it stays a count, since naming tens of thousands of pages is meaningless.
  Under the hood this is a second, purely-cosmetic channel out of the iroh transport (`outl_actions::SyncProgress` over the `sync-progress` Tauri event) — entirely separate from the load-bearing reload signal, so a dropped progress update can never affect what actually syncs.
  The feed's page names come from a new `resolve_page_labels` command that resolves the touched block ids to their page/journal slug; the engine caps how many it ships, so it stays cheap.

### Performance

- **A composite write — creating a page with content, pasting a subtree, running `outl batch`, importing a page — is now a single-digit-millisecond operation instead of tens to ~90ms (issue #192).**
  Every `Workspace::apply` used to end in its own `Storage::append_op`, and on macOS that call ends in `F_FULLFSYNC`, which costs roughly 4ms regardless of how little data moved.
  A "create a page" action that appended a root, a 7-block forest, and three properties fired eleven of those fsyncs back to back — the fsync, not the CRDT or the disk write, was the bottleneck.
  `Storage` grows `append_ops(&[LogOp])`: the default still loops `append_op`, but `JsonlStorage` overrides it to validate the whole batch, serialize every line, then open once, heal a torn tail once, write every line, and fsync **once** for the batch.
  `Workspace::begin_batch()` is the apply-side half — a new RAII guard (`WorkspaceBatch`) that every composite `outl-actions` function (`append_forest`/`append_tree`, page create, paste, template instantiate, block split) now opens around its multi-op body: each op still runs the full CRDT path one at a time, only the persist is deferred to one `append_ops` call per storage destination on commit.
  `outl batch` and `outl-md`'s external-edit reconcile (the largest single burst of ops in the system — a fresh import or boot re-diffs every block of every page) batch the same way.
  A dropped or errored batch still flushes whatever was applied before the failure, so `applied`/`failed_at` in `outl batch`'s response describe the exact same on-disk state the pre-batch per-op path did.

- **The TUI opens instantly and `Esc`/commit is snappy again on a large workspace.**
  On a ~2800-page vault, opening the journal and leaving edit mode stalled for over a second each.
  Two separate costs were behind it.
  First, **every commit rewrote the whole page's op log**: the diff defensively re-emitted `Create` + `Move` for every block and allocated fresh fractional positions each time, so every `Move` looked like a real reorder — an 11-block page fsynced 23 ops per keystroke-commit (slow `Esc`) and the log grew by the whole page every edit (slow boot).
  The diff now reuses each block's current position when its order is unchanged and filters ops that are already no-ops against the tree, so a one-block edit emits **one** op instead of 23 (measured 114 ms → 8.6 ms).
  Second, the **backlink index was built inline on the event loop** — reading all 2800 `.md` files (~1.75 s) on the first render and again on every whole-workspace change, which is what froze the open.
  It now builds on a worker thread (mirroring the existing workspace-index rebuild): the journal paints immediately and the "Linked from" panel fills in a beat later; a local edit patches just the current page's entries, and only whole-workspace changes (peer sync, plugins, page delete, cross-page edits) re-spawn the background build.

- **Desktop and mobile writes are now async — a commit never blocks the next keystroke.**
  Leaving edit mode used to render the page, SHA-256 the sidecar, and write both to disk synchronously on the IPC thread before the reply came back (tens of ms in release, hundreds in debug), and it fired the plugins' `onOp` sweep with an `await` on top.
  Now the commit only mutates the op log (the source of truth) and builds the reply view straight from the tree; the `.md` + sidecar projection is queued to a single background writer (`ProjectionWriter`), and the plugin sweep is fire-and-forget.
  The writer serializes and coalesces projections under the workspace lock, so the `.md`↔sidecar pair can never tear (no sync corruption), and a lagging projection is never data loss (the op log is truth; the next boot re-projects, and peers sync ops over iroh, not the `.md`).
  This brings the two GUI clients to the same async-on-write default the TUI has.

- **The TUI commit is now coalesced, so a burst of edits never blocks input.**
  Leaving edit mode used to persist synchronously — render `.md` + reconcile + fsync, tens of milliseconds — before the next keystroke could land, so typing `Esc o … Esc o …` stuttered.
  Now a commit boundary only marks the page dirty and repaints; the actual `render → write → reconcile_md → fsync` drains the instant the event loop goes idle (no keystroke waiting), so edits in a burst coalesce into one persist when the user pauses.
  A hard cap (600 ms) forces a flush even mid-burst so an unsaved edit can't linger, and every path that reads persisted state (navigation, peer reload, quit, `Ctrl+S`, `call:` re-run, code-block exec) flushes first — so nothing ever reads a stale `.md` or op log, and a quit never drops the last edit.

- **Opening today's journal is instant again on the desktop and mobile apps, even on a large workspace.**
  A ~66k-block / 211k-op workspace took seconds to paint the journal on first open (desktop noticeably, mobile much worse); the TUI stayed fast.
  The TUI was the clue: all three clients share the same op-log boot + snapshot, so the boot itself was fine — the GUI clients were paying for something the TUI does lazily.
  That something was **backlinks**: `build_page_view` (in the shared Tauri backend) computed `backlinks_for_page` synchronously — an `O(blocks-in-workspace)` scan — inside every page-open **and** every block-mutation reply, so the whole journal waited on a full-workspace walk before it could paint.
  The TUI has always computed backlinks lazily (cached, only when the panel is visible); the GUI now matches it.
  `PageView` no longer carries backlinks (the field stays in the wire shape but comes back empty); a new `page_backlinks(slug)` command computes them off the first-paint path, and each client fetches them after the outline renders (desktop via a slug-keyed effect, mobile via a `createResource`), so the panel fills in a beat later without ever blocking the journal.
  Mobile additionally moved its boot-time orphan-`.md` reconcile (a filesystem walk of every page) onto a background thread, the way the desktop already did, so it's off the cold-boot critical path too.

### Fixed

- **Opening a page with many backlinks is much faster (issue #169).**
  A user with a template referenced from 760 places reported multi-second page opens.
  The cause was `backlinks_for_page` being **quadratic**: it walks every block in the workspace and, per match, materializes the block's subtree — and both steps went through `children_of`, which rescans _every_ node in the tree on each call (`Tree` stores only `node -> (parent, position)`, with no child index).
  The walk now builds a `parent -> children` map once per call (one scan + per-parent sort; `O(n log n)` worst case) and threads it through the walk and subtree projection, eliminating the `O(n²)` rescans.
  This still does a full workspace walk (no inverted index), but the report's shape — 760 backlinks in a ~35k-block workspace — drops from **3.83 s to 41 ms** (~94×), with **no change to results** (the full backlinks test suite is the correctness oracle).
  This is a pure internal refactor: `backlinks_for_page` / `project_outline` / `project_outline_node` keep their signatures and output; the index is scratch state rebuilt per call, never cached, so it can't go stale.

- **Snapshot fast-boot now actually works in production, and can no longer drop a synced edit (issues #156, #128, #109).**
  The materialized-state snapshot that short-circuits full op-log replay on open was **inert in production**: the workspace wrote it to `<root>/.outl/snapshots` while the storage backend read it from `<root>/snapshots` (it derived the path from `ops_dir.parent()`, but production keeps the op log at `<root>/ops`, not `<root>/.outl/ops`).
  Writer and reader never met, so every boot silently fell back to a full replay — every existing test passed only because tests use `<root>/.outl/ops`, which makes the two paths coincide.
  Snapshot I/O is now owned entirely by `Workspace` (read/write straight to `<root>/.outl/snapshots`, keyed off the workspace `root`) and was removed from the `Storage` trait, so there is a single path derivation and the two-owners divergence can't recur.
  Naïvely fixing only the path would have armed a **silent data-loss bug**: the replay cutoff was a single global HLC, so a legitimately-low-HLC op from a lagging or offline peer, delivered after the snapshot, sat below the cutoff and vanished from the tree even though it was durably in storage.
  The cutoff is now a **per-actor vector clock** — boot replays, for each actor, every op above that actor's own high-water mark plus every op of an actor the snapshot never saw — so no concurrent write is ever dropped (snapshot boot stays observationally identical to a full replay, guarded by the convergence property suite).
  The long-lived clients (TUI, desktop, mobile) now enable the snapshot policy from `outl.toml` on open and flush a final snapshot on graceful shutdown, so the CLI's next invocation boots from it instead of replaying the entire op log (#109).

### Changed

- **`crates/outl-core/src/storage/jsonl.rs` split into cohesive modules (issue #161).**
  The op-log backend (1189 lines, past the file-size guard's hard limit) is now `storage/jsonl/{mod,read,append,tests}.rs`, each comfortably under 600 lines, with no logic or public-surface change.

## [0.8.0] — 2026-07-11

### Added

- **Multi-block batch operations, on the TUI and desktop (issue #23).**
  The TUI's Visual mode (`V`) gains **reorder**: `Alt+↑` / `Alt+↓` drag the whole selection among its siblings (mirror of the single-block `Alt`+arrows in Normal), alongside the existing range delete / indent / outdent / yank.
  On the desktop, multi-select no longer requires vim mode: **`Shift+↓` / `Shift+↑`** start and grow a contiguous block selection from anywhere (the non-vim entry), and a floating **batch toolbar** appears — `N selected` plus Indent, Outdent, Move up, Move down, Delete, and Done — so the range ops are reachable by mouse instead of only by chord.
  The toolbar fires the **same** `action-handlers` the keyboard does, so button and chord can't drift; only the toolbar's Delete confirms before erasing a range with nested children (the keyboard delete and the TUI erase without a prompt, matching vim).
  The range reorder (`Cmd/Ctrl+Shift+↑/↓` in Visual) loops the existing per-block move action, walking bottom-up for move-down so a block never drags over its own not-yet-moved neighbour; the selection follows because block ids are stable across the re-render.
  All four new bindings live in the shared `outl-shortcuts` catalog (`SelectRange{Down,Up}`, `MoveVisualRange{Up,Down}`), so a future client inherits them.
- **Template engine — reusable block structures and callable code blocks (issue #146).**
  Any page becomes a template the moment it gets a `template:: <name>` property; the page's outline is the template body, so templates are searchable, have backlinks, and sync like any other page — no special folder, no file-based config.
  Two invocation modes.
  **Structural** (`/template <name>` in the TUI, `outl template apply <name> --page <slug>` on the CLI, `outl_template_apply` over MCP) deep-copies the template's subtree under the target block, minting fresh `NodeId`s and op-log entries, and substitutes the built-in variables `{{date}}`, `{{today}}`, `{{yesterday}}`, `{{tomorrow}}`, `{{page}}`, and `{{time}}` in the block text.
  **Callable** (a ` ```call:<name> ` fence, run with `gx` in the TUI or the Run action on desktop) resolves the named template's code block, injects the `params::` declared on the call block, and executes it through the existing `outl-exec` runtimes — Roam's `{{roam/render}}` without a ClojureScript runtime.
  Callable execution lives once in `outl_actions::run_callable_block` and is intercepted inside the shared `run_code_block` action, so the desktop and mobile Run paths get `call:` fences for free instead of erroring "no runtime for `call:<name>`".
  On the GUI clients a `call:<name>` fence now renders as a proper code block (with a language chip and Run button) — the shared `detectFence` info-string pattern accepts the `:` so the block is no longer left as raw ` ``` ` text, and its `key: value` params are syntax-highlighted (as YAML) instead of rendering flat.
  Finishing an edit on a `call:<name>` block re-runs it automatically, so the `> **result:**` reflects the freshly-typed params without a manual `gx` / Run — on the TUI (Insert commit) and both GUI clients (the shared `edit_block` command).
  **Every template page shows where it was used.** The template page's backlinks panel now lists every block that rendered it (a `call:<name>` fence) or instantiated it (`from-template:: <slug>`), so you jump from a template to its call sites with no hand-written `[[link]]` — the matcher reads the fence and the provenance property directly, not just plain `[[refs]]`.
  **The daily journal is now a template too.** `outl init` creates a `templates/journal` page (`template:: journal`) instead of a `templates/journal.md` file, and opening a fresh daily note stamps that template automatically — the built-in variables resolve against the daily's date. Existing customized `templates/journal.md` bodies migrate into the page on `init` (best-effort).
  Callable params are injected as JSON (`serde_json`), so a value containing a quote can't break — or inject into — the generated program, and the language is canonicalized so `py`/`python3`/`node` aliases still receive the params prelude. Built-in date/time tokens resolve through the workspace clock (honouring `[calendar] timezone`), matching the journal date instead of reading UTC in containers.
  The engine lives once in `outl_actions::template` and every client wraps it (TUI, CLI, MCP), so the semantics stay identical across surfaces.
- **Paste with formatting now brings rich clipboard formatting across (bold, italic, links, lists) on the GUI clients.**
  Copying a formatted message — a Slack post, a Google Doc paragraph, a Notion block, a Gmail draft — puts the bold/italic/links/lists on the clipboard's `text/html` flavour; the `text/plain` flavour is stripped of them.
  The desktop and mobile paste used to read only `text/plain`, so a pasted Slack message arrived flat.
  It now reads `text/html` first and converts it to outl markdown (via **Turndown**, tuned for the outl dialect: `*italic*` not `_italic_`, `-` bullets, `~~strike~~`, and Slack `:emoji:` kept from the image alt text), then routes it through the same paste pipeline — so the formatting and the bullet structure survive.
  Google Docs (and other editors that encode weight as inline CSS) are handled too: a `font-weight:700` span becomes `**bold**`, and the `<b style="font-weight:normal">` wrapper Docs wraps the whole payload in no longer bolds the entire block.
  Plain text with no richer HTML behaves exactly as before.
  The converter lives once in `@outl/shared/paste` (`htmlToOutlMarkdown`) so both GUI clients stay identical.
- **Paste with / without formatting, with explicit chords per client.**
  "With formatting" routes the clipboard through the conversion pipeline: outline syntax (Roam `{{[[TODO]]}}`, GitHub `- [ ]`, Logseq) is normalized to the outl dialect, and **plain multi-paragraph text is split into one block per paragraph** — a pasted chat reply or email lands as a readable outline instead of one wall-of-text block (blank line = paragraph break; soft line wraps stay in one block).
  "Without formatting" splices the raw clipboard text into the current block, no conversion, no splitting.
  Desktop: `Cmd/Ctrl+V` = with formatting, `Cmd/Ctrl+Shift+V` = without.
  TUI: `p` = with formatting, `Shift+P` = without (both read the OS clipboard now; the old `p`/`P` yank-register paste is folded into this since copy mirrors the register to the clipboard).
  Mobile: paste is always with formatting.
- **Copy a block selection as clean markdown to the OS clipboard, in every client (issue #114).**
  Copying out of outl used to be a mess — selecting a block in the TUI with the terminal's mouse copied the on-screen tree guides (`│`), bullets, and fold markers, so pasting elsewhere produced garbage.
  Now every yank/copy writes the **canonical outl markdown** for the selection (each block plus its subtree) to the clipboard, so it re-pastes into outl as the same tree and reads as a tidy bullet list anywhere else.
  TUI: `yy` / `Y` / Visual `y` write the markdown to the clipboard via `arboard` **and** an OSC 52 escape, so it reaches the clipboard over SSH, inside tmux, and in Chrome OS **Crostini** where `arboard` has no display server (the in-app yank register that `p`/`P` reads is still filled too).
  Desktop: `Y` / Visual `y` copy the selection as markdown via `navigator.clipboard`.
  Mobile: the long-press **Copy** action copies the block and its subtree as markdown instead of a single block's raw text.
  A Visual range spanning a parent and one of its children no longer duplicates the child — the shared serializer drops any block whose ancestor is also selected.
  This is the copy-out inverse of the existing paste-in conversion, so the two round-trip; the serializer lives once in `outl_actions::copy_markdown` and every client wraps it.
- **Opt-in mouse support in the TUI — new `[tui] mouse_capture` config key.**
  Set `[tui] mouse_capture = true` in `~/.config/outl/config.toml` and the TUI captures the mouse: the scroll wheel moves the selection, a click selects the block under the pointer, and a drag selects a range that is copied to the clipboard as markdown on release.
  Default `false`, and deliberately so — capturing the mouse disables the terminal's own text selection (selecting a URL, copying a single word), which is muscle memory for many terminal users.
  The keyboard yank copies markdown to the clipboard regardless of this flag.

### Changed

- **P2P sync now defaults to outl's dedicated relay (`use1-1.relay.avelino.outl.iroh.link`) instead of the shared n0 public pool.**
  The relay only ever sees end-to-end-encrypted bytes (never your notes), but it _can_ observe coordination metadata — which two devices sync, and when.
  Defaulting to a dedicated, outl-scoped relay endpoint (hosted on n0 infra under our `*.iroh.link` namespace) is the first step toward a fully outl-owned relay; n0's shared relays remain the documented fallback (a malformed `[sync] relay_url` degrades to them rather than failing the bind).
  No action needed — a device with an empty / omitted `[sync] relay_url` picks it up automatically. Point `relay_url` at any `iroh-relay` to override. See `docs/relay.md` (the vanity `relay.outl.app` name is on the roadmap, pending TLS).

### Fixed

- **Template footguns from the issue #146 release audit.**
  Three sharp edges in the template engine (issue #146) are fixed.
  **Callable vs structural dispatch now keys off the presence of a runnable code block**, not on whether `params::` is declared — a callable template with a code block but no `params::` used to be misrouted as structural, so its ` ```lang…``` ` fence got deep-copied as literal text instead of executing; it now runs (with an empty `params`).
  **Duplicate template names are visible** — when two pages share a `template:: <name>`, resolution still picks the first in tree order but now logs a `tracing::warn!`, and `list_templates` flags the collision on each `TemplateEntry` (`duplicate`) so a client can surface it.
  **Plugin-instantiated templates honour the target page's date** — the host derived `{{date}}` from _today_ even on a journal page (`page_date: None`); it now derives it from the target slug, matching the CLI/TUI path.
- **P2P sync no longer reports a false "sync ok" that silently drops a device's edits.**
  A desktop-initiated delta-sync logged `catch-up: sync ok` as soon as it finished _writing_ its push — never confirming the peer durably _ingested_ it.
  Over a lossy desktop → mobile path (a backgrounded iPhone / carrier NAT), the connection could tear down cleanly for the initiator while the mobile never persisted the pushed ops, so a page edited on the desktop stayed empty on the phone even though sync claimed success.
  The responder already closes the stream with a `done` sentinel **only after** a durable ingest; the initiator now **requires** that sentinel before reporting success (a lost close on an otherwise-successful ingest just costs a harmless, deduped re-push).
  Regression: `initiator_reports_failure_when_responder_never_confirms_ingest`.
- **Sync connect no longer stalls ~30s on a stale peer address, and self-heals when a device moves networks.**
  iroh 1.0.0's QUIC multipath opens a path to every stored address at once and wedges ~30s on a dead one; a peer's old on-LAN IP (still in `peers.json` after it moved Wi-Fi / went cellular) passed the subnet filter and stalled every catch-up tick.
  Each connect attempt is now bounded by a timeout, and a stalled/failed direct dial falls back to a **bare-node-id** dial so iroh's relay + discovery resolves the peer's _current_ address instead of retrying the dead one forever.
  On top of that, when a peer dials _in_ directly, outl reads the live socket off the connection and rewrites the stored address to it (dropping the stale one), so the next outbound dial uses the fresh route with no re-pair — the stored address self-heals the moment the peer reconnects.
- **Mobile stops flip-flopping a page between two devices' states on the sync poll.**
  The mobile's routine reload (every ~3s) ran the orphan-`.md` reconcile + desync-recovery **inline** — operations that _mutate_ the op log (md → ops). On a page being edited on two devices while sync ingested peer ops, the desync-recovery false-positived on the racing read and minted fresh ops each poll, so the page oscillated between the desktop's and the phone's versions (and briefly flashed an empty "0 ops" state).
  Reconcile/recovery is a **boot** concern (a stable moment, no concurrent ingest); iroh peers ship _ops_, not `.md`, so a routine reload only needs to re-materialize the op log. The reload is now a pure re-read — orphan `.md` recovery still runs once at boot — and the reload no longer clobbers real content with a transient empty read.
- **Callable-template results stop churning the op log and oscillating across devices.**
  The `> **result:**` subtree was deleted and recreated on every run with fresh node ids, so two devices running the same `call:` block fought a delete/recreate war (each deleting the other's result), bloating the op log into the thousands and flip-flopping the page between the two devices' outputs.
  The result now uses a **deterministic node id derived from the call block** and updates in place, so re-runs are idempotent and two devices converge on one result (last write wins per line) instead of competing subtrees.

- **iroh sync survives restrictive networks — custom-CA proxies and post-VPN stale peer addrs (issue #133).**
  Two blockers that stopped Mac ↔ iOS sync on corporate / VPN networks are fixed in code.
  **Relay TLS with a custom root CA:** a network with a TLS-inspection proxy (its root CA installed in the OS keychain) had every relay handshake rejected with `invalid peer certificate: UnknownIssuer`, because iroh trusted only Mozilla's bundled roots — not the OS trust store that macOS / `curl` / Safari already honour.
  outl now delegates relay-TLS validation to the OS keychain (iroh's `platform-verifier` + `CaTlsConfig::system()`, wired once in `bind::n0_builder_ipv4_only`), so a keychain-trusted proxy cert is accepted.
  **Stale VPN/tunnel IPs after pairing:** a device paired while on a VPN captured its tunnel IPs (`10.x`, `100.x` CGNAT, a public WAN addr) into `peers.json` alongside the real LAN address, and iroh 1.0.0's multipath opened a path to each — stalling even same-WiFi direct sync on the dead paths (`MultipathNotNegotiated`) with the reachable `192.168.x` addr right there.
  A dial now keeps only the direct IPv4 addresses that share a subnet with a local interface, dropping unreachable tunnel IPs before they can stall the connect (the relay still covers genuine cross-network peers).
  A third failure mode — a proxy that blocks the relay's WebSocket `Upgrade` and returns `502` — is environmental (iroh 1.0.0 has no non-WebSocket relay transport); the workaround is a self-hosted `[sync] relay_url`, now documented under `docs/relay.md` → "Troubleshooting: restrictive networks".
- **Underscores inside a word no longer render as italics.**
  The inline tokenizer paired any `_…_` as emphasis, so pasted identifiers like `chamados_chat`, `inc_lag1`, `prod.ml_atendimento`, or `databricks_2_train` rendered half-slanted.
  outl now follows CommonMark: `_` only opens or closes emphasis at a word boundary, never intra-word (`*` stays the intra-word marker).
  A standalone `_italic_` still works.
- **`o` / new-line no longer errors with "block … is not in the tree" after a background reload.**
  On the desktop, a peer-driven reload (`peer-ops-changed`) replaced the outline without clearing the editing / selection cursor, so a block being edited could keep an id the reload had re-materialized or dropped — the next edit or new block then hit "block … is not in the tree".
  The reload now prunes a stale `editingBlockId` / `selectedBlockId` against the fresh outline, and `create_block` (desktop + mobile) falls back to appending at the page end if its anchor is gone.
- **Pasting into a freshly-created empty block no longer errors with "block … is not in the tree".**
  A block created with `o` (or a new line) carries only an `Op::Create`, no `Op::Edit`, so it has no materialized text yet — and the caret-paste path guarded the host's existence on `block_text`, which returns `None` for a text-less block, so pasting into a brand-new empty block was rejected as if the block didn't exist.
  The paste (with **and** without formatting) now checks the tree for the block and treats a missing text as empty, so it grafts into the new block on desktop and mobile.
- **Desktop error messages surface as a top-right toast instead of a hidden bottom banner.**
  The error surface used to render a full-width banner at the base of the outline, where the fixed bottom-left chrome cluster painted over its left edge — the message was half-covered.
  Errors now appear as a floating toast in the top-right notification corner, above every chrome element, with nothing overlapping it.
- **Journal date and status-line clock honour a configured timezone — new `[calendar] timezone` config key.**
  The journal's "today" and the TUI clock used to call `chrono::Local::now()`, which trusts the operating system's local timezone.
  In containers and Chrome OS **Crostini** the OS clock runs in UTC regardless of where the user is, so the date landed on the wrong day near midnight and the clock read an hour off (issue #107).
  A user can now set `[calendar] timezone = "Europe/London"` (any IANA name) in `~/.config/outl/config.toml`; the journal date and clock resolve in that zone, DST-aware.
  The fix is opt-in: with no timezone configured the clock stays on the OS local timezone, so nothing changes for a normally configured machine.
  Internally this is the new `outl_actions::clock` module (`init` / `now_local` / `today`, backed by `chrono-tz`); every client calls `clock::init` once at boot and every "today" routes through it (`page::today` delegates), so there is a single source of truth for the user's wall clock.

### Added

- **JavaScript plugin system (`outl-plugins`), shared by every client.**
  Plugins are bundled JavaScript described by a `plugin.json` manifest; a plugin written once runs on every outl client because it talks to the new `outl-plugins` crate, never to anything client-specific.
  The engine is **Boa** (pure-Rust, runs on iOS — no JIT, reused from `outl-exec`) behind a `PluginEngine` trait so it can move to QuickJS later only if a measured blocker appears.
  Execution is **describe → apply**: the JS side reads a pre-computed `ReadModel` and emits `HostIntent`s; the host drains them through `outl-actions` → `Workspace::apply`, so the op log stays the single source of truth and `.md` stays 100% clean.
  Live capabilities: `op-hook` (`onOp`), `slash-command`, `keybinding`, `config-schema` (read), `toolbar-button`, `ui-render`, and `content-transformer:text` / `:rich`; `sync-transport` is core-ready (convergence tested) but no client polls it on a timer yet.
  `keybinding` fires a bound chord from the **TUI** (Normal mode, single + two-chord, never overriding a native binding) and the **desktop** (a native binding always wins); `toolbar-button` renders a chrome button on desktop + mobile and surfaces the command in the TUI slash menu; `content-transformer` (`ctx.content.register(lang, fn)`) renders a fenced block as text on every read surface (`:text`, inline in the TUI) or as HTML in a sandboxed iframe on the GUIs (`:rich`).
  New host namespaces: `ctx.net.fetch` (blocking HTTP gated by `network:<domain>`; a denied domain returns `{ ok: false, error }` rather than throwing), `ctx.storage.{get,set,delete}` (per-plugin local KV gated by `storage:local`, stored at `<workspace>/.outl/plugins/<id>/storage.json`, deliberately outside the op log so it never converges), and `ctx.sync.register({ push, pull })`.
  A query engine plugs in as a `content-transformer` for the `query` fence — there is no separate `query-provider` capability, and inline `{{query}}` waits on a markdown token the parser defers.
  A capability the current client can't honor loads partially with a warning (never a crash); every host call is gated against the user-approved permission set (`read-page`/`write-page`/`read-op-log`/`submit-op`/`storage:local`/`network:<domain>`, with bare `network:*` rejected).
  Anti-loop is structural: `PluginHost` tracks how far into the op log it has dispatched, so a plugin's own ops never re-trigger its hooks.
  A runaway plugin can't wedge the engine either: Boa runs under `RuntimeLimits` (loop-iteration cap ~20M, recursion cap ~2000, stack cap), so an infinite loop or unbounded recursion surfaces as a JS error instead of a hung thread.
  Wired into **TUI** (plugin commands in the slash menu, `onOp` after each mutation) and the **CLI** (`outl plugin list / install / run / enable / disable / remove`, the last aliased `uninstall` / `rm`); the desktop/mobile wiring runs the host on a dedicated thread (the Boa context is `!Send`).
  Distribution day-zero: install from a local directory (`github:` source to follow), a `bundleHash` revalidated on every load, a per-workspace `installed.json` lockfile freezing the approved permissions, and a static `registry.json` index (the "store").
  Authors get `@outl/plugin-sdk` (typed `definePlugin` + host API) and two working examples: `examples/todo-archiver` (archives DONE blocks to a configurable page) and `examples/confetti` (throws a confetti burst when a block is marked DONE).
  The `ui-render` capability lets a plugin hand the GUI clients (desktop + mobile) a chunk of author-written HTML/JS via `ctx.ui.render(html)`, which they run in a **sandboxed iframe** (`sandbox="allow-scripts"`, no same-origin — isolated from the app DOM) as an ephemeral full-screen overlay.
  The host stays agnostic: it only transports the string the plugin produced, so the visual is the author's creativity, not a fixed catalog of effects.
  The TUI/CLI drop `ui-render` payloads (no webview); the op-hook still fires.
  New `outl_actions::block::move_under` (re-parent a block under an arbitrary page/block) backs the plugin `Move` intent.
  See `docs/plugins.md`, `docs/plugin-api.md`, and the manifest schema at `docs/schemas/plugin-v1.json`.
- **`:shortcode:` emoji syntax + autocomplete across every client.**
  The outl inline dialect now recognises GitHub-style gemoji shortcodes (`:tada:`, `:rocket:`, `:smile_cat:`, `:+1:`, `:100:`) and renders them as the unicode glyph (🎉, 🚀, 😸, 👍, 💯) on every read surface.
  The catalog is the [`emojis`](https://crates.io/crates/emojis) crate (Unicode CLDR + GitHub aliases, ~1800 shortcodes) so `outl_md::emoji::search` is the one ranking source TUI, mobile, and desktop share through a single `outl_emoji_search` Tauri command — no parallel index on the JS side.
  **Disk form is always the shortcode literal** (`:tada:`, never 🎉) so `.md` files stay greppable, diffable, font-independent, and safe across iCloud / Syncthing.
  The parser only tokenises `:foo:` when the catalog recognises `foo`; unknown runs (`:notarealemoji:`, `meeting at 14:00 :`) stay plain.
  URL boundaries fall out for free — the strict `[a-z0-9_+-]+` shape + catalog gate reject `https://example.com:8080/api`, `mailto:foo@bar.com`, and `git@github.com:avelino/outl.git` without a look-behind pass.
  Typing `:roc` inside any block opens a popup with the top eight matches (`outl_md::emoji::search`, exact → prefix → substring, shorter shortcodes win ties); `Tab`/`Enter` commits the canonical `:rocket:` form into the buffer.
  Wired into `outl-tui` (`AutocompleteKind::Emoji` + the existing overlay machinery), `outl-mobile` (UIKit chip strip via `buildEmojiShowMessage`), and `outl-desktop` (floating `EmojiSuggestPopup` under the textarea, parallel to `RefSuggestPopup`).
  The shared `@outl/shared/autocomplete::detectEmojiContext` + `applyEmojiSuggestion` own the trigger detection and splice so the three GUI surfaces stay byte-identical.
  See `docs/markdown-format.md` § "Emoji shortcodes" for the full dialect contract.
- **`@` mention autocomplete** — typing `@` in any block opens a person picker filtered to pages with the `type:: person` page-level property, fuzzy-matched against the typed name.
  Accepting a candidate inserts `[[@name]]`, a regular wikilink whose `@` is the link affordance only (page identity stays clean, slug has no `@`).
  Composite names like `@Thiago Avelino` work because the autocomplete query allows spaces.
  A "create new" candidate appears when the typed query doesn't match any existing person.
  Accepting it materialises a fresh `pages/<slug>.md` with `type:: person` already set, so the next mention of the same name surfaces it without manual property editing.
  Wired identically in `outl-tui`, `outl-desktop`, and `outl-mobile`.
  The shared `@outl/shared/autocomplete` library owns the trigger detection and the create-new helper for the GUI clients.
- **`type::` page-level property** — surfaced on `outl_md::WorkspaceIndex::PageEntry.page_type` and `outl_actions::PageMeta.page_type`.
  New filter `WorkspaceIndex::pages_by_type(t)` and consumer `outl_actions::page::search_persons(ws, query)` rank `type:: person` pages for the `@` mention popup.
  `type::` is just one of many user-facing page properties (`title::`, `icon::`, `pinned::`, `role::`, anything custom) and all of them now reach the workspace tree (see "Fixed" below).
- **`ref-projection-failed` Tauri event** — emitted by the desktop and mobile clients when `open_ref` resolved a target (the page is in the op log) but writing the resulting `.md` + sidecar failed.
  Frontend can listen via `onRefProjectionFailed` (desktop) and surface a toast so the user knows the link they just inserted isn't visible to peers yet.
  The op log retry on the next save / orphan scan is still automatic.

### Fixed

- **TUI now word-wraps block text to the pane width** ([#99](https://github.com/avelino/outl/issues/99)).
  Typing past the right edge of the terminal used to run a block off-screen instead of flowing onto the next visual row — terminals don't reflow on their own, and the outline deliberately avoided ratatui's `Paragraph::wrap` because that expands lines _after_ layout and would desync the `selected_line` scroll index.
  The outline now pre-wraps itself (`outl-tui` `view::wrap::push_wrapped`): wrapped rows are emitted up front so the scroll index stays honest, the first visual row keeps the bullet/fold marker, continuations re-indent under the text column, and the `│` indent rails repeat top to bottom.
  Wrapping runs on the already-styled spans (post-tokenization), so a break never splits a `**bold**` / `[[ref]]` token back into its literal markers, and wide glyphs (CJK, emoji) count as two cells.
  The block being edited (Insert) or selected in Normal mode stays on one line so the cursor column keeps matching the source bytes.
- **Page-level properties now reach the workspace tree.**
  The reconcile pipeline previously emitted `Op::SetProp` only for block-nested properties.
  Anything written at the top of a `.md` (above the first `-` bullet) — `title::`, `icon::`, `pinned::`, `type::`, custom keys — lived only on disk.
  The workspace CRDT never learned about them, so any consumer reading via `workspace.tree().property(page_id, …)` (desktop sidebar, mobile picker, `outl_actions::search_persons`) silently disagreed with the rendered markdown.
  The TUI hid the bug because its `WorkspaceIndex`-backed surfaces parse `.md` straight from disk.
  Cross-client divergence on every workspace populated outside the in-app picker (fixtures, vim users, Logseq/Roam imports, peers via iCloud) was the result.
  Fix: `outl_md::diff::diff_to_ops_with_page_props` emits `Op::SetProp` on the page root for every page-level property in the parsed AST.
  `outl_md::reconcile::reconcile_md` calls it on every reconcile pass.
- **Page root now materialises in the tree.**
  Pages authored externally never received an `Op::Create`, only the blocks under them did.
  Combined with the CRDT contract that `Op::Move` on a node without a preceding `Op::Create` is a no-op (an intentional design for peer-sync ordering), this left the page node as a ghost: present as `parent` of its blocks but absent from `children_of(NodeId::root())`.
  `list_all_pages`, `search_persons`, and the sidebar all skipped externally-authored pages silently.
  Fix: `outl_md::reconcile::ensure_page_root_in_tree` emits `Op::Create` when the page node is absent from `self.nodes`, or `Op::Move` when it exists at the wrong parent, plus `Op::SetProp` for `page-slug` / `page-kind`.
  Idempotent: 0 ops emitted on pages that are already materialised.
- **`open_ref` regenerates `.md` after creating a page.**
  Both desktop and mobile previously left newly-created pages on the op log only.
  The `pages/<slug>.md` projection never landed on disk until something else triggered `apply_page_md_with_sidecar` on that page.
  `WorkspaceIndex` (which parses `.md` from disk) disagreed with the tree CRDT silently, and a peer pulling the workspace via iCloud would never see the page at all.
  Fix: both clients now call `apply_page_md_with_sidecar` immediately after the `open_or_create_by_ref` mutation; failures emit `ref-projection-failed`.
- **`open_or_create_by_ref` no longer drops the `@` arm via slug normalisation.**
  `slugify("@avelino")` strips the `@` and returns `"avelino"`.
  The generic `find_by_slug(slugify(target))` branch used to run before the `@` arm, so a pre-existing `pages/avelino.md` (created before this feature, or by an external editor without `type:: person`) resolved via the generic path and returned early, never marking the page as a person.
  Fix: the `@` arm runs first and idempotently sets `type:: person` on every resolution, even when the page already existed.
- **`reconcile_md` reads the sidecar once.**
  The short-circuit check used to re-read the sidecar file separately from the diff inputs, racy if another process rewrote the sidecar between the two reads.
  Fix: single read, both consumers share the result.
- **Background-thread reconcile after open.**
  Opening a workspace used to block on a synchronous `reconcile_md` pass across every legacy page.
  With many pages, the first paint waited tens of seconds.
  Fix: `outl-desktop::workspace_open::spawn_background_reconcile` runs the orphan reconcile on a separate thread, locks the workspace per page (released between iterations), and emits `workspace-reconciled` on completion.
  Today's journal opens immediately; legacy pages materialise behind the scenes.

### Migration

- **`pipeline_version` in the sidecar drives forward-compatible re-reconciles.**
  The first boot on an upgraded binary scans every `pages/<slug>.outl` and re-runs `reconcile_md` on any sidecar whose `pipeline_version` is below the binary's `CURRENT_PIPELINE_VERSION`.
  Idempotent: the pipeline emits the same `Op::Create` / `Op::SetProp` ops that would have been emitted on first ingestion of the `.md`, the CRDT deduplicates by LWW, and the sidecar persists the bumped version.
  Two clients opening the same legacy workspace at the same time will each run their own reconcile (each actor owns its `ops-<actor>.jsonl`).
  The log inflates by roughly 2× the page-root ops once per legacy page per device.
  Acceptable for a one-shot migration: the CRDT converges deterministically.
  Subsequent boots skip the page via the `last_synced_hash` + `pipeline_version` short-circuit.

**Desktop client ships.**

`outl-desktop` (Tauri 2 + Solid + Tailwind) lands as the third client alongside `outl-tui` and `outl-mobile`, sharing the same `outl-actions` surface, the same op log, and the same workspace on disk.
Three new Rust crates (`outl-config`, `outl-theme`, `outl-shortcuts`) extract per-client config + palette + chord catalog out of the TUI so both clients converge on one source of truth; `@outl/shared` (`crates/outl-frontend-shared`) does the same for the Solid + DTO frontend code mobile and desktop both need.

The MINOR bump is the desktop addition; CRDT, sidecar, and existing CLI/TUI/mobile contracts are unchanged.

### Added

- **`outl-desktop`** — Tauri 2 client for macOS, Linux, Windows. 2-pane layout (Sidebar / OutlineView with inline backlinks at the bottom, mirroring the TUI), mini-calendar + pinned + recent in the sidebar, `outl-exec` code-block execution, cross-platform FS watcher (`notify`) that emits `peer-ops-changed` so the frontend reloads when iCloud / Syncthing / shared FS drops a peer's `ops-*.jsonl`. Distributed as a **universal macOS dmg** (arm64 + x86_64 lipo-merged) via `brew install --cask outl-desktop-beta`.
- **`outl-config`** — shared TOML config at `~/.config/outl/config.toml` (XDG on every OS — Windows routes through `dirs::config_dir()` to `%APPDATA%`). Read by TUI / CLI / desktop through the same `outl_config::load()` so a theme set in the desktop's Settings modal lights up in the TUI on the next launch.
- **`outl-theme`** — palette catalog with the seven existing presets (`outl`, `default-dark`, `light`, `dracula`, `solarized-dark`, `nord`, `monokai`). TUI derives its `Theme::from_palette` from here; desktop ships the palette over the Tauri wire and writes CSS custom properties.
- **`outl-shortcuts`** — `(chord → action)` catalog every client consumes. TUI translates `crossterm::KeyEvent` → `Chord`; desktop's `KeyboardEvent` adapter does the same. One binding change lights up on both clients.
- **`outl-frontend-shared`** (`@outl/shared`) — pure TS+Solid lib with the `MarkdownInline` renderer, paste / autocomplete helpers, DTO types, and the typed `invoke<T>()` wrappers every client uses. Mobile already consumed these locally; promoted in this release.
- **`PageMeta.pinned`** — the `pinned:: true` page property is now surfaced on `PageMeta` (matching `outl-md::index::PageEntry.pinned` exactly so the two never drift on which literals count as truthy). Sidebars on TUI + desktop pick it up.
- **Backlinks navigable on desktop** — `j`/`k` extends past the outline's last block into the inline backlinks section; `Enter` opens the source page and parks the cursor on the referencing block. Mouse click does the same. Mirrors what the TUI already did.
- **Workspace path fallback for `outl` with no args** — `outl_config::load().workspace.last` is consulted between `--workspace <DIR>` and the cwd, so the TUI lands on whatever workspace the desktop opened last with no flag.

### Changed

- **TUI sidebar chord** — `\` → `Ctrl+E` (mirroring desktop's `Cmd+Shift+E`, the VS Code "Show Explorer" convention).
- **TUI backlinks chord** — `B` → `Ctrl+B` (mirroring desktop's `Cmd+Shift+B`; we kept `Cmd+B` reserved for the universal markdown "bold" chord in Insert mode).
- **Sidebar + backlinks default to hidden** on the desktop now, matching the TUI's editor-hero defaults. Users opt the panes in with the chord.
- **Docs** — new `docs/shortcuts.md` (action × client matrix, where each chord lives in the code), `docs/config.md` (full TOML schema + per-OS path), `docs/homebrew.md` covers the desktop cask install + first-launch Gatekeeper workaround for the unsigned dmg.

### Fixed

- **Windows config path** — `outl-config::paths::config_dir()` now branches through `dirs::config_dir()` on Windows so the config lands under `%APPDATA%\outl\` (not `%USERPROFILE%\.config\outl\`, which is not a Windows convention).
- **`is_truthy` parity** — `outl_actions::page::is_truthy` no longer accepts `"pinned"` as a truthy literal; the set is now identical to `outl_md::index::is_truthy` (`true` / `yes` / `1` / `on`), so a hand-edited `.md` matches what the workspace index would also pick up.
- **fs_watcher Windows test** — `non_utf8_filename_is_ignored` is gated with `#[cfg(unix)]` (uses `OsStringExt::from_vec`), and `watched_root_label` tests now use `std::env::current_dir()` as a platform-portable absolute path anchor instead of the hardcoded `/tmp/ws` literal (not absolute on Windows).
- **Desktop outline scroll + narrow-window reflow** — body / `#root` now use `height: 100%` (was `min-height: 100vh`, which let the page grow with content and broke the height chain). `<main>` gained `min-w-0 min-h-0`; the AppShell grid template uses `minmax(0, 1fr)` instead of `1fr`. Same `min-width: auto` pitfall on both flex and grid axes; both unlocks pair.

### CI / Release

- **`desktop.yml`** — split into `check` (Linux, runs Clippy + Rust tests + Vitest + tsc + tauri bundle once) + `build` matrix (macOS arm64 + Windows x86_64 just compile + bundle). macOS x86_64 dropped from the PR matrix because the `macos-13` Intel runner pool is consistently depleted; release-time x86_64 binaries still ship via the universal dmg.
- **`release.yml`** — adds `build_desktop` (universal macOS dmg on `macos-latest`) and a single anchor in the bump-tap step so `Casks/outl-desktop-beta.rb` rides alongside `Formula/outl-beta.rb` on every push to main.

## [0.5.3] — 2026-06-02

**Unify backlinks, Insert-mode cross-block nav, anti-duplication policy.**

Two parallel backlinks pipelines (one on `outl-md::index`, one on `outl-actions`) had drifted on policy — self-references were dropped on the TUI panel but kept on mobile, and the user had to spot the divergence by comparing surfaces. 0.5.3 collapses them into one path through `outl_actions::backlinks_for_page`, deletes the cache on `outl-md::index`, and renames the related helpers so the call sites land on the shared API by default.

Insert mode also picks up the missing piece for vim/emacs muscle memory: `Up`/`Down` cross blocks (commit, move selection, re-enter Insert preserving the cursor column) the same way `Left`/`Right` already did.
Multi-line buffers (fenced code) absorb the move internally first.

### Added

- **`outl_core::Tree::properties_of(node)`** — iterator over every property currently set on a node, in one pass.
  Used by the outline DTO so each `OutlineNode` carries its own properties without scanning the workspace-wide map per block.
- **`outl_md::view::line_col_to_char(s, line, col)`** — inverse of the existing `char_to_line_col`.
  Vim-style column clamping (past EOL → end of line) and line clamping (past last → end of string).
  Lets `outl_tui::EditBuffer::move_up` / `move_down` wrap the same primitives the renderer (`block_to_rows`) already uses.
- **`outl_tui::EditBuffer::move_up` / `move_down` / `visual_column`** — three thin wrappers over `outl_md::view::char_to_line_col` + `line_col_to_char`.
  Cross-block Up/Down in Insert calls these first; only spills into the next block when the cursor was already on the buffer's first/last line.
- **`outl_actions::project_outline_node(workspace, node)`** — build a single `OutlineNode` (subtree + properties) from the workspace.
  Used by the backlinks builder so each backlink carries its source block as a self-contained outline.
- **`outl_actions::flatten_subtree_paths(node)`** — DFS-ordered paths inside an `OutlineNode` subtree.
  Moved here from `outl_md::outline_ops` so any client that consumes `Backlink::source_block` can navigate it.
- **`outl_actions::OutlineNode.properties`** — `(key, value)` pairs in alphabetical order.
  Workspace and disk paths both normalise to the same order so backlink panels and outline pages never disagree visually.
- **`outl_actions::PageMeta.icon`** — page-level `icon::` property surfaced on the meta.
  Clients pick their own fallback (mobile uses `📄`/`📅` by `kind`; TUI uses `📄`).

### Changed

- **Backlinks now route through `outl_actions::backlinks_for_page` only.** `outl_md::index::Backlink`, `WorkspaceIndex.backlinks()`, `refresh_backlinks_from_source`, `patch_backlink_text`, `flatten_backlink_subtree` were deleted.
  The `outl-md` index still owns page metadata and the block-level index; only the parallel backlinks cache went away.
- **`outl_actions::Backlink` is the rich struct.** Now carries `source_block: OutlineNode` (subtree + properties), `source_block_path: Vec<usize>`, `source_path: Option<PathBuf>` alongside `block_id`, `block_text` (TODO/DONE prefix stripped), `todo`, `source_page`.
  Mobile renders just `block_text` + `todo` today and ignores the rest; the TUI uses the full subtree to render its mini-outline in the backlinks panel.
- **`outl_actions::backlinks_for_page(workspace, root, meta)` / `backlinks_for_target(workspace, root, target)`** now take the workspace root so each backlink can carry its source `.md` path.
  CLI passes `&ctx.root`, TUI passes `&self.workspace_root`, mobile passes `storage_root`.
- **TUI cross-block Up/Down in Insert.** Commits the current buffer, moves the outline selection, re-enters Insert with the cursor on the preserved column.
  Guard: when `move_selection` would land `Focus` on the backlinks panel, the TUI stops in Normal mode instead of opening a different page mid-Insert.
  Backlink edits keep the older Esc → j/k → i workflow until cross-page commits get their own pass.
- **`App::backlinks_for_current` is cached.** Per-frame and per-keystroke render calls hit a `RefCell<Option<(slug, Vec)>>` cache; invalidated on `save`, `save_page_with`, `reload_workspace_from_disk`, and any view switch.
  Cuts the workspace scan from `O(blocks)` per call to one per slug change.
- **Self-references are kept in backlinks.** The "skip self-references as noise" heuristic on `outl_md::index` was dropped — a block on today's journal that mentions `[[2026-06-02]]` is exactly the "linked from" pin the user expects to see when revisiting that day.

### Refactored

- **`crates/outl-core/src/tree.rs` (854 lines) → `crates/outl-core/src/tree/{mod, cycle, op, apply}`** — `Tree::creates_cycle` in `cycle.rs`, `Tree::do_op` + `Tree::undo_op` in `op.rs`, `Tree::apply_op` in `apply.rs`.
  Struct and accessors stay in `mod.rs`.
  The 11 inline CRDT tests moved to `crates/outl-core/tests/tree_unit.rs`.
  **Algorithm semantics unchanged** — verified line-by-line against Kleppmann et al. 2022 and against the full invariant battery (convergence, cycle, cycle_chain, concurrent_edit_move, concurrent_delete_edit, late_op, idempotency, fractional_index, property_based, large_log: 32/32 green).
- **`crates/outl-tui/src/input.rs` (835 lines) → `crates/outl-tui/src/input/{mod, normal, insert, overlay, visual}`** — one handler per file, shared helpers (`cross_block_step`, `cursor_inside_open_fence`, `cross_block_nav_eligible`) stay in `mod.rs`.
- **`crates/outl-tui/src/actions/block.rs` (843 lines) → `crates/outl-tui/src/actions/block/{mod, insert, structural, backlink_edit, metadata}`** — Insert mode in `insert.rs`, create/indent/outdent/delete/move in `structural.rs`, cross-page backlink ops in `backlink_edit.rs`, properties + TODO toggle + pin in `metadata.rs`.
  TODO-prefix cycle helpers shared via `mod.rs`.
- **`crates/outl-tui/src/actions/lifecycle.rs` (669 lines) → `crates/outl-tui/src/actions/lifecycle/{mod, index_build, peer_sync, external, loading, persistence}`** — `App::new` and the shared `file_mtime` helper in `mod.rs`.
  Each submodule owns one concern.

No public API changed during the splits.
Clients (mobile, CLI, external consumers) need no update.

### Documentation

- **Anti-duplication policy** added to the root `CLAUDE.md` and echoed in every per-crate `CLAUDE.md`.
  Captures the lesson surfaced by the parallel `Backlink` structs and the near-miss with `line_start_and_column` (almost re-derived inside `EditBuffer` before the inverse `line_col_to_char` landed in `outl-md::view`).
  Rule: grep upstream first, prefer evolving the existing API over cloning the math.

### Internal

- `outl_md::Backlink`, `WorkspaceIndex.backlinks`, `refresh_backlinks_from_source`, `patch_backlink_text`, `flatten_backlink_subtree`, `outl_md::index::Backlink` removed.
- `outl_md::view` gained the `line_col_to_char` inverse.
- `outl_core::Tree.{nodes, properties, collapsed}` are now `pub(super)` so the split submodules can reach them.
  Public API unchanged.

## [0.5.1] — 2026-06-01

**Fix: multi-process writes against the same workspace.**

0.5.0 inherited an exclusive `flock` on `<root>/.outl/.lock` from the SQLite era.
The lock made sense when two writers on a single `log.db` would race, but JSONL stores one file per actor — the exclusive scope just blocked every legitimate co-tenant: TUI + MCP server, MCP server + `sink-outl` plugin, two CLI calls in flight.
Symptom: `INVALID_ARG: workspace ... is locked by another outl process` from the second opener, while the first ran fine and held the lock for its whole session.

0.5.1 splits coordination into two locks.
**Concurrent TUI + MCP server + CLI subprocess is the supported case** from here on.

### Added

- **`outl_core::WorkspaceLock` is now shared** (`LOCK_SH`).
  Every well-behaved `outl` process piles on.
  The lock still surfaces a hard filesystem error when `flock` itself fails, but never rejects a legitimate second opener.
- **`outl_core::ActorWriteLock`** — exclusive `flock` on `<root>/ops/.lock-<actor>`.
  Held by exactly one process per actor id at a time.
  This is the new write-coordination boundary.
- **`outl_core::resolve_write_actor(ops_dir, config_actor)`** — helper used by every workspace opener.
  Tries `config_actor` first; on `AlreadyHeld`, generates `ActorId::new()` and locks the ephemeral one instead.
  Returns the lock + actor id pair.
- **`WsCtx.ephemeral_actor: bool`** flag on the CLI/MCP context so `outl doctor` / `outl workspace info` can show when a process is writing under an ephemeral actor.

### Changed

- **`outl-cli::ws::open`** acquires the shared workspace lock plus a per-actor write lock through `resolve_write_actor`.
  On `outl` invocations that land while a server/TUI already holds the config actor, this process spins a fresh `ops-<ephemeral>.jsonl` and writes there.
  Readers merge every `ops-*.jsonl` in `ops/`, so peers see the full op log.
- **`outl-tui::open_workspace`** follows the same flow.
  The TUI used to refuse to launch when an MCP server was running against the same workspace; it now coexists.

### Why the ephemeral-actor fallback is safe

Every actor is independent at the CRDT layer (it's literally the mechanism multi-device sync relies on).
Two processes on the same device using two different actors merge the same way two devices would: the readers replay every `ops-<actor>.jsonl` in HLC order, the tree converges.
The only cost is `ops/` accumulating one jsonl per ephemeral lifetime — typically tiny files (a session's writes), and a future `outl gc` can consolidate them per device.

### Migration

None. 0.5.0 workspaces work as-is.
The next time you open a workspace with a second `outl` process, it will silently mint an ephemeral actor; the first process keeps writing under `config.toml[workspace].actor_id` as before.

## [0.5.0] — 2026-06-01

**Breaking: SQLite is gone.
JSONL is the only persistent storage.**

0.4.x kept two storage backends side by side — `SqliteStorage` for local-only workspaces and `JsonlStorage` for shared/synced ones.
The result was a class of "writes go through but disappear when you open the other client" bugs: any code path that opened a workspace via `outl-cli` got SQLite, while `outl-tui` and mobile (Tauri) followed `config.toml[workspace].storage` and got JSONL.
Same workspace, divergent op logs, silent loss.

0.5.0 collapses the surface: every client opens the workspace as `JsonlStorage` rooted at `<root>/ops/`.
There is no flag to choose, no `[workspace].storage` knob with two valid values, no SQLite fallback.
The `Storage` trait stays in place for future backends (ChronDB on the roadmap); the only impl that ships is JSONL plus the in-memory test double.

### Migration from 0.4.x

If your workspace was created with 0.4.x and you have data in `<root>/.outl/log.db`, the migration is a strict three-step sequence. 0.5.x cannot read SQLite and 0.4.1 is the last release that shipped `outl migrate-to-shared` (which this PR removed):

```bash
# 1. Pin 0.4.1 (last release with migrate-to-shared)
cargo install outl-cli --version 0.4.1 --locked

# 2. Run the one-shot migration (idempotent, leaves log.db intact)
outl migrate-to-shared <workspace>

# 3. Confirm ops/ops-<actor>.jsonl grew, then upgrade
cargo install outl-cli --version 0.5.1 --locked

# 4. Once you've verified peers see your data, delete log.db yourself
rm <workspace>/.outl/log.db <workspace>/.outl/log.db-shm <workspace>/.outl/log.db-wal
```

If you already had a mixed `log.db + ops/` workspace under 0.4.x, step 2 is still required — `migrate-to-shared` is idempotent (HLC dedup) and any ops that only ever made it into SQLite move over on this run.
After step 3, 0.5.x ignores `log.db` entirely.

### Removed

- **`SqliteStorage`** in `outl-core::storage`.
  Callers use `JsonlStorage` (persistent, per-actor JSONL) or `MemoryStorage` (the new in-memory test double, replaces `SqliteStorage::open_in_memory`).
- **`rusqlite` dependency.** Workspace `Cargo.toml` no longer pulls the SQLite C bundle.
  Faster builds, smaller binaries.
- **`outl migrate-to-shared`** subcommand.
  It only made sense while both backends coexisted; with only one backend the migration is a one-shot done on 0.4.1 before upgrading.
- **`config.toml[workspace].storage`** field is silently ignored now (kept readable so old configs don't error).
  Cleaning it up is fine but not required.

### Changed

- **`Paths` struct (`outl-cli/src/workspace_layout.rs`)** drops the `db: PathBuf` field, gains `ops: PathBuf` pointing at `<root>/ops/`.
  Every caller that touched `.outl/log.db` now targets the JSONL directory.
- **`outl init`** scaffolds `<root>/ops/` and opens `JsonlStorage` to materialize the per-actor `ops-<actor>.jsonl` file.
  The human output now reports `ops:` instead of `log:`.
- **`outl doctor`** drops the SQLite `PRAGMA integrity_check` finding and replaces it with a JSONL parse-and-load check (`JsonlStorage::open` surfaces every unreadable line via `tracing::warn!`, then the report carries the op count and the set of known node ids the sidecar cross-check needs).
- **`outl workspace info --json`** renames the `log_db` field to `ops_dir`.
  Stable-envelope shape otherwise unchanged.
- **`outl-tui::open_storage`** is now a one-liner.
  The config-driven match disappears; storage is always JSONL.
- **`Workspace::open_in_memory`** is unchanged in signature but uses the new `MemoryStorage` under the hood.
  No filesystem touch.

### Internal

- New `MemoryStorage` in `outl-core::storage::memory`.
  Pure `Vec<LogOp>` + snapshot slot, no I/O.
  Used by every test that previously called `SqliteStorage::open_in_memory()` and by `Workspace::open_in_memory`.

## [0.4.1] — 2026-06-01

Batch authoring for agents and scripts.
The 0.4.0 CLI / MCP surface covered every primitive write, but creating a structured page meant chaining N tool calls — one per block — which costs round-trips, turn budget on the agent, and time. 0.4.1 collapses that into the three composite shapes an agent or import pipeline actually wants: write a subtree, create a page with content, and stream a sequence of writes in one workspace session.

No storage or op-log format changes — every new tool is shimmed over the existing `outl-actions` primitives (`append_block`, `edit_text`, `set_property`).
Drop-in upgrade from 0.4.0.

### Added — composite write surface

- **`outl_block_append_tree` / `outl block append-tree`.** Append a root block plus its recursive children under a page or block in a single op-log session.
  Input shape: `{"text": "...", "children": [{"text": "...", "children": [...]}]}`.
  Response mirrors the input with `id` at every node so the caller can map specs back to freshly minted ids.
  CLI accepts the JSON inline (`--tree '{...}'`) or via stdin (`--tree -`).
- **`outl page create --content` / `outl_page_create` with `content`.** A new page lands with its full outline forest in one call instead of `page_create` + N × `block_append`.
  Accepts either a single root (`{text, children?}`) or a forest (`[{...}, {...}]`); the returned `content` array carries the block ids.
  Skipping the field keeps the historical empty-page behaviour.
- **`outl batch` / `outl_batch`.** Apply a list of writes sequentially in one workspace session.
  Supported `op` names: `page_create`, `page_update`, `page_delete`, `page_rename`, `block_append`, `block_append_tree`, `block_insert`, `block_update`, `block_move`, `block_delete`, `block_toggle_todo`, `daily_append`, `page_prop_set`.
  Each op's `args` mirror the matching standalone tool.
  **Stop-on-first-error semantics:** earlier ops stay in the op log (they're already CRDT ops; we don't roll them back), and the response carries `failed_at` / `failed_op` / `error` so the caller can recover or retry only the suffix that never ran.
  CLI exit code is `1` on partial failure.

### Added — `outl-actions::block`

- **`append_tree`, `append_forest`.** UI-agnostic primitives behind the new composite tools.
  `BlockTreeSpec` + `BlockTreeOutcome` are the shared DTOs (serde Serialize / Deserialize) so both client layers and future plugins can compose subtrees without re-deriving the recursion.

### Added — bench

- **`bench-cli-xlarge` workflow job.** Weekly + dispatch only.
  Generates a 10k-page batch payload via the new `xtask gen-10k` binary, applies it through `outl batch` end-to-end (subprocess + workspace lock + op log + sqlite + sidecar + md write), then runs `hyperfine` on `page list`, `search`, `query --tag`, `page get`, and `page render` against the populated workspace.
  Catches regressions in the surface that wraps the algorithm — the existing `bench-xlarge` job stays focused on the algorithm itself via criterion micro-benches.
- **`xtask` workspace member.** Internal task runner; today ships `gen-10k` (deterministic batch-payload generator) and is where any future codegen / fixture / bench helper lands.

### Docs

- `docs/cli.md` — new **Batch** section with the payload shape and failure semantics; `page create --content` and `block append-tree` documented inline next to the existing primitives.
- `docs/mcp.md` — multi-block authoring callout pointing at the three new composite tools.

## [0.4.0] — 2026-06-01

outl becomes scriptable.
A full machine-shaped CLI (page, block, daily, search, query, tag, prop, backlinks, export, workspace) lands with a stable JSON envelope and exit codes, and the same handlers are exposed over MCP via `outl mcp serve` (JSON-RPC over stdio) so Claude Desktop, Cursor, and any other agentic client can drive a workspace without parsing TUI output.
Business logic stays in `outl-actions`; the CLI and MCP are thin shims over the same code.

No storage or op-log format changes — drop-in upgrade from 0.3.x for data on disk.
**One breaking flag rename** for shell/cron users: `--path` is now `--workspace` everywhere.

### CLI (`outl-cli`) — new machine surface

- **Subcommands cover the full workspace API.** `outl page {list,get,create,rename,delete,prop}`, `outl block {get,edit,create,delete,move,toggle}`, `outl daily {today,get,range}`, `outl search`, `outl query`, `outl tag {list,page}`, `outl prop {list,page}`, `outl backlinks {page,block,embed}`, `outl export hugo`, `outl workspace {info,doctor}`.
  Every command writes a stable JSON envelope (`{ok, data, error, meta}`) to stdout and a typed exit code, so scripts and CI never have to scrape human output.
  `--human` keeps the friendly table format for interactive use.
- **One Workspace per process, index cached.** Each invocation opens the workspace once, reuses the in-memory index, and drops the per-call SQLite replay that older `outl serve`-style flows paid.
- **`--workspace` replaces `--path`.** The TUI, server, doctor, and every new subcommand now take `--workspace <dir>`.
  Existing scripts that pass `--path` must rename the flag (env var stays `OUTL_WORKSPACE`).
  The TUI's positional path argument is unchanged for direct double-clicks.
- **CLI integration suite** (`cli_machine.rs`) exercises page, block, search, and workspace commands against a real workspace so envelope shape and exit codes can't drift.

### MCP server (new: `outl mcp serve`)

- **JSON-RPC over stdio.** `outl mcp serve --workspace <dir>` speaks the MCP protocol with `initialize`, `tools/list`, `tools/call`, `resources/list`, `resources/read`, `prompts/list`, and `prompts/get`.
  Drop the binary into Claude Desktop's `claude_desktop_config.json` or Cursor's `mcp.json` and the agent can read journals, search, follow backlinks, edit blocks, and toggle TODOs against the same workspace your TUI/mobile is using.
- **Tools** mirror the CLI 1:1 (`outl_page_*`, `outl_block_*`, `outl_daily_*`, `outl_search`, `outl_query`, `outl_tag_*`, `outl_prop_*`, `outl_backlinks_*`, `outl_workspace_*`) so the LLM sees the same surface a human would script.
- **Resources** expose read-only views over `outl://daily/today`, `outl://page/<slug>`, `outl://search?q=…`, etc., for clients that prefer URI-addressed reads to tool calls.
- **Prompts** ship `summarize_day` and friends so the agent can pull a daily-note summary in one round-trip.
- **Per-session workspace + cached index.** The MCP server holds one `WsCtx` for the life of the session and routes every read through `ServerCtx::with_workspace`, which reuses that handle and invalidates the index after lazy journal materialisation in `outl://daily/today` and `summarize_day`.
  Earlier prototypes opened a fresh `WsCtx` per call and self-deadlocked on the workspace lock the session already owned — `resources/read` and `prompts/get` are now part of the same cached path as `tools/call`.
- **MCP smoke suite** (`mcp_smoke.rs`) walks `initialize` → `tools/list` → `tools/call` → `resources/read` in one session so the lock-reuse contract can't regress.

### Security / hardening

- **Slug validation at the boundary.** `outl-actions::is_valid_slug` rejects empties, `.`/`..` traversal, path separators, and control chars before any filesystem write, surfaced as a typed `ActionError::InvalidSlug` (`INVALID_ARG` in the CLI/MCP envelope).
  Hugo export adds a second `target_within` check against canonicalised paths so a legacy bad slug imported from disk still cannot escape `--out`.
- **Doctor split.** `workspace doctor` runs `collect_json` (full lock probe, used by `outl doctor` from the shell) and `collect_in_session_json` (probe off, used by the MCP tool which already owns the lock).
  Before this split, `outl_workspace_doctor` always warned about the workspace lock on perfectly healthy workspaces.
- **Quieter failures stop being silent.** Page delete/rename replace `let _ = remove_file(...)` with a `remove_or_warn` helper so a broken filesystem surfaces in logs instead of disappearing.
  Regression tests cover malicious slugs, doctor-clean inside an MCP session, and delete being idempotent when the `.md` is already gone.

### Docs

- New `docs/cli.md` and `docs/mcp.md` cover the machine surface and the MCP wiring for Claude Desktop / Cursor end to end (envelope shape, every subcommand, every tool, every resource).
- Getting-started, tutorial, sync, theming, TUI, and clients docs refreshed for the `--workspace` rename and the new subcommand names.

## [0.3.1] — 2026-05-31

Mobile UX polish + autocomplete fixes.
No protocol or storage changes — drop-in upgrade from 0.3.0.

### Mobile (`outl-mobile`)

- **Autocomplete (`[[…]]`) now actually fires on iOS.** The native ref suggester chip strip was orphaned — `createEffect` was being registered after an `await` inside `onMount`, which lost Solid's reactive owner.
  State was published once at boot and never updated as the user typed.
- **TODO/DONE prefix is visible (and editable) in Insert mode.** Tapping a TODO block used to show only the checkbox + body (`ship it`) with the `TODO` prefix hidden, so erasing the prefix from the editor was impossible.
  Now the prefix appears in the textarea (`TODO ship it`) and the checkbox flips to a bullet while editing — toggling state via the text Just Works.
- **Cursor lands inside `[[ ]]` / `(( ))` reliably.** `el.value = …` resets the textarea caret in iOS WKWebView; combined with Solid's `value={draft()}` rebinding the caret could end up outside the pair.
  Replaced with `setRangeText` + double `parkCaret` (sync + microtask) so every toolbar insert, paste completion, and suggester pick parks the caret where the user expects it.
- **Backspace inside empty `[[]]` / `(())` collapses the pair.** No more mashing backspace four times to undo an aborted ref.
  Same rule on TUI and mobile.
- **Smart Punctuation is OFF.** `--` no longer becomes `–`, `...` no longer becomes `…`, quotes stay straight.
  Code snippets and CLI commands in journals survive intact.
- **Toast pattern for errors** (auto-dismiss + Retry button) in place of the persistent red `<p>` that sat in the middle of the outline forever.
  Failed saves now offer a one-tap retry without losing the draft.
- **`commitInFlight` lock + 8 s timeout** serializes concurrent block edits (typing → TODO toggle → blur) so the older save never overwrites the newer, and a stuck Tauri command can't freeze Insert mode indefinitely.
- **Progressive loading message** ("Loading…" → "Connecting to iCloud…" → "Still waiting on iCloud…") + spinner + a Retry button on terminal failure. iCloud cold-start no longer reads as "the app froze".
- **Connectivity-aware SyncDot** uses `navigator.onLine` + `online`/`offline` listeners to actually show the offline pip (was dead code before).
  `aria-label` instead of `title` so iOS WKWebView users get the status verbally.
- **Tap targets meet Apple HIG** (~30 × 30 hit area on the bullet/checkbox; bullet is now actually tappable).
  `[[ref]]` and `#tag` taps navigate instead of opening the editor.
- **Long-press TODO** uses a distinct success haptic when creating a new TODO vs. cycling an existing one.
- **`SwipeRow` × `SwipeNavigator` conflict resolved** — swipe-right on the left edge no longer races the per-row swipe-delete (each one captures only its own direction).
- **`PageSwitcher`** opens the first match on `Enter`, dismisses on `Escape`, and supports swipe-down on the handle to dismiss (without stealing scroll from the list).
- **Backlinks empty state** so the bidirectional-linking concept is discoverable on freshly-created pages.
- **Performance** in long outlines: `draft()` is now a lazy getter prop only read by the block being edited (was triggering a reactive effect in every BlockRow per keystroke).
  Auto-resize coalesced into a single `requestAnimationFrame`.

### Shared (`outl-actions`)

- `edit_text` writes its argument **verbatim** instead of preserving a leading `TODO`/`DONE` prefix automatically.
  Callers that surface state separately (mobile checkbox) reattach the prefix themselves — required so erasing the prefix in the editor actually sticks.
  TUI path is unaffected (it always passes the raw block text through reconcile).

### TUI (`outl-tui`)

- Backspace inside an empty `[[]]` / `(())` now collapses both brackets in one keystroke (matches the mobile behaviour).

## [0.3.0] — 2026-05-30

Cross-device sync goes live.
A brand-new iOS app and the TUI share the same workspace over iCloud Drive, both driven by a shared `outl-actions` crate.
Block refs / embeds land in the markdown dialect.

### Mobile (`outl-mobile`) — new crate

- **Tauri 2 + SolidJS iOS client.** Bundle id `app.outl.mobile-app`, iCloud container `iCloud.app.outl.mobile-app`.
  Frontend is Solid + Tailwind; the Rust shell is intentionally thin (every workspace operation delegates to `outl-actions`).
- **iCloud Drive transport.** Workspace lives at `<ubiquity-container>/Documents/`.
  An ObjC bridge (`gen/apple/.../main.mm`) uses `NSMetadataQuery` to watch for peer changes and `NSFileCoordinator` + `startDownloadingUbiquitousItemAtURL` to force materialisation before reads — without those two steps the Rust side races the iCloud daemon and sees truncated op logs.
- **Per-device actor id** persisted under the app sandbox so each install writes to its own `ops-<actor>.jsonl`.
- iOS boot flash fixed; outl brand (palette + icon) applied across the app.

### Shared client (`outl-actions`) — new crate

- **Extracted every workspace mutation** (block edit, TODO toggle, indent / outdent, sibling create, delete, move, journal render, sync) out of `outl-tui` into a UI-agnostic crate.
  Functions take `&mut Workspace` + `&HlcGenerator` and route through `Workspace::apply` so the op log stays source of truth.
- TUI and mobile call the **same** functions for the same semantics — drift between clients is no longer possible by construction.
- `SyncEngine` interface owns the cross-device merge loop; iCloud is the v0 transport, iroh (phase 2) plugs in behind the same trait.

### Core (`outl-core`)

- **`JsonlStorage` op-log backend.** Single-file SQLite breaks under iCloud / Syncthing because the FS layer is last-write-wins per file.
  JSONL gives each actor its own `ops-<actor>.jsonl`, writes only to the local file, and merges every peer file on read.
- Folder layout is **`ops/`**, not `.ops/`. iCloud Documents skips dotted paths during cross-device sync — using a dot silently breaks multi-device workspaces.
  Same rule applied to the sidecar (`pages/<slug>.outl`, no leading dot).

### Markdown (`outl-md`)

- **`((blk-X))` inline refs and `!((blk-X))` embeds.** Stable `ref_handle` derived from the block's ULID tail (lazy 7+ char expansion on collision); sidecar bumped to v2.
  Embeds expand to the source root + children with a `↳` marker.
- Concurrent-safe writes over iCloud (atomic temp-file rename, no partial reads exposed to peers).
- `WorkspaceIndex` tracks block-ref backlinks alongside page-ref backlinks.

### TUI (`outl-tui`)

- Rebuilt as a **peer of shared workspaces** — same iCloud folder, same op log, same `outl-actions`.
  Edits on the laptop appear on the iPhone within seconds and vice versa.
- `((` autocomplete on block text, inline ref render, expanded embed view, Enter navigation to the source block, `/refer` and `/refer-embed` slash commands.
- `yr` chord copies the block's ref handle to the OS clipboard via arboard.
- outl brand (palette, icon, chrome) applied; mobile and TUI now look like the same product.

### CLI (`outl-cli`)

- **`outl migrate-to-shared` subcommand** rewrites a legacy SQLite workspace into the JSONL + sidecar layout consumed by both clients.
- `outl doctor` flags orphan `((blk-X))` and `!((blk-X))` handles.

### CI / release

- Release workflow rewritten as `prepare → tag → create_release (draft) → build matrix → publish_release`.
  Single `gh release create --draft` before the matrix and `gh release upload --clobber` per matrix leg, so paralleled jobs don't race each other on a repo with Immutable Releases turned on.
- macOS Intel binary now cross-compiles from `macos-latest` (ARM) instead of relying on the depleted `macos-13` runner pool.
- `outl-mobile` excluded from Linux CI jobs (Tauri iOS toolchain is macOS-only).

## [0.2.0] — 2026-05-26

Backlinks become a first-class part of the TUI: they live inline below the outline (no more side panel), render the referencing block with its children, and are fully editable in place.

### TUI (`outl-tui`)

- **Inline backlinks.** Replace the right-side panel with a section rendered below the outline, separated by a full-width `─` rule.
  Each source page shows up grouped under an icon + title header.
- **Full source block + children.** Backlinks render the referencing `OutlineNode` _with its subtree_ (not a truncated snippet), so you see context without jumping to the source page.
- **Cursor navigation crosses the boundary.** `j`/`k` flow transparently between outline and backlinks.
  `app.focus: Focus::{Outline, Backlink{idx, sub_path}}` tracks where the cursor lives.
- **In-place edits land on the source `.md`.** `i`/`I`/`a`/`Esc`, `Ctrl+T` (TODO/DONE cycle), `o`/`O` (sibling create), `Tab`/`Shift+Tab` (indent/outdent), `dd` (delete), `K`/`J` (move up/down) — all work on a backlink the same way they work on the outline, persisting straight to the source page via `EditTarget::SourcePage`.
- **Optimistic index updates for snappy UX.** Edits patch the in-memory `WorkspaceIndex` immediately (next frame shows the new state), then save without scheduling a full workspace rebuild on the hot path.
- Cursor column preserved when entering Insert (`i` honors vim semantics; `I` still jumps home).
- Ghost cursor on the last outline block when focus had moved into the backlinks section is gone (`render_block` gates by `Focus::Outline`).
- `view.rs` split into `view/{inline, outline, overlays, backlinks}.rs` by responsibility — each file under 450 lines.

### Markdown (`outl-md`)

- `Backlink` carries the full `source_block: OutlineNode` and its `source_block_path` (DFS path in the source AST) instead of a flat index plus truncated snippet.
  Repeated refs to the same target inside one block collapse to a single backlink.
- `WorkspaceIndex::refresh_backlinks_from_source(path, &page)` — optimistic patch of every cached `source_block` for backlinks pointing at `path`.
  Used by the TUI's cross-page edit path.
- `WorkspaceIndex::patch_backlink_text(path, target_path, &new_text)` for text-only optimistic edits.

## [0.1.0] — 2026-05-25

First public release.
Single-device editor; sync transport is on the roadmap but the algorithm and op-log infrastructure are already in.

### Core (`outl-core`)

- Tree CRDT implementation following Kleppmann et al. 2022 (`do_op` / `undo_op` / `apply_op` / `creates_cycle`).
- HLC timestamps with actor tiebreak.
- Append-only op log with sqlite backend (`SqliteStorage`).
- `Storage` trait so alternative backends (e.g.
  ChronDB) can slot in without touching the CRDT.
- Workspace file lock via `fs2::flock` — two `outl` processes on the same workspace get a clean error, not a race.
- Property-based test of strong eventual consistency over 100+ randomised op permutations.

### Markdown / sidecar (`outl-md`)

- CommonMark parse + render with the outl dialect (`title::`, `icon::`, page/block properties, `[[refs]]`, `#tags`, `((block-id))`, fenced code blocks, multi-line block text).
- `.foo.outl` JSON sidecar holding the IDs the `.md` deliberately doesn't carry.
  **The `.md` stays clean** — no `id::`, no UUIDs.
- 3-level matching algorithm (`outl-md::matching`) reconstructs which block kept which ID after an external editor saves the file.
- Workspace index (`WorkspaceIndex`) — title, icon, slug, backlinks, tag namespace; powers the switcher, autocomplete and backlinks panel.
  Built once on boot, refreshed in a worker thread on save.
- Roundtrip property test: `parse(render(ast)) == ast` over randomly generated outlines including multi-line and fenced cases.

### Code-block execution (`outl-exec`)

- `Runtime` trait + `RuntimeRegistry`.
  Shipped runtimes (each behind a Cargo feature for opt-out distributions):
  - `lisp` — Steel (Scheme R5RS-ish in pure Rust).
  - `js` — Boa (ES2015+ in pure Rust).
  - `python` — RustPython (Python 3 subset).
  - `lua` — mlua 5.4 (vendored).
  - `rust` — `rustc → wasm32-wasip1 → wasmtime`.
    Compiled artefacts cached in `~/.cache/outl/runtimes/rust/<hash>.wasm`. ~20× faster on a re-run of the same snippet.
- WASM sandbox infrastructure (wasmtime engine + WASI ctx with no preopens / no env / no sockets, fuel-based instruction cap, epoch-interruption timeout, in-memory stdin/stdout/stderr).
- Idempotent result subblock — re-running the same code overwrites the existing `> **result:**` child instead of duplicating it.
- `source-hash::` stamped on each result child so the upcoming auto-run loop can short-circuit unchanged sources.

### TUI (`outl-tui`)

- Journal-first: opens on today's date.
- Vim-style modes (Normal / Insert / Visual) with chord support (`dd`, `gg`, `gx`, `yy`, `qq`-to-quit).
- Insert mode autocomplete for `[[refs]]`, `#tags`, and `/commands` (Notion-style slash menu).
- Slash command system + vim palette share one registry — every built-in command shows up in both.
  Built-ins: `prop-block`, `prop-page`, `search`, `run`, `theme`, `today`, `open`, `refresh`, `write`, `quit`, `help`.
  The registry is the plugin-extension point.
- `gx` runs the code block under the cursor through `outl-exec`.
- `auto-run::` property runs a block automatically on page open (cache-aware via SHA-256 of the source).
- `icon::` page property surfaces in every place the title shows (header, switcher, backlinks panel, search results, autocomplete, inline `[[refs]]`).
- Multi-line blocks via `Alt+Enter` / `Ctrl+J` / `Shift+Enter` (Shift+Enter only on terminals that speak the kitty keyboard protocol); plain `Enter` auto-detects an open code fence and inserts a soft newline inside it.
- Vertical scroll with `PgUp`/`PgDn`/`Ctrl+D`/`Ctrl+U`/`gg`/`G` and auto-scroll when the selection moves off-screen.
- Hot reload on external `.md` edits (polls mtime every 750ms; warns instead of clobbering when you're mid-Insert).
- Error modal overlay for multi-line failures (rustc compile errors, traps, missing toolchain), keeping the status line for short successes.
- Themes: 11 presets, switchable with `/theme <name>` at runtime.

### CLI (`outl-cli`)

- `outl` (no subcommand) opens the TUI in `$PWD`.
- `outl init <path>` scaffolds a workspace.
- `outl serve [--once]` reconciles `.md` files into the op log (one-shot or watch mode).
- `outl import logseq <src> <dst>` and `outl import roam <backup.json> <dst>` strip `id::` lines, slugify, seed sidecars.
- `outl doctor` and `outl reconcile` placeholders for the integrity and orphan-resolution flows.

### Tooling / DX

- Workspace MSRV: rustc 1.88.
- CI: `fmt` + `clippy -D warnings` + `cargo test --workspace --all-targets` on Linux and macOS.
- Bench CI: `small` / `medium` / `large` on every PR + push; `xlarge` (10k+ files) on weekly cron + manual dispatch.
- File-size guard hook (`.claude/hooks/file-size-guard.sh`) blocks Rust files past ~900 LOC unless the change is intentional — forces a refactor conversation before drift accumulates.
- Background workspace-index build: `App::new` paints the journal immediately and spawns a worker thread for the global index; backlinks/icons fill in within ~ms of boot.

### License

MIT.

[0.1.0]: https://github.com/avelino/outl/releases/tag/v0.1.0
