# outl doctor

The integrity check you run before trusting a migration, and after any sync weirdness.
It is also the only writing mode the CLI has that is not an explicit mutation.
Split out of [cli.md](cli.md) because it is the one subcommand whose *refusals* need as much space as its behaviour: what `--repair` declines to touch is the part that keeps it from deleting your work.

> Why `--repair` refuses a page whose `.md` holds content the op log lacks: [RFC 0210](rfcs/0210-md-content-outside-op-log.md).
> Where the device store lives and why it is outside the workspace: [storage.md](storage.md#where-the-actor-id-lives--outside-the-workspace).

**Read-only by default** — it reports, it never fixes, unless you pass `--repair`.
Exit code is `1` when the report carries any error, so it drops straight into a script or CI step.

"Read-only" is literal, and it is asserted per directory, not per file:
a run without `--repair` leaves `ops/` **byte-identical** (including the `.ops-<actor>.idx` boot caches, which the storage layer would otherwise rebuild and persist just by being opened) and appends nothing to `.outl/orphans.log`.
The only thing a default run writes is its own stdout.

## What it checks

**Op log — the source of truth.**

- Every `ops-*.jsonl` is swept **line by line**, and each defective record is named with its **line number, byte offset, and reason**.
  Three reasons exist: invalid JSON, non-UTF8 bytes (a partial file sync can leave them mid-file), or two ops glued onto one line by an unsynchronized concurrent append.
  This matters because the boot path deliberately *skips* malformed records — one torn tail line must never lock you out of your workspace — and logs them only at `warn` level, where they scroll past.
  A glued line is a warning (every op is recovered); a line carrying no usable op is an **error**, because those mutations are gone.
- `.outl/snapshots/snap-<actor>.bin` is decoded and its content hash verified.
  A bad snapshot is a warning, not an error: it is a pure boot cache, so the only cost is that the next boot pays a full op-log replay.
  A snapshot that could not be **read** (permissions, a busy file, a half-arrived sync, a flaky disk) is reported separately and left exactly where it is — an unreadable file is not a proven-bad one, and `--repair` deletes for real.
- Each `.ops-<actor>.idx` offset index is cross-checked against its `.jsonl` — every offset must land on the first byte of a real record, and the index must not claim more ops than the file yields.

**Materialized tree — what the op log actually built.**

- **Trash contents.**
  Deletion is `Move(node, TRASH_ROOT)`, never a physical removal, so deleted blocks are still in the graph — invisible to every view.
  Doctor reports the total block count in the trash, how many top-level deletions produced it, and a text preview of each one.
- **Unmaterialized ops** — node ids the op log touches that never landed in the tree, i.e. `Edit` / `SetProp` / `SetCollapsed` whose effect you will never see.
- **Projection drift** — every page in the op log compared against its `.md` on disk: missing files, stale projections, missing sidecars.
- **`.md` content that never reached the op log** — a page whose sidecar agrees with the bytes on disk (so it looks like a merely *stale* projection) but which holds lines that exist in no op.
  Reported with the count and one of the lines, and **`--repair` leaves it alone**.
  Re-rendering the tree over it would delete that content and rebuild the sidecar from the same render, so nothing afterwards could tell the page had ever held more.
  Content in this state also does not sync to your other devices — peers exchange ops, not files.
  **`outl reconcile --ahead-of-log` is what brings it into the log.**
  Plain `outl reconcile` will not: the page is hash-faithful, so it reads as in-sync and the ordinary pass skips it.

  It clears the recorded hash on exactly the pages listed here and reconciles them, which emits ops for that content.
  Detection is shared with this check, so the two can never disagree about which pages qualify.
  This only helps while the content is still **on disk**.
  A page whose `.md` was already overwritten (before this guard existed) is unreachable this way.
  [`outl recover`](cli.md#outl-recover) reads the **op log** instead, where the same producer bug left the pre-truncation text as a recoverable earlier revision.

  **`outl reconcile --allow-bulk-delete`** is a different flag for a different state, and the two are worth keeping apart.
  A reconcile stops, writing nothing, when one page's `.md` would send more than **500 blocks** to the trash or more than **75%** of the blocks its sidecar knew (the share arm stands down under 20 blocks — clearing a scratch note is routine).
  The failure it guards is not a user deleting a section.
  It is a `.md` that arrived *wrong* — an iCloud placeholder whose bytes never downloaded, a half-flushed write — indistinguishable from a real bulk delete by shape, and only by scale.
  It does **not** clear any hash — it only turns the volume guard off.
  It also selects the opposite set from `--ahead-of-log`: that one visits pages holding *more* than the log, while a refused page holds *less*.
  Read what the `.md` actually holds before reaching for it — the guard fires precisely when the file, not the log, is the thing that is wrong.

**Files on disk.**

- `.md` ↔ `.outl` sidecar pairing, sidecar version, and sidecar block ids that are absent from the op log.
- Orphaned sidecars (a `.outl` with no `.md` next to it).
- Orphan `((blk-XXXXXX))` / `!((blk-XXXXXX))` references.
- **Sync-conflict copies** — `foo 2.md` (iCloud), `foo (conflicted copy).md` (iCloud/Dropbox), `foo.sync-conflict-….md` (Syncthing), across `pages/`, `journals/`, `ops/`, and `assets/`.
  Reported as **errors**: they are your content sitting outside the op log, and nothing in outl ever reads them.
  A `sprint 2.md` with no `sprint.md` next to it is a normal note and is not flagged.
- **Parser warnings** — every `.md` whose content stepped outside the outl dialect and got recovered by the permissive parser (typical case: a leading `# heading`, a free paragraph, imported markdown).
  A warning row goes into the doctor report, one per affected file.
  Under `--repair` only, one entry per warning is also appended to `.outl/orphans.log`, tagged `parse-warning <iso> <path>:<line> <kind> <raw>`, so the breadcrumb persists across runs.
  Rows are deduplicated on `<path>:<line> <kind>`: re-running never stacks a second copy of the same finding.
  That gate matters because `.outl/orphans.log` is also where **level-3 matching orphans** live: the record of blocks that could not be matched back into the op log.
  On a freshly imported graph the parse warnings outnumber them by orders of magnitude.
  Cleaning the offending lines (or saving the file from outl, which normalises to `- <raw>` on render) makes the warning disappear on the next run.

**Device store — the one thing checked outside this workspace.**

- **Actor bindings naming a workspace that no longer exists.**
  Reported as `info` on a plain run, with the count only; `--repair` is what drops them.
  The rule — and, more to the point, everything it refuses to touch — is under [The device store's stale actor bindings](#the-device-stores-stale-actor-bindings).

## `--repair`

Applies **only** the fixes that cannot lose data, and prints exactly what it will do before doing it (the same list a read-only run shows under `N repairable item(s)`).

| Fix | Why it is safe |
|-----|----------------|
| Re-project a page's `.md` + sidecar from the op log | The op log is the source of truth; a `.md` that disagrees with it is the wrong side. Requires a **healthy** op log (see below). Skipped when the file carries an unreconciled external edit — that is `outl reconcile`'s job. |
| Rebuild a missing sidecar | Only when the `.md` bytes already equal what the tree renders. Requires a healthy op log, same as above. Diverged content is never touched. |
| Delete a corrupt snapshot | Pure boot cache; rebuilt on the next boot. Only for a snapshot that was read end-to-end and then failed to decode — never one that could not be read. |
| Prune stale backup generations | Only `.outl/repair-backup/` directories that are **both** older than 14 days **and** outside the 10 most recent. |
| Drop a dead device-store actor binding | Only a binding whose workspace directory is **gone**, whose parent directory is still **present**, and which is older than **30 days** — all three (see below). The record is backed up first. |
| Delete an abandoned device-store scratch file | A write killed between composing its temp file and publishing it, untouched for over **24 hours**. Not backed up: it never became a record, and if it had, the record is already there. |

What it will **never** do: delete a `.md`, write a single byte into `ops/`, move a block to the trash, or pick a winner between two sync-conflict copies.

### The device store's stale actor bindings

This is the one check whose subject lives **outside** your workspace.

`~/.config/outl/actors/` (or `$OUTL_DEVICE_DIR/actors/`) holds one record per workspace this machine has ever opened: which actor id this device writes under there.
Nothing ever removed one, so a workspace you deleted last year still has its binding — on one development machine that reached 1,208 records, 1,166 of them pointing at directories that no longer exist.

The count is reported as **info**, never a warning: a stale binding is ~190 bytes and breaks nothing, because the workspace it names is gone.

Dropping one is not free, which is why the rule is strict.
The next open of that workspace mints a **fresh** actor — a second `ops-<actor>.jsonl` for a device that already had one — the fork this store exists to prevent.
So "the directory is missing" is not enough on its own: an unplugged drive, an unmounted network volume, and an iCloud folder not downloaded here all look identical to a deleted one.

Three conditions, all required:

1. The root is **gone** — not merely unreadable.
   A permission or I/O error keeps the binding.
2. The root's **parent directory is still there**, on the **filesystem the root was bound on**.
   Deleting a folder leaves its parent behind; an unmounted volume takes the whole path with it.
   A workspace that is *itself* a mount point would defeat the parent test alone (unmounting `/Volumes/Notes` leaves `/Volumes` behind), so each binding records which filesystem its root lived on, and a surviving parent on a different filesystem keeps the binding.
   A binding written before that stamp existed has nothing to compare and keeps the plain parent reading.
3. The record is **older than 30 days** — the age of the *record*, not time since the deletion.
   A binding is rewritten only when its workspace moves, so a graph you have had for years and deleted today is eligible right away.
   Condition 2 is what protects a live workspace.

A record whose `root=` does not survive a parse/write round trip (a path ending in a space, or holding a newline) is also kept: it names a path that was never written, and acting on it would drop a live binding.
The same goes for a root the store's text format could not spell faithfully in the first place, such as a path holding non-Unicode characters.

The verdict is re-checked immediately before the delete, so a drive plugged back in between the listing and the repair keeps its binding.
Each dropped record is copied to `.outl/repair-backup/<timestamp>/device-store/actors/` first; restoring is a `cp` back into `<device_dir>/actors/`.
`iroh/identity.key`, `machine-id` and `backups/` are never touched — `identity.key` **is** this device's node id.

A wrong drop is bounded: one extra ops file, and **no ops lost**, since every reader merges every `ops-*.jsonl` in the directory.

### A damaged op log suspends every page repair

Both of the page-level fixes above write a file **rendered from the materialized tree**, and the tree is only ever as complete as the op log it replayed.

That matters more than it sounds, because the boot path is deliberately forgiving: `JsonlStorage` skips records it cannot parse, so one torn line never locks you out of your workspace.
The cost is that a damaged log replays a **truncated** tree that looks perfectly healthy from the inside — nothing in it remembers what was skipped.
A `.md` that is still a faithful, complete projection of its page then compares as "stale" against that shorter render, and re-projecting it would overwrite your content with an incomplete one.

So when the doctor finds the log damaged, it **withholds** the page repairs instead of offering them, in `--repair` and in the read-only list alike, and says why.
Three things count as damaged:

- one or more `ops-*.jsonl` lines carrying no usable op,
- an `ops-*.jsonl` that could not be opened, or that hit an I/O error mid-scan,
- a sync-conflict copy under `ops/` (a forked op log — the ops in the fork never reached the tree).

Recover the log first — restore `ops/` from a backup, or let a healthy paired device sync it back — then re-run.
Corrupt-snapshot deletion is not withheld: it is a pure cache, and dropping it only forces the full replay a damaged log wants anyway.

### A large deletion is announced first, and needs `--force`

Re-projecting a page removes content whenever the op log says so — a paired device deleted a block, the log is right, this `.md` is behind.
That is normal and it is why the fix exists.

What is not normal is removing *thousands* of lines in one pass.
That happened: a `--repair` run removed 1,426 lines from 233 pages and printed `708 fixed`, with nothing in the output mentioning a single line ([RFC 0210](rfcs/0210-md-content-outside-op-log.md)).

So the doctor now measures, **before writing anything**, how many content lines on disk each re-projection would not reproduce:

- Every offered action carries its own cost — `re-project pages/notes.md from the op log — removes 12 content line(s) from disk`.
- A totals line states the workspace-wide figure, in `--repair` **and** in the default read-only mode, because read-only is where you decide whether to authorise the write.
- Past **100 content lines** or **20 pages that lose content**, the page repairs stand down and the run tells you so.
  Add `--force` to authorise them: `outl doctor --repair --force`.

A page the write only *adds* to counts as zero, so the ordinary bulk case — a device that just paired and has the whole graph unprojected — never asks for a flag it does not need.
Suppression is all-or-nothing over the page writes; corrupt-snapshot deletion, backup pruning and the device-store binding prune still run.
None of those three writes into your graph — the binding prune does not even touch the workspace — so holding them back would suppress nothing the ceiling exists to protect.

### Backups

Every file `--repair` touches is copied to `.outl/repair-backup/<timestamp>/<relative path>` **before** the write, so undoing a repair is a plain `cp` back.
Device-store records are the one thing that has no path relative to the workspace; they land under `<timestamp>/device-store/actors/` and restore into `<device_dir>/actors/`.

Those generations are pruned at the end of each `--repair` run, because they are otherwise permanent.
A prune is a repairable item like any other: it is listed under `N repairable item(s)` on a read-only run, and a `--repair` whose *only* work is a prune still runs.
That is the case the pruning exists for — a workspace with nothing wrong with it is exactly the one that would otherwise keep every generation forever.
`.outl/` is dot-prefixed, so iCloud drops it and iroh never ships it — but Syncthing, Dropbox and a shared volume all replicate it, and every generation is a full copy of every `.md` that run touched.
A generation has to fail **both** guards before it goes — older than 14 days *and* outside the 10 newest — and each prune is reported as its own action.
A directory whose age can't be read is always kept.

### Scope

`--repair` is CLI-only.
The `outl_workspace_doctor` MCP tool always runs read-only — a tool call is not the place to start rewriting files on your disk.
That includes the `.outl/orphans.log` rows described above, which is why they are gated behind `--repair` rather than written on every check.

#### `--json`

Emits the standard envelope.
`data` carries `workspace`, `actor`, `op_count`, `error_count`, `warn_count`, plus `findings[]` (each `{severity, message}`, severity being `ok` / `info` / `warn` / `error`) and `repairable[]` (what `--repair` would do).
Only after an actual `--repair` run, `data.repair` is present with `backup_dir`, `actions[]`, `repaired`, and `failed`.

