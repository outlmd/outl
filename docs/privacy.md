# Privacy Policy — outl

_Last updated: 2026-10-18_

outl is a local-first outliner for personal notes and daily journals.
This policy describes what data the app handles and what it does not.

## Short version

- Your notes are private. They are never sent to us, and never part of a crash report.
- We do not sell any of your data, and we do not use analytics, advertising IDs or tracking.
- We do not operate any backend service for your notes.
- When outl crashes, it sends a crash report to Sentry, a crash-reporting service. This is **on by default** and you can turn it off. [Details below](#crash-reports).
- Your workspace lives entirely on your own devices and (optionally) in your own iCloud Drive.
- There is no account, no sign-in, no email collection.

If anything below is unclear or you have a question, open an issue at <https://github.com/outlmd/outl/issues>.

## What outl stores, and where

Everything outl saves lives in one of two places, both controlled by you:

1. **Your device's local filesystem** — when you use outl without iCloud (TUI, desktop), the workspace is a folder you pick.
2. **Your iCloud Drive** — on iOS, outl uses your iCloud "outl" container (`iCloud.app.outl.mobile-app`). Your Apple ID owns it. The developer never sees it.

A workspace contains:

- Markdown files (`.md`) — your notes, exactly as you wrote them.
- Sidecar files (`.outl`) — small JSON files that map block IDs to your markdown so outl can resync without changing the markdown.
- An append-only operation log (`ops-<actor>.jsonl`) — one file per device, used internally to merge edits across devices.
- A per-device anonymous identifier (a random ULID stored in the app's sandbox at `<sandbox>/actor`) so the merge engine can label each device's edits. This identifier never leaves your device or your iCloud, is not linked to any account, and is regenerated if you delete the app.

That is the complete list. There is no usage cache and no log of what you do. The only thing outl sends off the device on its own is a crash report, described in [Crash reports](#crash-reports).

## What outl does NOT do

- We do not run any servers that your copy of outl talks to.
- We do not include analytics SDKs (no Amplitude, Mixpanel, PostHog, Firebase, Segment, or similar).
- We do not send anything except crash reports (see [Crash reports](#crash-reports)). No usage events, no performance traces, no session data.
- We do not include advertising SDKs or tracking pixels.
- We do not request access to your contacts, photos, camera, microphone, location, calendar, or health data.
- We do not transmit your notes anywhere. Markdown files stay on your device and in your iCloud only.

## Crash reports

When outl crashes, it sends the crash to us through [Sentry](https://sentry.io).
That is how a crash on a device we do not have gets fixed, and by sending it you are helping improve the product.
Your notes are private and are not sent.

Only a crash (a Rust panic) is reported.
Errors that outl handles, performance data and usage events are not.

### What is sent

- the name of the client (TUI, CLI, desktop, mobile)
- the outl version
- the operating system, as a bare name (`macos`, `linux`, `windows`, `android` or `ios`) — **not** its version, build or kernel
- the CPU architecture (`aarch64`, `x86_64`, ...)
- the place in our own source code where the crash happened, as a file name and a line number (`mod.rs:42`) — never a path from your disk
- the panic message, after it has been run through a scrubber that removes file paths and page identifiers

### What is never sent

- page titles, page slugs or block text
- file paths, including the path of your workspace
- your hostname, username or environment variables
- your IP address (storing it is disabled in the Sentry project; see below)
- any persistent identifier: no device id, no actor id, nothing that links two reports to the same person or device
- any account information, because there is no account

### On by default, and how to turn it off

Crash reporting is on by default.
An earlier version of this policy said any crash reporting would be optional (opt-in).
It ships as opt-out instead: a reporter nobody turns on says nothing about the users who hit a crash and never file an issue, and those are the ones it is for.
You can see this in the first-run notice and turn it off at any time:

- **Desktop:** Settings, Privacy section.
- **Mobile:** Settings, Privacy section.
- **Terminal (TUI and CLI):** run `outl telemetry off`. `outl telemetry status` shows the current state and `outl telemetry on` turns it back on. `OUTL_TELEMETRY=0` silences a single invocation.

The choice is stored in `~/.config/outl/config.toml` under `[telemetry]`, so it applies to every outl client on that machine.
No client reports from a CI environment.

If `~/.config/outl/config.toml` cannot be read — a hand edit that broke the syntax, a half-written copy from another device — outl sends nothing rather than falling back to the default.
The file is the only record of your answer, so a file we cannot read is a question we cannot answer, and the safe reading of silence is no.
`outl telemetry status` says so and exits non-zero.

Builds that you compile yourself do not report, because they do not contain our Sentry key.

### Limits of the coverage

Only crashes in outl's Rust code are reported.
A crash in the Android Kotlin code or in the iOS native bridge is not.

### Retention

Reports are kept for the shortest period the Sentry plan allows, and the Sentry project is configured not to store IP addresses.
Reports are used only to fix crashes.

## Peer-to-peer sync

When you pair two of your own devices, they exchange notes directly over an encrypted iroh connection — outl operates no server that stores your data, and a relay only forwards already-encrypted bytes.
Pairing a device grants it your notes, so outl treats a paired peer as authenticated but not trusted: what it is still validated against, and what remains open, is recorded in [RFC 0155](rfcs/0155-peer-trust.md).

## iCloud sync (iOS)

Sync between your devices happens through Apple iCloud Drive. Your data flows through Apple's infrastructure under Apple's terms; the outl developer is not involved in that transit and has no access to the content. See Apple's iCloud privacy policy for details: <https://www.apple.com/legal/privacy/>.

## Code execution feature

outl can execute fenced code blocks (JavaScript, Python, Lua) using interpreters embedded in the app.
Execution runs entirely on your device, and the result is written back as a markdown subblock in the same file.
Every block has a deadline (2 seconds on iOS, 5 on desktop and in the terminal) after which the app stops waiting and reports a timeout.
What happens to the block itself depends on the language: a Lua block (and, on desktop, a Rust block) is stopped at the deadline, while a Python or JavaScript block that ignores it keeps running in the background, using one CPU core, until the app exits.
After a few such blocks the app refuses to start another until it is restarted, so a runaway block cannot quietly consume the whole machine.
No code is downloaded from the internet, no remote code is executed, and the feature does not send your code anywhere.

A block runs only when you ask — pressing `gx`, or setting `auto-run::` on it yourself.
The interpreters are sandboxed to the language itself: a block cannot reach the filesystem, run programs, read environment variables, or open network connections.
The one thing a block can read is your own workspace, and only through an explicit API (`outl.query`) that answers questions about your notes — the same notes the block is written in.

A block also runs when you finish editing a `call:` block, which executes the code on the template's page rather than code you typed in front of you.

Note that a code block is not always one you typed.
A note that arrived from another of your paired devices, or from a graph you imported, can contain one, and it runs with the same permissions as a block you wrote.
Treat a fenced block from a source you do not control the way you would treat any other file from that source.

## Children

outl is a productivity tool with no social features, no chat, no shared content, and no user-generated content directed at others. It is suitable for general audiences.

## Open source

outl is open source under the MIT License. The full source code is at <https://github.com/outlmd/outl>. You can verify everything stated above by reading the code.

## Changes to this policy

If we introduce a feature that changes what is described here, this document is updated before the feature ships, and the change is noted in the project changelog.
The 2026-10-18 update is one: crash reporting, which this page used to rule out and had earlier described as a possible opt-in feature, now ships on by default with an opt-out.

## Contact

Open an issue at <https://github.com/outlmd/outl/issues>.
