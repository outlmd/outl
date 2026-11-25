# Backups and restore

outl keeps local, versioned snapshots of your workspace, so a bad import, a deleted page or a broken sync can be undone after the fact.
Snapshots are git commits in a repository that lives outside your notes folder.
You never see it unless you go looking.

## What you get

- **Automatic.**
  The desktop app and the TUI take a snapshot in the background, at most once every 30 minutes, and only when something changed.
- **Manual.**
  `outl backup now` takes one when you want a checkpoint, for example before an import.
- **Safe to restore.**
  A restore extracts an old snapshot into a folder you choose.
  It never writes into your live workspace.

## Requirements

`git` on your `PATH`.
Without it, backups log a warning and do nothing, and your notes are unaffected.
The mobile apps (iOS and Android) have no `git`, so they take no snapshots.
Phone notes are covered by sync to your other devices and by the snapshots those devices take.

## Commands

```bash
outl backup status                      # set up? when was the last one?
outl backup init                        # create the repository (idempotent)
outl backup now                         # snapshot now
outl backup now -m "before roam import" # with your own message
outl backup list                        # newest first, 20 by default
outl backup list --limit 100
outl backup restore <id> --into ~/restore
```

Every command takes `--workspace <DIR>` and defaults to the current directory.

A first run on a fresh workspace:

```
$ outl backup status
no backup repository — run `outl backup init`
$ outl backup init
backup repository ready at /path/to/workspace
run `outl backup now` to take the first snapshot
$ outl backup now
$ outl backup list
c2879e2  2026-11-25T08:35:45-03:00  outl snapshot 2026-11-25 08:35
```

## Restore a page, step by step

1. Find the snapshot from before things went wrong: `outl backup list`.
2. Extract it somewhere else:

   ```
   $ outl backup restore c2879e2 --into ~/restore
   snapshot c2879e2 extracted to /Users/me/restore

   Your live workspace was NOT modified.
   Compare, then copy back only what you want:
     diff -ru /Users/me/restore /Users/me/notes
   ```

3. Open `~/restore/pages/<slug>.md`, copy the text you want back into the live page, or paste it into outl.
4. Prefer pasting into outl over copying files over the live workspace.
   outl turns your edit into ops, which sync to every device.
   Never copy files from the restore folder over `ops/`: the op log is append-only per device.

If you only lost text inside a block and the op log still holds it, `outl recover` reads the log directly.
It is read-only unless you pass `--apply` ([doctor.md](doctor.md)).

## What is in a snapshot

Included: `ops/` (the source of truth), `pages/`, `journals/`, `templates/`, `assets/` and `.outl/config.toml`.
Excluded: caches the next boot rebuilds (`.outl/snapshots/`, `*.idx`, lock files, `*.tmp`).

A `.gitignore` in your workspace cannot drop these paths.
Each snapshot checks that every `ops/*.jsonl` is in the commit and reports an error otherwise, so a history without your op log never passes as a backup.

## Where it lives

`<device dir>/backups/<workspace name>-<hash>.git`, where the device dir is `~/.config/outl/` unless `$OUTL_DEVICE_DIR` or `$XDG_CONFIG_HOME` moves it.
Three consequences:

- It is outside your notes folder, so Syncthing, Dropbox and iCloud never replicate git's internals.
- It is per device.
  Each machine keeps its own history.
- If you keep your notes in your own git repository, outl does not touch it: your index, branches and hooks are never read or written.
  Moving or renaming the workspace starts a fresh history and leaves the old one on disk.

## Settings

```toml
# ~/.config/outl/config.toml
[backup]
enabled = true          # false turns the automatic pass off
interval_minutes = 30   # minimum gap between automatic snapshots
```

`interval_minutes` is a floor, not a schedule.
The elapsed time is read from the git history, so restarting the app does not reset it.
`outl backup now` ignores both keys.

## Per-client behavior

| Client | Automatic snapshots | Manual |
|---|---|---|
| CLI | No (one-shot process) | `outl backup now` |
| TUI | Yes, first pass 60 seconds after launch | Same |
| Desktop | Yes, first pass 60 seconds after launch | Same, from a terminal |
| Mobile | No snapshots on a phone (no `git`) | Not available |

How the background pass is built and the rules a client must keep: [client-sync.md → Automatic backups](client-sync.md#automatic-backups).

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `no backup repository` | Never initialized, or the workspace moved | `outl backup init` |
| No snapshots appear | No changes since the last one, `git` missing, or `enabled = false` | `outl backup status`, then check `git --version` |
| History shorter than expected after moving the folder | A moved workspace starts a new repository | The old one is in the `backups/` folder of the device dir |
| An automatic pass logs `index.lock` | The TUI and the desktop snapshotted the same workspace at once | Harmless. The loser skips that interval |

Backups protect against your own mistakes.
They are not sync, and they stay on the machine that made them.
