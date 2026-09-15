# Privacy Policy — outl

_Last updated: 2026-06-17_

outl is a local-first outliner for personal notes and daily journals.
This policy describes what data the app handles and what it does not.

## Short version

- We do not collect, transmit, or sell any of your data.
- We do not operate any backend service for outl.
- We do not use analytics, crash reporting, advertising IDs, or third-party SDKs of any kind.
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

That is the complete list. There is no hidden telemetry file, no usage cache uploaded anywhere.

## What outl does NOT do

- We do not run any servers that your copy of outl talks to.
- We do not include analytics SDKs (no Amplitude, Mixpanel, PostHog, Firebase, Segment, or similar).
- We do not include crash reporting SDKs (no Sentry, Crashlytics, Bugsnag).
- We do not include advertising SDKs or tracking pixels.
- We do not request access to your contacts, photos, camera, microphone, location, calendar, or health data.
- We do not transmit your notes anywhere. Markdown files stay on your device and in your iCloud only.

## Peer-to-peer sync

When you pair two of your own devices, they exchange notes directly over an encrypted iroh connection — outl operates no server that stores your data, and a relay only forwards already-encrypted bytes.
Pairing a device grants it your notes, so outl treats a paired peer as authenticated but not trusted: what it is still validated against, and what remains open, is recorded in [RFC 0155](rfcs/0155-peer-trust.md).

## iCloud sync (iOS)

Sync between your devices happens through Apple iCloud Drive. Your data flows through Apple's infrastructure under Apple's terms; the outl developer is not involved in that transit and has no access to the content. See Apple's iCloud privacy policy for details: <https://www.apple.com/legal/privacy/>.

## Code execution feature

outl can execute fenced code blocks (JavaScript, Python, Lua, Lisp) using interpreters embedded in the app.
Execution is bounded by a hard timeout per block — 2 seconds on iOS, 5 on desktop and in the terminal — runs entirely on your device, and the result is written back as a markdown subblock in the same file.
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

If we ever introduce a feature that would change what is described here (for example, optional crash reporting), this document will be updated to reflect it before the feature ships, and the change will be noted in the project changelog.

## Contact

Open an issue at <https://github.com/outlmd/outl/issues>.
