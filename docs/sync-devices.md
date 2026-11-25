# Sync your devices

Put the same notes on a laptop, a desktop and a phone, edit on any of them offline, and let them merge when they meet.
This page is the task-oriented guide: pick a transport, pair two devices, check that it works, fix it when it does not.
[Sync, done right](sync.md) is the long version of why the merge is safe.

## The short version

1. Install outl on both devices.
2. On the device that has your notes, start pairing.
   It shows a QR code and a ticket.
3. On the other device, scan the QR (phone) or paste the ticket (`outl peer pair --ticket`).
4. Wait for the green sync dot.

No account, no server of yours, nothing to configure.
Devices talk directly over an encrypted connection, and a relay helps them find each other when a firewall is in the way.
The relay forwards bytes it cannot read ([relay.md](relay.md)).

## Pick a transport

| | `iroh` (default) | `file` |
|---|---|---|
| How devices exchange notes | Direct encrypted connection, relay as fallback | A folder that something else syncs (iCloud Drive, Syncthing, a network share) |
| Setup | Pair each device once | Point every device at the same folder |
| Works across Apple and non-Apple devices | Yes | Only as far as the folder tool does |
| Both devices must be online at the same time | No. Ops queue on disk and are pulled on reconnect | No |
| Config | `[sync] transport = "iroh"` | `[sync] transport = "file"` |

Stay on `iroh` unless you already trust a folder-sync tool with your notes.
Switch with `transport = "file"` in `~/.config/outl/config.toml` ([config.md → `[sync]`](config.md#sync)).
For `file`, every device opens the same synced folder as its workspace, for example `outl --workspace ~/Library/Mobile\ Documents/iCloud~app~outl~mobile-app/Documents`.

## What each client can do

| | Pair (host: show QR / ticket) | Pair (join: scan or paste) | See paired devices | Sync now | Remove a device |
|---|---|---|---|---|---|
| CLI | `outl peer pair` | `outl peer pair --ticket <ticket>` | `outl peer list`, `outl peer status` | `outl sync` | `outl peer remove <id>` |
| TUI | Run `outl peer pair` in a shell | Run `outl peer pair --ticket` in a shell | `:sync` panel | `:sync` panel | `outl peer remove` in a shell |
| Desktop | Settings, Sync, **Pair a new device…** | Not in the app. Use `outl peer pair --ticket` | Settings, Sync | Sync dot click, or **Refresh** | Settings, Sync |
| Mobile | Settings, **Devices and sync**, **Show my QR** | **Scan QR** (camera) | Devices sheet | Devices sheet, pull to refresh | Devices sheet |

The sync dot in the top bar of the desktop and mobile apps summarizes this at a glance: green means at least one paired device is reachable, orange means none is.

## Pair two devices

The device that has your notes is the **host**.
The device that joins is the **joiner**.
When pairing finishes, the joiner adopts the host's workspace identity: from then on both folders are the same graph.
An empty folder on the joiner is the normal case.
If the joiner already held notes, they merge into the host's graph and nothing is deleted.

### Desktop host, phone joiner

1. On the desktop, open Settings (`Cmd/Ctrl+,`) and go to **Sync**.
2. Type a name for this device if you want one (default `desktop`).
   The name shows up on your other devices.
3. Click **Pair a new device…**.
   A QR code and a ticket appear with "Waiting for device to scan…".
4. On the phone, open Settings, then **Devices and sync**, then **Scan QR**.
   Allow camera access when iOS asks.
5. The desktop shows "Paired with `<phone name>`".

The code works for two minutes and once.
Cancel and start again for a fresh one.

### Two computers, from the terminal

On the host, from the workspace folder:

```bash
cd ~/my-notes
outl peer pair
```

It prints your node id, a QR code and a ticket, then waits.
On the joiner, from the folder that should become the replica:

```bash
mkdir ~/my-notes && cd ~/my-notes
outl peer pair --ticket <ticket>
outl sync
```

Add `--name laptop` to either side to choose the label the other device shows.
`--ticket -` reads the ticket from stdin, and `outl peer qr <ticket>` redraws a QR when the terminal was too narrow the first time.
Full flag list: [cli.md](cli.md#workspace--admin).

### A server that is always on

A small box that stays online means your devices sync even when no two of them are awake together.
See [self-hosting.md](self-hosting.md).

## Check that it works

```bash
outl peer list     # who is paired (reads a file, no network)
outl peer status   # dials each peer now: online (Nms) or offline
```

In the apps, open the Sync panel (desktop) or Devices sheet (mobile).
Each device shows its state, and a live line shows what is moving ("Receiving 42 changes from laptop").

| What you see | Meaning | What to do |
|---|---|---|
| Green dot | A paired device answered or dialed in recently | Nothing |
| Orange dot | No paired device is reachable | Open outl on the other device. A phone only answers while the app is open or in a background window |
| Amber "went away, will retry" | The other device locked its screen or slept mid-sync | Nothing. The next pass resends |
| Red "failed" | The other device refused or the two builds cannot talk | Update both to the same version. If you removed this device from the other one, pair again |

## Daily use

- **Edit offline.**
  Every edit is written to your device's own op log first.
  Reconnecting pulls only what the other side is missing, however long you were away.
- **Sync now.**
  Click the sync dot on desktop, pull to refresh on mobile, or run `outl sync`.
- **Phone in your pocket.** iOS and Android only give the app short background windows.
  Edits made elsewhere are usually there when you open the app, and one tap on the dot pulls the rest.
  On iOS, enable Background App Refresh for outl.
- **`outl` commands in a script.**
  A one-shot CLI command writes to disk and exits.
  Another running outl (app, TUI, `outl serve`) pushes it out, or run `outl sync` to flush yourself.
  `outl sync` exits with code 3 when nothing was flushed (no paired device, P2P off, or another process on this machine already holds the connection).

## When two devices edit the same thing

You do not resolve conflicts by hand.

| Situation | Result |
|---|---|
| Both edit different blocks | Both edits land |
| Both type into the same block | Text merges character by character |
| Both move the same block to different parents | One move wins, the same one on every device. The other is kept in the log, not lost |
| Two moves that would put each block inside the other | One applies, the other becomes a no-op. No cycle ever forms |

The algorithm and the proof: [crdt.md](crdt.md).
If your folder-sync tool (iCloud, Dropbox, Syncthing) creates `… (conflicted copy)` or `.sync-conflict-…` files, `outl doctor` lists them and never picks a winner for you ([doctor.md](doctor.md)).

## Lost or retired device

| Goal | Command | Scope |
|---|---|---|
| Stop syncing with a device you still own | `outl peer remove <id-prefix>`, or remove it in the Sync panel | This machine only. Run it on each device that paired with it |
| Lock out a device you no longer control | `outl peer revoke-all` (asks you to type `revoke`; `--yes` to skip) | Rotates the workspace identity. Re-pair every device you keep |

Neither can take back notes the device already received.
Details and limits: [sync.md → Managing peers](sync.md#managing-peers).

## Troubleshooting

| Symptom | Likely cause | Fix |
|---|---|---|
| Paired, nothing appears | Pairing links devices, it does not pull history | Run `outl sync`, or open the app and wait for the dot |
| `rejecting sync from peer on a different workspace` in the log | The joiner never adopted the host's workspace id | Update both devices, then `outl peer pair --ticket` again from the joiner's replica folder |
| Pairing hangs about 30 seconds, then times out | One of the host's advertised addresses is unreachable (VM or VPN interface, guest Wi-Fi) | Stop the VPN or VM, mint a fresh ticket, or put both devices on the same network |
| Peer shows offline but you know it is on | It is behind a NAT and only dials out, or its app is closed | Open the app on that device. A dial in from it also turns the dot green |
| iPhone never finds the computer on Wi-Fi | Local-network permission was declined | Allow outl under the iOS Local Network setting. Sync still works through the relay |
| Works from the installed app, not from `cargo run` | A source build is a separate device with its own node id | [development.md](development.md#testing-p2p-sync-from-a-source-build) |
| Restrictive network (VPN, TLS-inspecting proxy) | Relay traffic blocked | [relay.md → Troubleshooting](relay.md#troubleshooting-restrictive-networks-vpn-tls-inspection-proxy) |

More cases, each with the log line to look for: [sync.md → Troubleshooting sync](sync.md#troubleshooting-sync).

## Settings that affect sync

| Key | Default | Effect |
|---|---|---|
| `[sync] transport` | `"iroh"` | `"iroh"` or `"file"` |
| `[sync] relay_url` | outl's relay | Your own `iroh-relay`. Ignored by `file` |
| `[backup] enabled` | `true` | Local git snapshots as a safety net ([backups.md](backups.md)) |

Every key: [config.md](config.md).
