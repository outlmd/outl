#!/usr/bin/env bash
# Stop hook: a change to the desktop's layout surfaces owes a layout run.
#
# ## Why a hook and not a habit
#
# The `+ prop` affordance went `hidden` -> `flex` on row hover, which
# added 18px to the block and pushed every row below it down. The text
# jumped under the pointer and flickered at row boundaries. Nothing
# failed: `tsc` was clean, 163 tests were green, and the defect is
# invisible to every one of them, because it only exists once the thing
# is painted and pointed at. It was found by a user, in a video.
#
# A battery that only runs when someone remembers would not have caught
# it either. So the trigger is the edit itself.
#
# ## Why Stop and not PostToolUse
#
# Same reason as `file-size-sweep.sh`: `PostToolUse` matches `Edit|Write`
# and the Bash tool does not match it, so a file written with `python3 -`
# or `cat >` reaches disk unseen. A Stop sweep asks "what changed on
# disk", however it got there. It also fires once per turn instead of
# once per edit, which matters for a nudge pointing at a multi-minute
# container run.
#
# ## Why it nudges instead of running
#
# The battery boots a real app in Docker. That is seconds warm and
# minutes cold, and it needs a daemon this machine may not have running.
# A Stop hook that blocks on it would hold every turn hostage to
# `docker`. It names the command and gets out of the way.
#
# ## Loop safety
#
# Exiting 2 sends Claude back to work, so an unclearable condition loops.
# `stop_hook_active` is set on re-entry: report once, then stand down.

set -uo pipefail

event_json=$(cat)
case "$event_json" in
  *'"stop_hook_active":true'*|*'"stop_hook_active": true'*) already_fired=1 ;;
  *) already_fired=0 ;;
esac

repo_root="${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel 2>/dev/null)}"
[ -n "$repo_root" ] || exit 0
cd "$repo_root" || exit 0

# The surface list lives in one file both this hook and the battery
# read (`battery/ux-surfaces.txt`), so they cannot disagree about what
# counts as a surface.
hash_script="$repo_root/.claude/skills/run-desktop/battery/surfaces-hash.sh"
list_file="$repo_root/.claude/skills/run-desktop/battery/ux-surfaces.txt"
[ -x "$hash_script" ] && [ -f "$list_file" ] || exit 0

is_ux_surface() {
  while IFS= read -r entry; do
    case "$entry" in ''|'#'*) continue ;; esac
    case "$1" in "$entry"|"$entry"/*) return 0 ;; esac
  done < "$list_file"
  return 1
}

changed=$(
  {
    git diff --name-only HEAD 2>/dev/null
    git ls-files --others --exclude-standard 2>/dev/null
  } | sort -u
)
[ -n "$changed" ] || exit 0

touched=""
while IFS= read -r f; do
  [ -n "$f" ] || continue
  is_ux_surface "$f" || continue
  [ -e "$f" ] || continue
  touched="${touched}  $f"$'\n'
done <<< "$changed"

[ -n "$touched" ] || exit 0

# Already proven against exactly these bytes? The battery stamps the
# surface hash, and only on a pass.
stamp="$repo_root/.claude/.ux-battery-stamp"
if [ -f "$stamp" ] && [ "$(cat "$stamp")" = "$(bash "$hash_script" "$repo_root")" ]; then
  exit 0
fi

if [ "$already_fired" -eq 1 ]; then
  printf 'desktop UX surfaces changed and the layout battery has not run:\n%s' "$touched" >&2
  printf 'Not blocking again. Run `/ux-desktop` when you can.\n' >&2
  exit 0
fi

printf 'These changed, and they decide what the app looks like once painted:\n\n%s\n' "$touched" >&2
printf 'Run the layout battery before reporting done:\n\n' >&2
printf '    /ux-desktop\n\n' >&2
printf 'It boots the real app in a container and points at every row to check\n' >&2
printf 'that nothing moves. `tsc` and vitest cannot see a reflow: the `+ prop`\n' >&2
printf 'affordance pushed every row below the pointer down 18px with a clean\n' >&2
printf 'typecheck and 163 green tests, and a user found it in a video.\n\n' >&2
printf 'If docker is unavailable here, say so and leave it unrun rather than\n' >&2
printf 'reporting the change as verified.\n' >&2
exit 2
