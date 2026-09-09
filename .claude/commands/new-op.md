---
description: Guide for adding a new variant to the Op enum (e.g. Op::Tag, Op::Link). Covers every place that needs to change.
argument-hint: <VariantName>
---

You want to add `Op::$1` to the outl tree CRDT.
MANDATORY checklist:

## 1. Define the variant in `crates/outl-core/src/op.rs`

```rust
$1 {
    node: NodeId,
    // ... op fields
    // CRITICAL: include `old_*` fields for undo.
    // Without old_*, undo_op cannot revert.
}
```

## 2. Implement `do_op` in `crates/outl-core/src/tree.rs`

- Fill `old_*` in the `LogOp` before applying the mutation.
- If the op can violate an invariant (cycle, etc), check first and treat as a no-op on materialization (but the op still goes to the log).

## 3. Implement `undo_op`

- Revert using the `old_*` fields of the `LogOp`.
- Idempotency: undo of an op that was never applied must be a no-op.

## 4. Update serialization

- Verify that serde derive already covers it.
  If there's a binary field, ensure base64 or bincode.
- Add conversion to the SQLite schema in `storage/sqlite.rs` if the op has extra fields.

## 5. Mandatory tests (in `crates/outl-core/tests/`)

- Convergence: 3 replicas apply the op in different orders → same final state.
- Idempotency: applying 2x = applying 1x.
- Reordering: a late-arriving op forces a correct undo/replay.
- Interaction with `Move`: an op concurrent with a Move on the same node converges.

## 6. Document in `docs/crdt.md`

- Add a paragraph in the "Operations" section describing semantics.
- Add a concurrent example if the op has non-obvious interactions.

## 7. Reaching the clients

An `Op` a user can trigger needs a way to trigger it, and that path is declared, not hand-rolled:

- **Action + capability.** Add the `Action` to `outl-shortcuts`; `support()` is an exhaustive `match`, so it will not compile until all three clients state what they do with it (root `CLAUDE.md` invariant 12).
- **The command.** Body in `outl-tauri-shared/src/commands/`, one entry in the matching `*_commands!` list in `src/wrappers/catalog.rs`, then an `invoke_handler!` entry in **both** clients.
  Never hand-write the `#[tauri::command]` wrapper.
  Shared bodies take `String`, not `&str`.
- **The mutation.** Route it through `outl_actions::commit_page` so the undo snapshot, backlink invalidation, peer announce and projection all happen, in order.
- **The wire shape.** If the reply carries a new DTO, mirror it in `crates/outl-frontend-shared/src/api/types.ts` and pin it in `outl-tauri-shared/tests/wire_types.rs`.
- **The TUI.** It still derives ops from rendered markdown ([issue 263](https://github.com/outlmd/outl/issues/263)), so a new op reaches it through `outl-actions`, not through `save_page_with`.

Skipping any of these compiles fine and ships an op no client can produce.

## 8. Pre-flight before PR

- [ ] `cargo fmt`
- [ ] `cargo clippy -- -D warnings`
- [ ] `cargo test -p outl-core`
- [ ] 100% coverage on the new branches in `do_op`/`undo_op`
- [ ] Invoke the `crdt-invariant-checker` agent
- [ ] Invoke the `paper-verifier` agent if the op has an analog in the paper
- [ ] `cargo test -p outl-tauri-shared` — the command-parity and wire-type pins

**Do not skip steps.** A new op that breaks convergence destroys trust in outl.
