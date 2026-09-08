#!/usr/bin/env bash
# PostToolUse hook: warn (and at higher thresholds, *block*) when a
# source file grows past sensible limits.
#
# The point isn't a hard line-count cap — it's to force a conversation
# about responsibility. A 700-line file is usually a design smell
# (multiple concerns sharing a module); a 1000-line file always is.
#
# Covers Rust (.rs) and the frontend (.ts/.tsx). TypeScript was outside
# this guard until 2026-09 and the gap is why the four largest files in
# the repo are all frontend: `Journal.tsx` reached 3,212 lines — 3.5x
# the "refuse" threshold, with a single `Journal()` function spanning
# 2,654 of them — while the guard dutifully policed 400-line Rust
# modules. A limit that only applies to the language you happened to
# write it for is not a limit.
#
# Thresholds (lines of source, blank/comment included):
#   < 400     OK, no signal
#   400..600  notice (informational; printed to stderr, doesn't block)
#   600..900  warning (exit 2; reminds Claude to consider extracting)
#   >= 900    refuse-with-rationale (exit 2 + strong note; Claude must
#             propose a refactor before the next big edit)
#
# Same numbers for both languages on purpose: JSX is more verbose per
# unit of logic, but a 900-line component is exactly as hard to reason
# about as a 900-line module, and a per-language allowance would just
# encode the excuse.
#
# Reads tool_input.file_path from stdin JSON. Skips files outside
# /crates/, build output, and vendored dependencies.
#
# THIS IS ONLY HALF THE GUARD. A PostToolUse hook fires only when Claude
# Code is the editor; `scripts/check-file-size.sh` enforces the same 600
# for everyone in CI, against the frozen baseline at
# `.github/file-size-baseline.txt`. The two hold the same threshold, the
# same extensions and the same exclude list, so a change to any of them
# here needs the matching edit there.

set -uo pipefail

event_json=$(cat)

file_path=$(printf '%s' "$event_json" | sed -n 's/.*"file_path"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')

# Language label used in the messages below, and the extension filter.
case "$file_path" in
  *.rs)  lang="Rust" ;;
  *.tsx) lang="TSX" ;;
  *.ts)  lang="TypeScript" ;;
  *) exit 0 ;;
esac

case "$file_path" in
  */crates/*) ;;
  *) exit 0 ;;
esac

# Build output and vendored code — never ours to split.
case "$file_path" in
  */target/*|*/node_modules/*|*/dist/*|*/gen/*) exit 0 ;;
esac

if [ ! -f "$file_path" ]; then
  exit 0
fi

lines=$(wc -l < "$file_path" | tr -d ' ')

# Pretty filename for messages.
rel=${file_path#"${CLAUDE_PROJECT_DIR}/"}

if [ "$lines" -lt 400 ]; then
  exit 0
fi

if [ "$lines" -lt 600 ]; then
  # Informational only — no exit 2, no blocking.
  printf 'note: %s is %d lines. Watch for accumulation; extract when responsibilities diverge.\n' \
    "$rel" "$lines" >&2
  exit 0
fi

if [ "$lines" -lt 900 ]; then
  printf 'WARNING: %s is %d lines. This is past comfortable single-module size.\n' \
    "$rel" "$lines" >&2
  printf 'Consider extracting one of the concerns into a sibling module before the next big edit.\n' >&2
  printf 'See docs/architecture.md and the per-crate CLAUDE.md for the layering principle.\n' >&2
  exit 2
fi

# >= 900 — strong push.
#
# The extraction advice differs by language and nothing else does, so it is
# one variable rather than two near-identical message blocks.
#
# shellcheck disable=SC2016  # the backticks are markdown, not substitution —
# single quotes are exactly what keeps them literal.
if [ "$lang" = "Rust" ]; then
  extract_advice='  2. Extract each into a sibling module (`mod x;` in the crate).
  3. Re-export from the parent only the names other modules need.'
else
  extract_advice='  2. Extract each into a sibling module or component file.
     Pure helpers that both clients could use belong in
     `crates/outl-frontend-shared/` — check it first, since a copy here
     is the parallel-implementation bug the reuse-first rule prevents.
  3. Export from the parent only the names other modules need.'
fi

cat >&2 <<MSG
STOP: $rel is $lines lines. A $lang file that big has accreted multiple
concerns and is hard to read, hard to test, and hard to evolve.

Before the next non-trivial edit to this file:
  1. Identify 2-4 distinct responsibilities inside it (state, IO,
     rendering, key handling, AST helpers, ...).
$extract_advice

Invoke the \`refactor-architect\` agent (.claude/agents/) to propose
a concrete split, or stop and ask the user for guidance.
MSG
exit 2
