# When something does not work on mobile

Find the symptom, read the cause, do the fix.
Your notes are not at risk in any of these.
They are on the phone, and a delete in outl is a move to the trash, never a physical removal.

If the fix is not here, [open an issue](https://github.com/outlmd/outl/issues) with your OS version, the app version, and what you were doing.

Sections:

- [Sync](#sync)
- [Pairing](#pairing)
- [Reminders](#reminders)
- [Capture](#capture)
- [Editing](#editing)
- [Android only](#android-only)

---

## Sync

### The sync dot is orange

Orange means the phone cannot reach any paired device right now.
Go down this list in order.

1. **Nothing is paired.**
   Open **Settings → Devices and sync**.
   An empty list says there are no paired devices yet.
   [Pair a device](mobile.md#pair-a-phone-with-your-laptop).
2. **The other device is not running outl.**
   Your laptop has to have outl open (desktop app, terminal UI, or a running `outl serve`).
   A laptop that is asleep or has outl closed cannot answer.
   Open outl there, then tap the sync dot on the phone.
3. **Local Network is off (iPhone and iPad).**
   outl finds a laptop on your Wi-Fi through the local network.
   If you tapped "Don't Allow" the first time, the phone looks like it is on an empty network.
   Settings → outl → turn on **Local Network**.
4. **The network blocks device to device traffic.**
   Guest Wi-Fi and "client isolation" stop two devices on the same network from seeing each other.
   Move both to another network, or turn client isolation off on the router.
   Away from your home network the relay carries sync, which needs no setup but needs internet on both ends.
5. **The phone has no connection at all.**
   The dot also goes orange when the phone itself is offline.

Tap the dot to retry.
Taps while a pass is running are ignored on purpose.

### Notes from the laptop are not on the phone

- Pull down on the page, or tap the sync dot, and wait for the spinner to stop.
- Open **Settings → Devices and sync** and watch the live progress line.
  A first sync of a large workspace moves in batches and can take a while.
- Check the paired list still shows the laptop.
  Pairing sets up the link, and history crosses over on the first sync after it.

### Sync only works while outl is open

That is the OS, and it differs by platform.

- **iPhone and iPad:** iOS suspends the app's network a few seconds after you leave it.
  outl finishes the pass in flight, and iOS then lets it run a short pass a few times a day.
  Turn on **Background App Refresh**: iOS Settings → General → Background App Refresh (globally), and iOS Settings → outl → Background App Refresh.
  It never runs in the background if no device is paired.
- **Android:** the process is frozen soon after you leave.
  A catch-up runs about every 15 minutes while the process is alive.
  If Android has reclaimed it, sync resumes when you open outl.
  No notification or battery setting is involved.

### Laptop shows a red row every time I lock my phone

Locking the screen interrupts a sync that was in the middle of an exchange.
Nothing is lost, the other side re-sends on its next pass.
outl asks the OS for a short window to finish the pass first, so you see this less often, not never.

### "A different workspace"

If a log or panel says it rejected a peer for being on a different workspace, the two devices have separate workspace ids.
That happens when the phone never adopted the laptop's workspace during pairing.
Remove the device on the phone (**Devices and sync**), then pair again from a fresh code on the laptop.
If you had written notes on the phone before pairing, pairing merges both sides and deletes nothing.
Full explanation: [sync.md → Troubleshooting sync](sync.md#troubleshooting-sync).

---

## Pairing

### Scan QR says camera access is off

Settings → outl → **Camera**, turn it on, come back, tap **Scan QR** again.
The camera is used only to read the code.

### Pairing times out

- The code is single use and expires.
  Show a new one on the laptop.
- The laptop advertises addresses the phone cannot reach.
  VM bridges (Parallels, UTM, Docker) and VPN interfaces end up in the code.
  Stop them, show a new code.
- The phone changed network after the code was made.
  Show a new code.
- Both devices need the same recent build of outl.

### The pairing code does not show on the phone

Use **Show my QR** on the phone and scan it from the other device.
It works both ways.

---

## Reminders

### The reminder did not notify

Check in this order.

1. **Is outl running?**
   Reminders fire only while the app is running, in the foreground or the background.
   If the OS closed it, the notification waits until you open outl.
   This is a known gap, see [reminders.md → Background delivery](reminders.md#background-delivery--what-ships-today).
2. **Is delivery on?**
   Reminders tab → **Notify me on this device**.
3. **Is it inside quiet hours?**
   A reminder that comes due in the window fires after it ends.
4. **Are notifications allowed?**
   OS Settings → outl → Notifications.
5. **Does the block still carry the rule?**
   Long press the block, **Properties…**, and check for `remind::`.
   A rule that reads `until DONE` stops when the block is done.

### The banner has no Snooze or Done buttons

The buttons are attached when outl starts.
Open outl once and try the next reminder.

### I snoozed on the phone and the laptop still buzzed

A snooze silences every device, but the other device only learns of it when the two sync.
Sync, then it holds.

---

## Capture

### The line I captured is not in my journal

- Open outl.
  A share sheet or Shortcut capture waits in an inbox on this phone until the app opens.
- It lands in the journal of the day you captured it.
  Check yesterday if it was close to midnight.
- It is not on your other devices until outl has opened here and synced.

More: [mobile-capture.md](mobile-capture.md#if-nothing-shows-up).

---

## Editing

### The keyboard toolbar buttons moved

They reorder by how often you use them, but only between editing sessions.
**Settings → Lock button order** freezes them.
**Reset button order** goes back to the original layout.

### My `[label](url)` link does nothing

Only `http`, `https` and `mailto` open.
Other schemes are refused.

### A diagram shows as code

**Settings → Diagrams** is set to **Off**.
Set it to **Pele**.

### I deleted a block and cannot restore it from the phone

The trash is not on the phone yet.
Sync to a computer and restore it there.
In the meantime **Undo** in the keyboard toolbar steps back through recent edits.

### I need my notes as plain files

**Settings → Devices and sync → Export as Markdown**.
It copies every page, journal and attachment into a new folder next to the workspace.
The Files app does not list outl's storage, so to read those files, sync to a computer where they are ordinary `.md` files.

### I cannot find the setting for an `outl.toml` option

The phone has no terminal, and its Settings sheet has only the rows listed in [mobile.md](mobile.md#settings).
An option without a row there is not reachable from the phone today.
The full option list is [`config.md`](config.md).

---

## Android only

### "outl can't start on this device"

The app shows this instead of closing without a word when its native library does not load.
Your notes were not opened or changed.
The screen lists the CPU type (ABI), the memory page size, the Android version and the device model.
Open an issue and paste those lines.
The usual causes are a device with 16 KB memory pages running a build that was not linked for them, or a CPU type the APK does not include.
Download the latest APK from the release page first.

### No devices found on the same Wi-Fi

outl discovers devices on the local network with multicast, which Android turns off to save battery unless an app holds a lock.
outl holds it while the app is in the foreground.
Keep outl open on screen while pairing.
Sync in the background goes through your known devices and the relay, and needs no multicast.
