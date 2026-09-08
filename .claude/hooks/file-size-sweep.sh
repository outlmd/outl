#!/usr/bin/env bash
# Stop hook: run the CI file-size ratchet once at the end of a turn.
#
# ## Why this exists when file-size-guard.sh already runs
#
# `file-size-guard.sh` is a PostToolUse hook on `Edit|Write`. That matcher is
# the whole problem: **the Bash tool does not match it**. A file created or
# rewritten with `python3 - <<'PY'`, `sed -i`, `cat > file`, `git mv` or any
# other shell path never passes a single guard in `.claude/hooks/`.
#
# That is not a hypothetical. The change that introduced this hook created
# `Journal.context-actions.ts` (300 lines), `JournalHeader.tsx` (113),
# `scripts/check-file-size.sh` (153) and `crates/outl-md/tests/lang.rs` (182)
# entirely through the Bash tool. None of them were seen by the size guard,
# the semantic-linebreak guard, or the doc-sync guard.
#
# So the per-file hook covers "Claude used Edit/Write", and this covers
# "something changed on disk, however it got there". Different questions.
#
# ## Why a full sweep is affordable here
#
# `scripts/check-file-size.sh` walks `crates/` and compares against the frozen
# baseline in ~0.6s. That is fine once per turn; it would not be fine on every
# Edit, which is why this is a Stop hook and not a second PostToolUse entry.
#
# ## Loop safety
#
# A Stop hook that exits 2 sends Claude back to work. If the condition cannot
# be cleared, that loops. Claude Code sets `stop_hook_active: true` on the
# payload when it is re-entering after a Stop hook already fired, so this
# reports and gets out of the way on the second pass rather than blocking
# again. The finding still reaches the user through the printed message.

# shellcheck disable=SC2016  # backticks throughout the message text are
# markdown, not command substitution; single quotes keep them literal.
set -uo pipefail

event_json=$(cat)

# Re-entry after this hook already fired once: report, do not block again.
case "$event_json" in
  *'"stop_hook_active":true'*|*'"stop_hook_active": true'*) already_fired=1 ;;
  *) already_fired=0 ;;
esac

repo_root="${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel 2>/dev/null)}"
script="${repo_root}/scripts/check-file-size.sh"

# Nothing to enforce if the script is absent (older checkout, or the repo root
# could not be resolved). Never fail a turn over the guard's own absence.
[ -x "$script" ] || exit 0

if output=$("$script" 2>&1); then
  exit 0
fi

if [ "$already_fired" -eq 1 ]; then
  printf 'file-size ratchet still failing (not blocking again):\n%s\n' "$output" >&2
  exit 0
fi

printf 'The file-size ratchet failed:\n\n%s\n\n' "$output" >&2
printf 'This ran as a Stop hook because the per-edit guard only sees Edit/Write.\n' >&2
printf 'A file written through the Bash tool reaches disk without passing it, so\n' >&2
printf 'this sweep is what catches it.\n\n' >&2
printf 'Either split the file (the `refactor-architect` agent proposes a split), or,\n' >&2
printf 'if a recorded file legitimately shrank, re-record with\n' >&2
printf '`scripts/check-file-size.sh --update`.\n' >&2
exit 2
