#!/usr/bin/env bash
# PostToolUse hook: catch code written in the place a cross-client
# surface used to live, after that surface got a single declaration.
#
# Three surfaces in this repo are declared once and consumed everywhere.
# Each replaced N hand-maintained copies, and each has a test that fails
# when a client drifts. The problem this hook solves is different: a
# model (or a person) working from the shape of the *old* code writes the
# hand-rolled version again, because that is what the git history is
# full of. The test catches it in CI; this catches it at the keystroke,
# with the file the change actually belongs in.
#
#   1. Tauri command wrappers
#      → outl-tauri-shared/src/wrappers/catalog.rs
#      A `#[tauri::command]` in a client crate is fine only for the
#      handful that need more than `State<'_, AppState>` (an AppHandle,
#      a second State). Everything else is generated.
#
#   2. The post-mutation commit sequence
#      → outl_actions::commit_page
#      A page mutation followed by a bare
#      `apply_page_md_with_sidecar_guarded` is step 5 of five.
#
#   3. The Rust ↔ TypeScript wire contract
#      → outl-tauri-shared/tests/wire_types.rs
#      A new `export interface` in types.ts with no pin is a DTO nobody
#      is watching.
#
# Soft signal (exit 2 with a message). The edit already happened; this
# nudges toward the right file rather than undoing anything. Every check
# is deliberately narrow — a false positive here trains the reader to
# ignore the hook, which costs more than the miss it prevents.
#
# Reads tool_input.file_path from stdin JSON.

set -uo pipefail

event_json=$(cat)
file_path=$(printf '%s' "$event_json" | sed -n 's/.*"file_path"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')

[ -n "$file_path" ] || exit 0
[ -f "$file_path" ] || exit 0

# ---------------------------------------------------------------------
# 1. A hand-written #[tauri::command] whose body already exists shared.
#
# The signal is not "this file has a #[tauri::command]" — several
# legitimately do. It is "this file hand-writes a command that
# outl-tauri-shared already has a body for", which is precisely the
# duplicate the catalog exists to remove.
#
# `workspace_stats` / `reload_workspace` are the counter-example and the
# reason the check is shaped this way: they are hand-written on both
# clients, they differ (the desktop can swap workspaces at runtime), and
# they have no shared body. Nothing to route.
# ---------------------------------------------------------------------
case "$file_path" in
  */outl-desktop/src-tauri/src/commands/*.rs|*/outl-mobile/src-tauri/src/commands/*.rs)
    case "$file_path" in */mod.rs) exit 0 ;; esac

    repo_root="${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel 2>/dev/null)}"
    shared_dir="${repo_root}/crates/outl-tauri-shared/src/commands"
    [ -d "$shared_dir" ] || exit 0

    # One record per command: "name<TAB>full signature". The signature is
    # what says whether the wrapper is a legitimate exception, so it has
    # to be read per function — `open_ref` needs an `AppHandle` and lives
    # in the same file as commands that do not, and a file-level check
    # would let the exception mask a real duplicate.
    duplicated=()
    while IFS=$'\t' read -r name sig; do
      [ -n "$name" ] || continue
      # Needs more from Tauri than State<'_, AppState>: keep it.
      case "$sig" in
        *AppHandle*|*PluginService*|*"Window"*) continue ;;
      esac
      # Two `State<` arguments means a second managed type.
      states=$(printf '%s' "$sig" | grep -o 'State<' | wc -l | tr -d ' ')
      [ "$states" -gt 1 ] && continue
      if grep -rqE "^pub (async )?fn ${name}\b" "$shared_dir" 2>/dev/null; then
        duplicated+=("$name")
      fi
    done < <(awk '
      /#\[tauri::command\]/ { c=1; sig=""; next }
      c {
        sig = sig " " $0
        # Keep accumulating until the argument list actually closes.
        # `pub(crate) fn foo(` contains a ")" of its own, so a bare
        # paren test stops one line in and never sees the arguments.
        if (sig ~ /\)[ \t]*(->|\{)/) {
          n = sig; sub(/.*fn /, "", n); sub(/[(<].*/, "", n)
          gsub(/\t/, " ", sig)
          print n "\t" sig
          c = 0
        }
      }' "$file_path")

    if [ ${#duplicated[@]} -gt 0 ]; then
      cat >&2 <<EOF
NOTE: $(basename "$file_path") hand-writes ${duplicated[*]}, which
outl-tauri-shared already has a body for.

The Tauri command surface is declared once, in
crates/outl-tauri-shared/src/wrappers/catalog.rs, and each client's
commands/<module>.rs is a single macro invocation:

    outl_tauri_shared::<module>_commands!(crate::state::AppState);

To add a command: body in outl-tauri-shared/src/commands/, one entry in
the matching *_commands! list, then an invoke_handler! entry in BOTH
clients. Shared bodies take String, not &str.

Hand-write a wrapper only when it needs more than State<'_, AppState> —
an AppHandle to emit an event, a second State for the plugin thread.
Those wrappers call the shared body with extra arguments, so they will
trip this note; keep them, and ignore it.

Root CLAUDE.md -> Anti-patterns; outl-tauri-shared/CLAUDE.md ->
"One command surface, not two".
EOF
      exit 2
    fi
    ;;
esac

# ---------------------------------------------------------------------
# 2. A mutation projected by hand instead of through the pipeline.
# ---------------------------------------------------------------------
case "$file_path" in
  *.rs)
    if grep -q 'apply_page_md_with_sidecar_guarded' "$file_path"; then
      # The owners of that call are allowed to make it: outl-actions
      # itself, and the projection worker. Everyone else is a caller.
      case "$file_path" in
        */outl-actions/src/*|*/outl-tauri-shared/src/*) ;;
        *tests*|*/tests/*) ;;
        # Known-pending call sites, frozen so the hook is quiet on
        # existing debt and loud on new code — the same shape as the
        # file-size ratchet. These are the CLI and TUI callers issue
        # #264's definition of done names: each takes step 5 alone and
        # none of them wrote down which of the other four it skips.
        #
        # This list may only ever get shorter. Migrating a file to
        # `commit_page` means deleting its row, not editing it.
        */outl-cli/src/cmd/page.rs) ;;
        */outl-cli/src/cmd/prop.rs) ;;
        */outl-cli/src/cmd/asset.rs) ;;
        */outl-cli/src/cmd/block.rs) ;;
        */outl-cli/src/cmd/daily.rs) ;;
        */outl-cli/src/cmd/template.rs) ;;
        */outl-tui/src/actions/exec.rs) ;;
        */outl-tui/src/actions/autocomplete.rs) ;;
        */outl-tui/src/actions/block/template.rs) ;;
        *)
          # Only speak up when the same file also mutates, which is what
          # makes the other four steps apply. A read-only re-projection
          # (doctor, reconcile, import) legitimately wants step 5 alone.
          if grep -qE '\b(edit_text|append_block|split_block|indent|outdent|move_after|move_up|move_down|toggle_todo|toggle_quote|set_property|delete)\(' "$file_path"; then
            cat >&2 <<EOF
NOTE: $(basename "$file_path") mutates a page and calls
apply_page_md_with_sidecar_guarded directly.

That is step 5 of the five-step commit sequence. The other four —
pre-mutation undo snapshot, backlink-index invalidation, peer announce,
and the projection itself — live in outl_actions::commit_page, which
takes &mut Workspace and a CommitHooks impl (only \`project\` is
required; the rest default to no-ops).

If you are deliberately skipping the others, say which and why in a
comment next to the call. Repo-wide there were 34 such calls across 18
files, each one a caller that decided the rest did not apply, and none
of those decisions were written down.

outl-actions/CLAUDE.md → "The commit pipeline"; issue #264.
EOF
            exit 2
          fi
          ;;
      esac
    fi
    ;;
esac

# ---------------------------------------------------------------------
# 3. A wire interface with nobody watching it.
# ---------------------------------------------------------------------
case "$file_path" in
  */outl-frontend-shared/src/api/types.ts)
    repo_root="${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel 2>/dev/null)}"
    pins="${repo_root}/crates/outl-tauri-shared/tests/wire_types.rs"
    [ -f "$pins" ] || exit 0

    unpinned=()
    while IFS= read -r name; do
      grep -q "\"${name}\"" "$pins" || unpinned+=("$name")
    done < <(sed -n 's/^export interface \([A-Za-z_][A-Za-z0-9_]*\).*/\1/p' "$file_path")

    if [ ${#unpinned[@]} -gt 0 ]; then
      cat >&2 <<EOF
NOTE: types.ts declares interfaces with no pin: ${unpinned[*]}

The Rust ↔ TypeScript wire contract is hand-written on purpose ("never a
generator", types.ts line 1). What makes that safe is
crates/outl-tauri-shared/tests/wire_types.rs, which serializes a real
Rust value and compares its JSON keys against the interface.

Add an assert_wire_shape case, or an UNPINNED row with a reason. An
unpinned DTO drifts silently: the field arrives as \`undefined\` at
runtime, on a device, with both languages compiling clean.

That test found ReminderDto.plain_text on its first run.
EOF
      exit 2
    fi
    ;;
esac

exit 0
