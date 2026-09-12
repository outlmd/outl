## What this PR does

One paragraph.
The *why* first, then the *what*.

## How to verify

```bash
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Plus any feature-specific checks (manual smoke, fixture files, screenshot of TUI state, ...).

## Related issues / docs

Closes #...
Related to #...
Updated docs: `docs/...`

## Anything reviewers should look at carefully

- Is there a CRDT-correctness implication?
  Did `crdt-invariant-checker` pass?
- Is there a markdown-format implication?
  Did `markdown-roundtrip-tester` pass?
- New public API on `outl-core` / `outl-md`?
  Is it documented in the per-crate CLAUDE.md?
- Any change to keymaps in `outl-tui`?
  Updated `docs/tui.md` and the in-app help popup?
- Any change to a wire contract?
  The MCP `tools/call` result shape, the CLI `--json` envelope, a sidecar field, the op log format.
  These are visible to clients you cannot see, so say what breaks and add a `CHANGELOG.md` entry.
- Does this need an RFC?
  Yes if it touches an invariant, a data format, the CRDT, sync, or a projection path, or if it has a trade-off someone could reasonably want to reverse.
  Rule of thumb from [`docs/rfcs/README.md`](../docs/rfcs/README.md): if you are writing a paragraph here explaining *why this way*, that paragraph is an RFC.

## Out of scope for this PR

What this PR is *not* doing, even if related.
Helps reviewers not nudge for scope creep.
