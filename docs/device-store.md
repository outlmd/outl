# The device store

Where outl keeps the state that must **differ** per device, and why that is outside the workspace.

This is the detail behind `crates/outl-core/CLAUDE.md` → "Actor id is device-local, and the workspace cannot hold it", which carries the rule; this page carries the reasoning, the failure modes, and what the garbage collector refuses.
Decision record: [RFC 0211](rfcs/0211-state-that-leaves-a-boundary.md).

"One `ops-<actor>.jsonl` per device, never shared" is what makes last-write-wins-per-file harmless on every file transport.
That invariant is only as strong as where the actor id is stored.

It used to live at `<root>/.outl/config.toml` — **inside** the directory the user syncs.
Syncthing, Dropbox, NFS, a shared network volume and `git clone` all replicate `.outl/`, so both devices read the same `actor_id`.
`ActorWriteLock` does not catch it: `flock(2)` is advisory and machine-local, so each device acquires its lock successfully and both append to one file.
Last write wins, ops vanish, nothing errors.
The only reason this was not a daily disaster is that iCloud Documents drops dot-prefixed paths, so `.outl/` never travelled — an accident of one transport.

`device/` moves the answer outside the workspace:

- `DeviceStore::actor_for_instance(&WorkspaceId, root, fallback)` → `<device_dir>/actors/<workspace-id>`.
  Keyed by `WorkspaceId` because that is the id two paired devices *agree* on, which is what makes the actor they disagree on well-defined — **and** by the workspace directory, because the id lives at `<root>/.outl/workspace-id` and therefore travels inside a `cp -R`.
  Two copies of one directory keyed on the id alone share an actor, and iroh keys its gossip topic on that same id, so the copies reconcile as one workspace and dedup each other's genuinely-distinct ops by `ts`.
  A binding records the root it was made for; a mismatching root forks a second actor unless the recorded one is *provably gone* (a move or rename), and anything unreadable counts as still live.
- `DeviceStore::device_actor()` → `<device_dir>/actor`, the single device-wide actor the Tauri clients have always used (their `HlcGenerator` is bound at app start, before a workspace exists).
  Device-local already, so it never had the *cross-device* bug; it lives here so both GUI clients read one implementation.
- `DeviceStore::machine_id()` → `<device_dir>/machine-id`, a device fingerprint that *may* be published into the shared config, because it is only ever compared against the local value.
  Bound to a hash of an OS identifier of the physical machine (`/etc/machine-id`, `IOPlatformUUID`, `MachineGuid`) and **reminted when that changes**, because `$HOME` is replicated by Migration Assistant, Time Machine, VM images, chezmoi and NFS.
  A remint invalidates every actor binding stamped with the old id, so each workspace forks.
  Platforms exposing no such identifier (iOS above all) are inconclusive and change nothing — a documented gap, not a silent one.

Every device-store file is `key=value` lines composed in a sibling temp file and published in one step; a create is a compare-and-swap, so two processes racing a first open converge rather than minting two ids.
The publish step is `link(2)`, not an `O_EXCL` open, because an `O_EXCL` open creates the file **empty** and fills it a moment later.
A reader landing in that window parses a blank record as *absent* — exactly the answer that licenses it to overwrite the winner.
`machine_id` mints under that same compare-and-swap, and matters more there.
A lost actor costs one extra ops file; a lost machine id invalidates every binding **and** every `actor_claimed_by` claim already in a workspace config, so those workspaces never adopt their legacy ops file again.
A bare legacy line (the Tauri clients' plain-ULID `actor` file) still parses.

`device_dir()` honours `$OUTL_DEVICE_DIR` before the XDG layout.
That override is what keeps the test suite (and any container) off the developer's real store — the repo's `.cargo/config.toml` points every cargo-spawned process at `.dev-device-store`.
That path is deliberately **not** under `target/`, which `cargo clean` erases along with the iroh identity key that is this device's node id.
`the_test_suite_runs_against_an_isolated_device_store` fails outright when that file is missing, because a suite that silently writes into `~/.config/outl/` is how 64 entries got there, 15 of them pointing at `TempDir` paths that no longer exist.

#### The store has a GC now, and its whole design is what it refuses

`device/gc.rs` answers invariant 9's fourth question for this store: *what cleans it up?*
Until it existed, nothing did — `actors/` gained one record per workspace this device ever opened and lost none, so a workspace the user deleted kept its binding forever (1,208 records on a dev machine, 1,166 orphaned).

**Dropping a binding is not free**, and that asymmetry is the entire design.
The next open of that workspace mints a *fresh* actor — a second `ops-<actor>.jsonl` for a device that already had one, with every op it previously wrote no longer attributed to it.
That is the fork this store exists to prevent, so a GC that guesses wrong causes the bug it is tidying up after.
Keeping a stale record costs ~190 bytes.

So "the root is missing" is **not** the rule, because an unplugged drive, an unmounted network volume, an undownloaded iCloud folder and an archived workspace all look exactly like a deleted one.
A binding is `BindingVerdict::Stale` only when the root is gone, its *parent* directory is still present, and the record is older than `STALE_BINDING_TTL` (30 days).
The parent check is what does the real work: a deleted folder leaves its parent behind, while a missing mount takes the whole path with it.
A workspace that is itself a mount point defeats the parent check alone (unmounting `/Volumes/Notes` leaves `/Volumes`), so a binding also stamps the root's filesystem device id (`dev=`), and a surviving parent on a different filesystem keeps the entry.
Everything else — including a record with no `root=`, and any path we failed to *read* rather than observed to be absent — is `Inconclusive`, which always keeps it.

Two things about that rule are easy to misread, and both are pinned by tests:

- **The TTL is the record's age, not time since the deletion.**
  A binding is written on first open and rewritten only when its workspace *moves*, so nothing records when a directory went away.
  A workspace bound years ago and deleted a minute ago is `Stale` immediately (`an_old_binding_whose_workspace_just_vanished_is_stale`).
  Buying the stronger reading means stamping `seen=` on every open, which turns the common read path into a write on a store that may be read-only — a trade not made here.
- **A record that does not survive the parse is not evidence.**
  `write_record` does not escape and `parse` trims, so a root ending in a space (or holding a newline) reads back as a *different*, non-existent path whose parent exists — the exact shape that authorises a delete, for a workspace that is alive.
  `Record::is_lossy` reports the failed round trip and `judge` drops the root rather than trusting it.
  The same defect exists one layer earlier: the writer serializes the root via `Path::display()`, which replaces non-Unicode path data with U+FFFD before the parser ever sees the text, so `judge` also drops a root carrying the replacement character.
  Before the GC that leniency cost one redundant rewrite per open; the GC is what changed its price.

`gc.rs` is the **single owner** of that verdict.
`DeviceStore::prune_binding` re-asks it immediately before deleting, because listing and pruning are two passes with a user in between, and a workspace can come back in that gap.
It also refuses any path outside `actors/`: `iroh/identity.key` **is** this device's node id.

The same module also collects **abandoned scratch files** (`STALE_SCRATCH_TTL`, 24h).
`record.rs` composes every write in a `.<name>.<pid>.<seq>` sibling and removes it after publishing, so a killed process leaves one behind forever.
They stay out of the binding listing on purpose: a scratch file names no workspace, so reporting one as "a binding whose workspace is gone" invents a graph that never existed.
They are also never backed up, because a half-published write is by definition content that never became a record.
Deleting one a live writer still holds is survivable on both publish paths — `create_new_record`'s `hard_link` fails with something other than `AlreadyExists` and falls through to `exclusive_create`, and `write_record` recomposes a scratch its `rename` found missing and publishes again.

The surface is `outl doctor` (reports the count) and `outl doctor --repair` (drops them, after a backup) — see `outl-cli/CLAUDE.md`.
`scripts/gc-dev-device-store.sh` is the *developer's* sweep of `.dev-device-store`, differing in **exactly one** way: no TTL, because test debris does not deserve a 30-day wait.
It carries the parent check — the condition that actually protects a live workspace — which it used to skip, deleting on `[ -d "$root" ]` alone, and since it reads `$OUTL_DEVICE_DIR` that rejected rule reached real bindings.
If the two diverge again, align the **script** to `gc.rs`, never the reverse.

**Migration lives in `outl_ws::actor`, not here**, because it needs `config.toml`.
`config.toml`'s `actor_id` is a legacy value adopted only by the device named in `[workspace] actor_claimed_by`, and that marker is stamped when the config is **created**, never on first open — the default transport (iroh) never ships `config.toml`, so a claim written at open time propagates to nobody.
A workspace with no claim is adopted by nobody: every device forks once and the old ops file stays readable.
Read that module before changing anything about actor resolution.

Rules that follow:

- ❌ Never re-derive the write actor from anything under `<root>/`.
  A value two devices can read identically is not a device identity.
- ❌ Never put the machine id, or any other device-local value, on the sync surface.
  It is the opposite of invariant #7: state that must **diverge** per device must never travel.
