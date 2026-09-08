---
description: Runs fmt + clippy + test + doc (Rust) and vitest + tsc (TS) on the whole workspace. Use before reporting done.
allowed-tools: Bash(cargo fmt:*), Bash(cargo clippy:*), Bash(cargo test:*), Bash(cargo build:*), Bash(cargo doc:*), Bash(RUSTDOCFLAGS=*:*), Bash(bun run:*), Bash(bun install:*)
---

Run in sequence and report the result of each step:

1. `cargo fmt --all -- --check` — formatting
2. `cargo clippy --workspace --all-targets -- -D warnings` — lints
3. `cargo test --workspace --all-targets` — tests
4. `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` — docs (CI runs this; breaks on intra-doc links to private items, e.g. ``[`Foo`]`` where `Foo` is `pub(crate)`.
   Drop the brackets: `` `Foo` ``.)
5. `bun run test` — vitest across every package declaring the script (`@outl/shared`, `outl-desktop`, `outl-mobile`)
6. `bun run typecheck` — `tsc --noEmit` across every package declaring it, which is those three plus the `examples/*` plugins (14 today)

Steps 5 and 6 are not optional extras: roughly a third of every GUI client is TypeScript, and several Rust invariants are enforced *only* on the TS side.
`shortcuts.support.test.ts` is the desktop half of invariant 12 (a catalog that promises a chord no handler implements), and the theme suites are the client half of invariant 13.
Reporting "done" on a Rust-only run leaves those unverified.

Both are fast — the full vitest fan-out is ~4s — so there is no reason to skip them.
If `bun` is missing, say so explicitly rather than silently reporting a Rust-only pass as a full one.

If any step fails, **stop** and show the exact output.
Do not attempt to fix automatically — only report.

Output format:

```
fmt:        PASS | FAIL (N files)
clippy:     PASS | FAIL (N warnings)
test:       PASS | FAIL (N failures)
doc:        PASS | FAIL (N warnings)
vitest:     PASS | FAIL (N failures)
typecheck:  PASS | FAIL (N errors)

[failure details, if any]
```
