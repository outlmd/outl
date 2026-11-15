# App Store review notes (iOS)

App Store Connect asks for **App Review Information > Notes** on every submission.
Build 0.13.704 (version 0.13.4) was returned under Guideline 2.1 (Information Needed) because that field was empty.
This page is the source of truth for what goes in it: the block under [Review Notes (paste into App Store Connect)](#review-notes-paste-into-app-store-connect) is meant to be copied verbatim, and everything else on the page is for the maintainer.

Every claim in the paste-ready block is checked against the code, so change the code and this page together.
Where a fact has another owner, this page links to it instead of restating it:

- What leaves the device, and the crash reporter: [privacy.md](../privacy.md).
- The relay and what it can see: [relay.md](../relay.md).
- Pairing and sync: [sync.md](../sync.md).
- Bundle id, entitlements and export compliance: [ios-platform.md](../ios-platform.md).

---

## Review Notes (paste into App Store Connect)

The block below is plain ASCII and fits the 4,000 character limit of the Notes field.

```text
PURPOSE AND AUDIENCE
outl is a local-first outliner and daily journal. It is a personal productivity app for anyone who keeps notes, to-dos and a daily journal. Notes are plain Markdown files stored on the device. General audience; there is no social or shared content.

HOW TO REVIEW (about 2 minutes, one device, no setup)
1. Launch the app. First run shows two onboarding screens.
2. Tap "Keep on this device". On the next screen tap "Start writing". (Skip "Pair a device": it is optional and only needed to sync a second device of the same person.)
3. You land on today's journal. Tap "Tap to start writing" (or the + button) and type a few bullets. Use the keyboard toolbar to indent a bullet and to toggle a TODO.
4. Use the week strip at the top to open the calendar, and the gear icon in the header to open Settings > Privacy > Crash reports.
The screen recording attached to this submission shows exactly this flow on a physical iPhone.

NOT APPLICABLE
- Accounts: there is no sign-up, sign-in, email or user account, so there is nothing to delete and no demo login or password.
- User-generated content shared with others: none. Notes are private to the user. There is no feed, chat, public page or content from other people, so report/block does not apply.
- In-app purchases and payments: none. No StoreKit, no subscription, no payment provider, no ads.
- Demo data: not needed. The app starts with an empty journal.

EXTERNAL SERVICES USED BY THE CORE APP
- Crash reports (Sentry): on by default, can be turned off in Settings > Privacy > Crash reports. Only Rust panics are reported: client name, app version, OS name (no version), CPU architecture, source file and line, and a scrubbed panic message. Never sent: notes, page titles, file paths, IP address, device or user identifiers. Privacy policy: https://outl.app/docs/privacy.html
- Sync relay (only if the user pairs devices, which the review does not need): peer-to-peer sync between the user's own devices uses iroh over QUIC with TLS 1.3. For NAT traversal, and to forward already-encrypted bytes when a direct connection is not possible, the app contacts a relay at use1-1.relay.avelino.outl.iroh.link (public n0 relays are a documented fallback). The relay cannot read note content.
- Optional plugin marketplace: opening the plugin list (Browse) fetches a public index from plugins.outl.app. No plugin is bundled, installed or enabled by default. A plugin can only reach the network if the user installs it and it declares a named host.
- Not used: analytics, advertising SDKs, an auth provider, or any cloud model or AI API. Fenced code blocks in a note run only on the device, inside embedded interpreters with no file, process or network access.
- Storage is an app-local folder. iCloud is not used and is not needed for review.

PERMISSIONS
- Camera: only to scan a pairing QR code when the user taps "Pair a device". Not needed for review.
- Local Network: only to find the user's other devices on the same Wi-Fi. If the prompt appears, Allow is recommended, but the journal works the same if declined.

REGIONS
Features are identical in every region. The only regional effect is that a configured calendar timezone decides which date counts as "today".

REGULATED OR LICENSED CONTENT
The app is not in a regulated industry and includes no third-party protected material that needs a license.
```

---

## Screen recording script (maintainer, not for pasting)

Apple asked for a recording of the typical flow on a **physical device**.
A simulator capture or a still of the splash screen does not meet that.

Preparation:

- Use a physical iPhone on a current iOS release, with the build that was rejected (or a newer one).
- For a first-run walkthrough, delete the app and install it again, so onboarding shows.
  Reinstalling also gives a fresh, empty journal.
- Turn on Do Not Disturb and start Control Center screen recording with the microphone off.
- Do not pair a device, so the camera prompt never appears.
- Everything typed below is synthetic.
  Do not type real notes, names or places.

Steps, in order:

1. Start recording, then open outl from the Home Screen (the app launching on camera, not a splash still).
2. If the Local Network prompt appears, tap **Allow**.
3. On "Where do your notes live?", tap **Keep on this device**.
4. On "Sync with another device?", do **not** open "Pair a device".
   Tap **Start writing** (or "Skip for now"; both land on the same screen).
5. Today's journal is open and empty.
   Tap "Tap to start writing" and type:
   `Plan the week`
6. Press return and type:
   `Buy oat milk and apples`
   Tap **Indent** in the keyboard toolbar so it nests under the first bullet.
7. Press return and type:
   `Pick up the dry cleaning`
   Tap **Toggle TODO** in the toolbar, so the bullet becomes a task.
8. Press return, tap **Outdent**, and type:
   `Read chapter 3 of [[Reading list]]`
9. Close the keyboard, tap the `Reading list` link to open that page, and type:
   `Finished chapter 2 on Sunday`
10. Go back to today's journal (the back control in the header, or the **Journal** tab in the bottom bar).
11. Open the calendar from the week strip, pick another day, then return to today.
12. Tap the gear icon in the header to open Settings, scroll to **Privacy**, and pause on the **Crash reports** row so the opt-out switch is readable.
13. Close Settings and stop recording.

Do not record a login screen (there is none) and do not use a second device.
Keep the video short, around two minutes.

### Sample result (reference only, not part of the notes)

Helps check that the recording matches what the app writes.
Today's journal file should end up with this content (the date in the file name is the day you record):

```markdown
- Plan the week
  - Buy oat milk and apples
  - TODO Pick up the dry cleaning
- Read chapter 3 of [[Reading list]]
```

The app has no graph import on iOS, so do not ask reviewers to load a vault.
Nothing in the repo needs to be copied onto the device for the review.

---

## Maintainer checklist (not for pasting)

What a human still has to do.
Merging this page changes nothing in App Store Connect and does not resubmit the app.

- [ ] Paste the [review notes block](#review-notes-paste-into-app-store-connect) into App Store Connect > App Review Information > Notes.
- [ ] Record the video on a physical iPhone with the [script above](#screen-recording-script-maintainer-not-for-pasting).
- [ ] Attach the video in App Review Information.
- [ ] Replace the store screenshots with the app in real use: today's journal with notes in it, not a splash screen.
- [ ] If App Store Connect asks the export compliance questions for the build, answer them there.
  `ITSAppUsesNonExemptEncryption` stays undeclared on purpose; see [ios-platform.md](../ios-platform.md#export-compliance-the-flag-is-about-the-paperwork-not-about-the-code).
- [ ] Before resubmitting, read the [privacy policy](../privacy.md) once as a reviewer would.
  It still describes iCloud Drive as the iOS storage and says the app does not ask for camera access, while the build stores notes in an app-local folder and carries a camera usage string for pairing.
  Reconcile it if a reviewer is likely to compare the two.
- [ ] Resubmit the build for review.
