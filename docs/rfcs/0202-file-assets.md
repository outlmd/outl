# RFC 0202 — Asset bytes are content-addressed blobs, deliberately outside the op log

| | |
|---|---|
| **Status** | Shipped |
| **Issue** | [#202](https://github.com/outlmd/outl/issues/202) (anchor), [#203](https://github.com/outlmd/outl/issues/203) |
| **PR** | [#201](https://github.com/outlmd/outl/pull/201) |
| **Date** | 2026-08-06 |
| **Reference doc** | [markdown-format.md § Asset links](../markdown-format.md#asset-links-nameassetshashext) |
| **Invariant** | root `CLAUDE.md` invariant 7 — **explicit exception**, licensed by content-addressing; see "Why this is not a violation" |
| **Guarded by** | `import_copies_and_content_addresses`, `import_bytes_content_addresses_with_given_ext`, `identical_bytes_dedupe_to_one_path`, `extensionless_file_imports_cleanly`, `oversize_is_rejected_before_copy`, `resolve_finds_existing_asset`, `resolve_missing_asset_is_none`, `resolve_rejects_traversal_and_external`, `filename_special_chars_are_escaped_in_the_label`, `weird_extension_is_sanitised_to_alnum`, `image_uses_embed_not_plain_link`, `non_image_uses_plain_link_not_embed` (`crates/outl-actions/src/asset.rs`), `detects_asset_links`, `safe_asset_names_accept_plain_basenames`, `safe_asset_names_reject_traversal_and_separators` (`crates/outl-md/src/asset.rs`), `assets_transfer_from_peer`, `asset_pull_from_peer_without_assets_is_harmless` (`crates/outl-sync-iroh/tests/regression.rs`), `asset_manifest_roundtrips`, `empty_asset_manifest_is_valid_zero_length_frame`, `decode_asset_manifest_rejects_short_buffer`, `asset_manifest_drops_names_with_embedded_newline` (`crates/outl-sync-iroh/src/protocol.rs`), `missing_assets_section_defaults_to_100_mib`, `partial_assets_section_keeps_other_defaults` (`crates/outl-config/src/schema.rs`), `"resolves a local image asset to an <img> via readAssetDataUrl"`, `"refuses a non-http(s) remote image scheme and shows a chip"`, `"falls back to a file chip when a local asset fails to load"` (`crates/outl-frontend-shared/src/markdown/MarkdownInline.test.tsx`), `only_yrs_text_may_put_opaque_bytes_in_the_op_log`, `every_variant_has_a_sample` (`crates/outl-core/src/op.rs`) |

## Why

Users attach files.
A PDF of a contract, a screenshot of a dashboard, a scan.
Before [#202](https://github.com/outlmd/outl/issues/202) the only way to reference one from a block was to type the path to wherever the file already sat on that machine.
That is a dead link on every other device, and a dead link on the same device the moment the user reorganizes a folder.
The workspace stopped being self-contained exactly where the user needed it most.

The interesting part of this problem is not the copy.
It is that the obvious fix is wrong in a way that cannot be undone.

Everything else in outl that must survive on two devices goes through the op log — that is root `CLAUDE.md` invariant 7, and it is stated as a *default position*: model it as an `Op`.
Applied here, that reads as "add an `Op` carrying the bytes", and it fails in three compounding ways:

- **The log is replayed in full.**
  Every device replays every op at boot, and ships them to every peer.
  An ordinary op is a JSON line of tens to a couple of hundred bytes.
  One 8 MB attachment is worth tens of thousands of them, on every device, forever.
- **The log is append-only, and `Delete` is a `Move`** (invariant 6).
  There is no operation that removes an op.
  Attaching a file by mistake would be permanent on every machine that ever synced, with no compaction path that does not break replay determinism.
- **It would undo work already paid for.**
  [RFC 0137](0137-storage-scale.md) exists to keep boot and RSS constant as the log grows.
  Binary payloads inside the log defeat it by construction.

So the decision worth recording is not "we added an `assets/` directory".
It is *why* this one category of state is allowed to live outside the log, and under what condition that permission expires.

## What we chose

An uploaded file is copied to `<workspace>/assets/<sha256-hex>.<ext>` and referenced from markdown by a clean link.
The bytes never enter the op log.
Only the **link** is workspace state, and it enters the log the ordinary way, as an `Op::Edit` when the caller inserts it into a block.

```markdown
- the diagram: ![diagram.png](assets/9a8b7c…d6e5.png)
- see the spec: [proposal.pdf](assets/1b2c3d…e4f5.pdf)
```

Four owners, each with a single job:

| Layer | Owner | Job |
|---|---|---|
| Pure primitives | `outl_md::asset` | `ASSETS_DIR`, `hash_bytes`, `asset_rel_path`, `is_asset_link`, `is_safe_asset_name` — no filesystem, ever |
| Filesystem | `outl_actions::asset` | `import_asset` / `import_asset_bytes` (atomic tmp + rename, capped by `[assets] max_bytes`), `resolve_asset_path`, `assets_dir` |
| Wire | `outl-sync-iroh::engine_assets` | the `outl-asset/1` ALPN: manifest negotiation, per-name pull, hash verification, best-effort |
| Render | `@outl/shared` + `outl-tui` | `<img>` / file chip / TUI placeholder — display only, decided from the url extension |

### Why this is not a violation of invariant 7

Invariant 7 is about state two devices can **disagree** about.
Its exact words: "if two users (or one user on two devices) can disagree about a value and you want them to reconcile, the state belongs in an `Op`".

A content-addressed blob cannot disagree.
The filename *is* the SHA-256 of the bytes, so two devices that write `assets/<hash>.pdf` write byte-identical content, and last-write-wins resolves to a no-op.
Disagreement would require a hash mismatch, and a hash mismatch is a **different filename** — a different asset, not a conflicting version of one.
There is no merge to perform, so there is nothing for the CRDT to do.

That is the entire licence for this exception, and it is why the sidecar does *not* get the same pass: a sidecar is a mutable shared JSON file whose name says nothing about its contents, so two devices editing it genuinely conflict.

**The licence has an expiry condition.**
It is content-addressing, and nothing else.
The day an asset filename becomes something a user or a device chooses — `report.pdf`, `latest.png`, a UUID minted at import — last-write-wins becomes real and this RFC must be revisited rather than cited.
`is_safe_asset_name` and `claimed_hash` (`crates/outl-sync-iroh/src/engine_assets.rs`) are where that assumption is load-bearing on the wire.
The responder serves plain basenames only, and the initiator verifies received bytes against the hash the name claims.

The clearest evidence that the boundary held: `outl-core` has never heard of assets.
The `Op` enum is `Move`, `Edit`, `SetProp`, `Create`, `SetCollapsed`, `SnoozeRemind`, and `grep -ri asset crates/outl-core/src` returns nothing.
That absence is the design.

### Replication

Assets replicate like the `.md` projections, not like ops.
The `file` transport (iCloud Drive, Syncthing, a shared filesystem) carries them for free — they are ordinary files in the workspace folder.
The iroh transport ships them over a dedicated `outl-asset/1` stream, described in [`docs/sync.md` → Binary-asset transfer](../sync.md).
The protocol negotiates a manifest of the peer's asset basenames first, then pulls only what is missing.
Because names *are* hashes, "do I already have this?" is a set difference on filenames, with no version vector and no per-file metadata.

## Why not the alternatives

**Model the bytes as an `Op`** — `Op::AddAsset { bytes }`, or a base64 payload inside `Op::Edit`.
This is what invariant 7's default position literally instructs, which is exactly why it needs refuting in writing.
Refused for the three compounding reasons in **Why**: full replay on every device, no removal path in an append-only log where `Delete` is a `Move`, and the direct undoing of [RFC 0137](0137-storage-scale.md).
The asymmetry that settles it: a wrong op-log decision is permanent everywhere, and a wrong directory-layout decision is a migration.

**Put the bytes, or a base64 payload, in the `.outl` sidecar.**
The sidecar is structural matching metadata, synced as one last-write-wins blob per page (`outl-md/CLAUDE.md` → "Sidecar is not a sync surface"), and it is **rebuilt from the `.md`** whenever it looks stale.
Storing bytes in a rebuildable cache is a data-loss shape, not a storage decision.

**Inline the bytes as a `data:` URI in the `.md`.**
It keeps everything in one file and destroys the file.
Invariant 2's point is that a `.md` stays readable and editable in a plain editor, and a 5 MB single line is neither.
It also breaks matching outright: level 2 skips blocks over 4096 characters by design (`crates/outl-md/src/matching.rs`), so every asset block would fall to level 3 on any external edit.

**Reference the file where the user already keeps it** — an absolute path, or a relative path pointing outside the workspace.
Zero copies, and a dead link on the second device.
It also reintroduces the original complaint: the link breaks when the user moves the file.
The workspace stops being self-contained, which is the property the whole `file`-transport story rests on.

**Store under the original filename** — `assets/report.pdf`.
Loses dedupe, and loses the licence above, which is the real cost.
Two devices attaching different files both named `report.pdf` genuinely conflict, and a file transport resolves that by silently discarding one.
It also puts arbitrary user-chosen names on the wire, where the current guard is a plain-basename check plus a hash verification that would no longer be possible.

**A separate content store** — `iroh-blobs`, or a git-LFS-style pointer store.
Technically the closest match to the problem, and rejected on cost rather than correctness.
It adds a second persistent store and a second sync surface for something a directory of hash-named files already solves, and it gives the `file` transport nothing: iCloud and Syncthing carry a directory for free and know nothing about a blob store.
That would turn `transport = "file"` from a real opt-out into a degraded mode.

**Keep the OS-open path and never render inline.**
This was the shipped 0.9 behaviour, and [#203](https://github.com/outlmd/outl/issues/203) is why it changed: an imported Roam graph showed clickable text where the user expected to *see* the image.
Rendering is a client concern and it did not move the bytes; it is recorded here only so the two decisions are not confused with each other.

## The opposite direction

**An asset has no history and no deletion.**
This is the sharpest cost, and it is the mirror of the property that licenses the exception.
Removing the link from a block is an ordinary `Op::Edit` that converges everywhere — the reference is gone on every device.
The bytes stay in `assets/` on every device that ever pulled them.
There is no garbage collection (`#202` lists it under "consider"), so a user who attaches the wrong file — a payslip, a private scan — cannot un-attach it in any way that reaches their peers.
They have to delete the file by hand on each machine, and **nothing tells them that**.
An op log would have had the same permanence and also bloated forever, so this is not an argument for the rejected design — but it is a real loss the user can hit, and it has no report path today.

The same fact cuts the other way, stated so the trade is legible: undo of that block edit restores a working link, because nothing collected the bytes.

**The mirrored case for sync: this RFC secures the write path, and the read path has a visible gap.**
Ops are small and gossip fast; blobs ride a separate stream and can lag by minutes on a large file.
So there is a window where the link exists on a device the bytes have not reached.
The read path answers honestly rather than silently: `resolve_asset_path` returns `Ok(None)`, and both GUI clients surface "asset not found on this device yet" instead of a broken image or a dead click.
That phrasing is the decision — the user is told it is a *timing* problem, not a missing file — and `asset_pull_from_peer_without_assets_is_harmless` pins that an absent asset never escalates into a sync failure.

**What is refused, and whether the user hears about it.**
A file over `[assets] max_bytes` (default 100 MiB) is rejected *before* the copy, and the error reaches the caller — `oversize_is_rejected_before_copy`.
A link that resolves outside `assets/` returns `ActionError::InvalidAssetPath` rather than opening the file.
The read side is bounded too: `read_asset_data_url` caps at 25 MB via a structural `Take` so a large file cannot be base64'd into the webview, and refuses anything that is not a regular file, because `metadata().len()` lies for a FIFO.
In all three cases the user gets a message and their alternative is the OS-open path, which streams instead of buffering.

**What this now permits that it did not before: a peer writes bytes into the workspace directory.**
Before #202 nothing but ops and projections landed there.
`engine_assets` validates every received name as a plain basename and verifies the content against the hash the name claims, skipping a mismatch rather than failing.
`asset_manifest_drops_names_with_embedded_newline` pins the framing against a name that would smuggle a second entry.
The surface is guarded, and it is still a surface that did not exist a release ago.

**Assets are not encrypted and not access-controlled.**
An attachment in a workspace on a shared filesystem is an ordinary readable file, and hash-naming does not obscure content to anyone holding the file.
Nothing tracks this.

## How it cannot regress

1. **The rule, where an editor reads it.**
   Root `CLAUDE.md` invariant 7 is the rule this excepts.
   The exception itself lives in `outl-actions/CLAUDE.md` → "Functions never", which carries the bytes-versus-state argument inline and closes with "don't use this as precedent for a second exception without the same argument".
   That is deliberate placement: someone about to add a second filesystem escape hatch is editing that crate, not reading `docs/rfcs/`.
   `outl-md/CLAUDE.md`'s `asset.rs` row states the other half — that module never touches the filesystem.
   The user-facing disk story is [`docs/markdown-format.md` → Asset links](../markdown-format.md#asset-links-nameassetshashext).
   The wire story is [`docs/sync.md` → Binary-asset transfer](../sync.md), and the config key is [`docs/config.md` → `[assets]`](../config.md).
   This RFC is the rationale layer those five point at.

2. **Tests.**
   The content-addressing property that licenses the whole exception is pinned by `import_copies_and_content_addresses` and `identical_bytes_dedupe_to_one_path`.
   If the filename stops being the hash, they fail, and the licence in this RFC expires at the same moment.
   The traversal guards are `resolve_rejects_traversal_and_external` and `safe_asset_names_reject_traversal_and_separators`.
   The wire framing is `asset_manifest_drops_names_with_embedded_newline` and `decode_asset_manifest_rejects_short_buffer`.
   Both groups are the hardening a "simplification" would strip first.
   `assets_transfer_from_peer` is the end-to-end proof that blobs move without ops.
   The bounded / degrading read path is pinned on the client side by the three `MarkdownInline.test.tsx` cases named in **Guarded by**.

   **Named gap — the central invariant is unpinned.**
   **No test asserts that asset bytes never enter the op log.**
   It holds by construction today because the `Op` enum has no asset variant, so the compiler is the only guard, and a future `Op::AddAsset` would break nothing mechanically.
   The cheap pin does not exist yet: import a multi-MB asset, then assert the actor's `ops-<actor>.jsonl` grew by no more than the link edit.
   That is the one layer of the three-layer contract in [`docs/rfcs/README.md`](README.md) this RFC is missing, and it should be written the next time this area is touched.

## Scope

**Not covered — orphan asset garbage collection.**
No block references the file, and the bytes stay forever.
Listed under "consider" on [#202](https://github.com/outlmd/outl/issues/202), with no issue of its own.
See The opposite direction for why it is more than a disk-space concern.

**Not covered — importing another tool's attachments.**
Copying Logseq's `assets/`, resolving Obsidian vault-relative embeds, and downloading Roam's remote images are the "in progress" checklist on [#202](https://github.com/outlmd/outl/issues/202).
A referenced-but-missing file becomes an `assets_missing` warning, never a hard failure.

**Not covered — how a client renders an asset.**
Issue [#203](https://github.com/outlmd/outl/issues/203).
The `![alt](url)` token itself obeys the dialect rule in [RFC 0008](0008-markdown-dialect-and-sidecar-tokens.md).
The image / chip / TUI-placeholder split is owned by [`docs/markdown-format.md` → Asset links](../markdown-format.md#asset-links-nameassetshashext) and `crates/outl-frontend-shared/CLAUDE.md`.

**Not covered — terminal image protocols.**
Kitty and iTerm2 inline images sit on top of the TUI placeholder, out of scope in #203.

**Not covered — Android `content://` pickers, previews and thumbnails.**
Both on the [#202](https://github.com/outlmd/outl/issues/202) roadmap; only the iOS `file://` normalization shipped.

**Not covered — encryption at rest.**
No issue tracks it.
