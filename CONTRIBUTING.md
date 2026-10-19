# Contributing to outl

Thanks for wanting to help.
The short version of how we work is on this page.
Two long-form pages back this up — read both before a non-trivial PR:

> **[outl.app/docs/development](https://outl.app/docs/development)** — the engineer onramp: how to set up, run, test, debug, and ship the project.
>
> **[outl.app/docs/contributing](https://outl.app/docs/contributing)** — the rules of the game: invariants, what reviewers look at, what we won't block your PR for.

The contributing page is the same content the [Copilot reviewer](.github/copilot-instructions.md) runs against, so you won't be surprised by anything in review.

## Quick start

```bash
git clone https://github.com/outlmd/outl.git
cd outl
cargo build --workspace
cargo test --workspace
```

You need Rust 1.99+.
`rust-toolchain.toml` pins the exact version, so `rustup` will install it for you on first build.

## Layout

```
crates/
├── outl-core/      Tree CRDT, op log, storage trait — never depends on UI.
├── outl-md/        Parse/render, sidecar, matching, slugify, inline tokens.
├── outl-actions/   UI-agnostic workspace ops (shared by every client).
├── outl-cli/       The `outl` binary.
├── outl-tui/       The terminal UI.
└── outl-mobile/    Tauri 2 mobile app (iOS first).
docs/               User-facing docs; rendered at outl.app/docs.
```

Each crate has its own `CLAUDE.md` describing what it owns and the invariants.
Read it before editing that crate.

## Commit messages

[Conventional Commits](https://www.conventionalcommits.org):

```
feat(tui): visual mode yank
fix(md): preserve sidecar version field on rewrite
docs(sync): walkthrough of the cycle case
refactor(core): extract HLC generator
```

Subject under 70 chars.
Body for the *why* if it's not obvious from the diff.

## Reporting bugs / asking questions

- **Bug:** open an issue with the [bug report template](.github/ISSUE_TEMPLATE/bug_report.md).
  Include repro steps and what you expected.
- **Security issue:** see [SECURITY.md](SECURITY.md).
  Don't open a public issue.
- **Question or half-formed idea:** start a thread in [Discussions](https://github.com/outlmd/outl/discussions) or ask on [Discord](https://discord.gg/dnfAbCYWv).
- **Design discussion:** open an issue with the `discussion` label or a draft PR labeled `RFC`.

## Where to start

Issues labeled [`good first issue`](https://github.com/outlmd/outl/labels/good%20first%20issue) have a clear scope and don't need you to know the whole codebase.
[`help wanted`](https://github.com/outlmd/outl/labels/help%20wanted) are bigger, but nobody is on them yet.
Comment on the issue before you start so two people don't build the same thing.

## Code of conduct

Everyone in the project follows the [Code of Conduct](CODE_OF_CONDUCT.md).

## License

By contributing, you agree your work is licensed under the [MIT License](LICENSE) (inbound = outbound).
