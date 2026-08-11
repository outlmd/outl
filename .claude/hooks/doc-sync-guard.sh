#!/usr/bin/env bash
# PostToolUse: when a code edit lands without updating the
# documentation that describes that surface, warn so Claude treats
# doc maintenance as part of the same change — not a separate chore
# the user has to remember.
#
# Rationale: CLAUDE.md (root + per-crate), .github/copilot-instructions.md,
# and docs/*.md are how the next contributor (human or LLM) learns
# this codebase. Code drift without doc drift is how the next PR
# arrives reimplementing something that just shipped. The
# `paste::normalize_external_syntax` duplication in PR #47 is the
# canonical incident.
#
# Three rules, in order of severity:
#
#   1. CATALOG. A new top-level `pub fn|struct|enum|const` in any
#      shared crate (outl-core / outl-md / outl-actions) must appear
#      by name in the Shared primitives catalog. The symbol is the
#      canonical reuse handle for the workspace.
#      The catalog is one logical document split across four files:
#      docs/shared-primitives.md is the hub/index and carries no rows;
#      the rows live in docs/primitives-core.md,
#      docs/primitives-markdown.md and docs/primitives-actions.md.
#      This rule greps ALL of them (glob: docs/primitives-*.md), so a
#      future part lands without touching this hook.
#
#   2. PER-CRATE. Any non-test edit in `crates/<crate>/src/` should
#      reflect in `crates/<crate>/CLAUDE.md` when it touches the
#      public surface (the edit adds `pub`, or it's a >20-line block).
#      Internal refactors with no public-surface change pass silently.
#
#   3. HIGH-LEVEL DOCS. Specific source files map to specific
#      `docs/*.md` (op log → crdt.md, sidecar → markdown-format.md,
#      TUI keymap → tui.md, CLI cmd → cli.md, MCP → mcp.md, storage →
#      storage.md). An edit in one of those files should bring its
#      doc along.
#
# Non-blocking: exit 2 with a structured message. Claude reads it and
# either updates the docs in the same response, or replies confirming
# the edit is internal-only.
#
# Reads tool_input.file_path from stdin JSON.

set -uo pipefail

event_json=$(cat)
file_path=$(printf '%s' "$event_json" | sed -n 's/.*"file_path"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')

[ -z "$file_path" ] && exit 0
# Accept `.rs` (catalog + per-crate + high-level mappings all apply)
# plus `.ts`/`.tsx` for the desktop/mobile frontend shortcut wiring
# — keybinding changes can land entirely on the JS side (e.g. a new
# handler in `action-handlers.ts`) and would still leave the desktop
# CLAUDE.md shortcut table out of sync.
case "$file_path" in *.rs|*.ts|*.tsx) ;; *) exit 0 ;; esac

repo_root="${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel 2>/dev/null)}"
[ -z "$repo_root" ] && exit 0
[ ! -d "$repo_root" ] && exit 0

rel=${file_path#"${repo_root}/"}

# Skip tests, target/, examples/, build scripts.
case "$rel" in
  */tests/*|*_tests.rs|*_test.rs) exit 0 ;;
  */target/*|*/examples/*|build.rs) exit 0 ;;
esac

# Only act on edits inside crates/<crate>/src/.
case "$rel" in
  crates/*/src/*) ;;
  *) exit 0 ;;
esac

crate_dir=$(printf '%s' "$rel" | sed -E 's|^(crates/[^/]+)/.*|\1|')

# --------------------------------------------------------------------
# Diff inspection helpers.
# --------------------------------------------------------------------

# True if `file` has working-tree changes vs HEAD (or is untracked).
touched() {
  local f=$1
  if ! git ls-files --error-unmatch -- "$f" >/dev/null 2>&1; then
    [ -f "$repo_root/$f" ] && return 0 || return 1
  fi
  ! git diff --quiet --no-color -- "$f" 2>/dev/null
}

# Count of `+pub fn|struct|enum|const ...` lines added in the working
# tree diff of $file_path. Used by rule 1 + rule 2.
new_pub_count() {
  local f=$1
  if git ls-files --error-unmatch -- "$f" >/dev/null 2>&1; then
    git diff --no-color -U0 -- "$f" 2>/dev/null \
      | grep -cE '^\+pub (fn|struct|enum|const) [A-Za-z_]' || true
  else
    # New file: count its public symbols straight from the file.
    grep -cE '^pub (fn|struct|enum|const) [A-Za-z_]' "$f" 2>/dev/null || true
  fi
}

# Total lines added by the working tree diff (excluding context).
added_line_count() {
  local f=$1
  git ls-files --error-unmatch -- "$f" >/dev/null 2>&1 || {
    wc -l < "$f" 2>/dev/null | tr -d ' '
    return
  }
  git diff --no-color -U0 -- "$f" 2>/dev/null \
    | grep -cE '^\+[^+]' || true
}

# New symbol names extracted from the working tree diff.
new_symbols_of() {
  local f=$1
  local input
  if git ls-files --error-unmatch -- "$f" >/dev/null 2>&1; then
    input=$(git diff --no-color -U0 -- "$f" 2>/dev/null)
  else
    input=$(sed 's/^/+/' "$f" 2>/dev/null)
  fi
  printf '%s\n' "$input" \
    | grep -E '^\+pub (fn|struct|enum|const) [A-Za-z_][A-Za-z0-9_]*' \
    | sed -E 's/^\+pub (fn|struct|enum|const) ([A-Za-z_][A-Za-z0-9_]*).*/\2/' \
    | grep -vxE 'new|default|from|with_state' \
    | sort -u
}

# --------------------------------------------------------------------
# Findings collector — one message at the end.
# --------------------------------------------------------------------

warnings=()

# --------------------------------------------------------------------
# Rule 1 — Shared primitives catalog.
# --------------------------------------------------------------------

case "$crate_dir" in
  crates/outl-core|crates/outl-md|crates/outl-actions)
    new_syms=$(new_symbols_of "$file_path")
    if [ -n "$new_syms" ]; then
      # The catalog is split across a hub + N parts. Grep every part
      # that exists; a symbol documented in any of them counts.
      catalog_files=()
      for c in "$repo_root"/docs/shared-primitives.md "$repo_root"/docs/primitives-*.md; do
        [ -f "$c" ] && catalog_files+=("$c")
      done
      missing=()
      if [ ${#catalog_files[@]} -gt 0 ]; then
        while IFS= read -r sym; do
          [ -z "$sym" ] && continue
          grep -qE "\b${sym}\b" "${catalog_files[@]}" 2>/dev/null || missing+=("$sym")
        done <<< "$new_syms"
      fi
      if [ ${#missing[@]} -gt 0 ]; then
        msg="rule 1 — Shared primitives catalog: %s added new public symbol(s) missing from the catalog:\n"
        for sym in "${missing[@]}"; do
          msg+="    - pub ${sym}\n"
        done
        msg+="  fix: add an entry under the matching sub-table in the part that owns the concept —\n"
        msg+="       docs/primitives-core.md (op log / tree / sync / storage / backups),\n"
        msg+="       docs/primitives-markdown.md (parse / render / sidecar / index / inline / assets),\n"
        msg+="       docs/primitives-actions.md (block + page mutations, backlinks, exec, templates, reminders)\n"
        msg+="       AND .github/instructions/shared-primitives.instructions.md.\n"
        msg+="       (docs/shared-primitives.md is the index — it carries links, not rows.)\n"
        msg+="       (catalog-sync-guard.sh verifies the catalog and its mirror stay in sync.)"
        warnings+=("$(printf "$msg" "$rel")")
      fi
    fi
    ;;
esac

# --------------------------------------------------------------------
# Rule 2 — per-crate CLAUDE.md.
# --------------------------------------------------------------------

crate_claude="$crate_dir/CLAUDE.md"
if [ -f "$repo_root/$crate_claude" ]; then
  new_pub=$(new_pub_count "$file_path")
  added=$(added_line_count "$file_path")
  significant=0
  [ "${new_pub:-0}" -gt 0 ] && significant=1
  [ "${added:-0}" -gt 20 ] && significant=1
  if [ "$significant" = "1" ] && ! touched "$crate_claude"; then
    msg="rule 2 — per-crate doc: %s changed (new pub: ${new_pub:-0}, lines added: ${added:-0}) but ${crate_claude} has no matching change.\n"
    msg+="  fix: update the 'Public surface' table / 'What this crate owns' list / invariants section in ${crate_claude} to reflect the new behavior.\n"
    msg+="       if the change is genuinely internal-only (private helper rename, internal refactor), say so explicitly and continue."
    warnings+=("$(printf "$msg" "$rel")")
  fi
fi

# --------------------------------------------------------------------
# Rule 3 — high-level docs.
# --------------------------------------------------------------------

docs_to_check=()

case "$rel" in
  crates/outl-core/src/op.rs|crates/outl-core/src/tree/*.rs|crates/outl-core/src/log.rs)
    docs_to_check+=("docs/crdt.md")
    ;;
esac
case "$rel" in
  crates/outl-core/src/storage/*|crates/outl-core/src/storage.rs)
    docs_to_check+=("docs/storage.md")
    ;;
esac
case "$rel" in
  crates/outl-md/src/sidecar.rs|crates/outl-md/src/parse.rs|crates/outl-md/src/render.rs|crates/outl-md/src/inline.rs)
    docs_to_check+=("docs/markdown-format.md")
    ;;
esac
case "$rel" in
  crates/outl-tui/src/keymap*.rs|crates/outl-tui/src/actions/*|crates/outl-tui/src/modes/*)
    docs_to_check+=("docs/tui.md")
    ;;
esac
case "$rel" in
  crates/outl-tui/src/theme*.rs)
    docs_to_check+=("docs/theming.md")
    ;;
esac
case "$rel" in
  crates/outl-cli/src/cmd/*.rs|crates/outl-cli/src/output.rs)
    docs_to_check+=("docs/cli.md")
    ;;
esac
case "$rel" in
  crates/outl-cli/src/mcp/*.rs)
    docs_to_check+=("docs/mcp.md")
    ;;
esac
case "$rel" in
  crates/outl-actions/src/sync.rs)
    docs_to_check+=("docs/sync.md")
    ;;
esac
case "$rel" in
  crates/outl-actions/src/*)
    docs_to_check+=("docs/clients.md")
    ;;
esac
# Shortcut catalog: every binding edit lands on at least three
# user-facing surfaces (the catalog crate's own doc, the desktop
# client's help table, the TUI doc with its parallel keymap). We
# bypass the ≥10-line threshold further down because a one-line
# `Binding::new` swap is exactly the kind of change that ships
# without doc updates if we let it slide.
shortcut_change=0
case "$rel" in
  crates/outl-shortcuts/src/defaults.rs|crates/outl-shortcuts/src/action.rs)
    docs_to_check+=("crates/outl-shortcuts/CLAUDE.md")
    docs_to_check+=("crates/outl-desktop/CLAUDE.md")
    docs_to_check+=("docs/tui.md")
    shortcut_change=1
    ;;
esac
case "$rel" in
  crates/outl-tui/src/input/*)
    docs_to_check+=("docs/tui.md")
    docs_to_check+=("crates/outl-shortcuts/CLAUDE.md")
    shortcut_change=1
    ;;
esac
# Frontend wiring for the desktop shortcut dispatcher + action
# handlers. Catches `Cmd+T` swaps that live entirely in JS or in
# the per-block textarea `onKeyDown` (the `Cmd+Enter` race we just
# undid is the canonical incident).
case "$rel" in
  crates/outl-desktop/src/lib/shortcuts.ts \
  | crates/outl-desktop/src/lib/action-handlers.ts \
  | crates/outl-desktop/src/components/BlockRow.tsx)
    docs_to_check+=("crates/outl-desktop/CLAUDE.md")
    docs_to_check+=("crates/outl-shortcuts/CLAUDE.md")
    shortcut_change=1
    ;;
esac

# Dedupe and filter to existing + untouched docs.
if [ ${#docs_to_check[@]} -gt 0 ]; then
  uniq_docs=$(printf '%s\n' "${docs_to_check[@]}" | sort -u)
  stale=()
  added=$(added_line_count "$file_path")
  # Rule 3 normally requires ≥10 added lines (avoid pestering on
  # tiny refactors). Shortcut / binding edits bypass the gate: a
  # single-line chord swap is the most likely change to ship without
  # the user-facing tables being updated, which is exactly what we
  # learned by missing the `Cmd+T` → `Cmd+J` swap in the last
  # round-trip.
  threshold_met=0
  [ "${added:-0}" -ge 10 ] && threshold_met=1
  [ "$shortcut_change" = "1" ] && threshold_met=1
  if [ "$threshold_met" = "1" ]; then
    while IFS= read -r doc; do
      [ -z "$doc" ] && continue
      [ ! -f "$repo_root/$doc" ] && continue
      touched "$doc" || stale+=("$doc")
    done <<< "$uniq_docs"
    if [ ${#stale[@]} -gt 0 ]; then
      msg="rule 3 — high-level docs: %s changed (lines added: ${added}) but its user-facing doc has no matching change:\n"
      for doc in "${stale[@]}"; do
        msg+="    - ${doc}\n"
      done
      msg+="  fix: update the listed doc(s) to reflect the new behavior, OR confirm explicitly that this change is invisible to readers of those docs (internal refactor)."
      warnings+=("$(printf "$msg" "$rel")")
    fi
  fi
fi

# --------------------------------------------------------------------
# Rule 4 — doc examples rustdoc never compiles.
# --------------------------------------------------------------------
#
# `cargo test --doc` compiles every doc example EXCEPT ```ignore and
# ```compile_fail. Those blocks can reference symbols that no longer
# exist, or teach a recipe the codebase just replaced, and nothing in
# CI notices. The whole repo has 3 of them today, so this rule is
# cheap and its blast radius is tiny.
#
# The incident: outl-sync-iroh's `//! ## Quick start` kept teaching
# the 4-step "assemble identity + peers + relay by hand" recipe after
# `build_default_transport` became the one owner. Every symbol in the
# stale example still existed and was still `pub`, so no
# symbol-existence check could have caught it — 4b is the sub-rule
# that would have.
#
#   4a  DANGLING SYMBOL. A reference the example itself declares as
#       coming from this workspace (`use outl_x::{A, b}`, `outl_x::A`,
#       or `Type::method(` where `Type` is defined here) that no longer
#       resolves.
#   4b  STALE RECIPE. The file's module-level (`//!`) example is
#       unchecked, the diff moved the crate's public surface, and the
#       diff touched no doc-comment line at all — so nobody re-read the
#       recipe rustdoc cannot check.
#   4c  DELETED SYMBOL STILL DOCUMENTED. A `pub` item the diff removed
#       that exists nowhere in the workspace anymore but is still named
#       inside a doc comment somewhere in the repo.
#
# Deliberately NOT checked (each would cost false positives worth more
# than the coverage): bare calls `foo(`, method calls on a local
# `recv.method(`, `std::`/third-party paths, enum variants, and a
# method that vanished from its own type but survives elsewhere in the
# owning crate (a blanket/derive impl could legitimately supply it).

ws_grep() {  # $1 = grep flags, $2 = ERE
  [ -d "$repo_root/crates" ] || return 1
  grep -r $1 -E --include='*.rs' \
    --exclude-dir=target --exclude-dir=node_modules \
    "$2" "$repo_root/crates" 2>/dev/null
}

# Filter a newline-separated list of names down to the ones the workspace
# defines nowhere. One grep for the whole batch (a per-symbol sweep cost
# ~200ms each and the examples name half a dozen).
#
# Kind is deliberately not tracked: the only question asked here is "does
# this name still exist at all". A fn that became a type is a refactor,
# not a dangling doc reference.
undefined_names() {
  local names alt defined kinds
  names=$(grep -E '^[A-Za-z_][A-Za-z0-9_]*$' <<< "${1:-}" | sort -u)
  [ -z "$names" ] && return 0
  alt=$(tr '\n' '|' <<< "$names" | sed 's/|$//')
  kinds='(async[[:space:]]+)?fn|struct|enum|trait|type|union|const|static|mod|macro_rules!'
  defined=$(ws_grep -ho \
    "^[[:space:]]*(pub(\([^)]*\))?[[:space:]]+)?(${kinds})[[:space:]]+(${alt})\b" \
    | sed -E 's/.*[[:space:]]//' | sort -u)
  comm -23 <(printf '%s\n' "$names") <(printf '%s\n' "$defined")
}

# File declaring `struct|enum|trait|type NAME`. Only reached for a type the
# batch above already confirmed exists, so it runs at most a couple of times.
type_file() {
  ws_grep -l \
    "^[[:space:]]*(pub(\([^)]*\))?[[:space:]]+)?(struct|enum|trait|type|union)[[:space:]]+$1\b" \
    | head -1
}

# Body of every ```ignore / ```compile_fail doc block in $1, comment
# markers stripped. $2 = "module" restricts to `//!` blocks.
unchecked_examples() {
  awk -v only_mod="${2:-}" '
    BEGIN { marker = (only_mod == "module") ? "//!" : "//[/!]" }
    $0 ~ ("^[[:space:]]*" marker "[[:space:]]*```") {
      if (!inblk) {
        lang = $0; sub(/.*```/, "", lang); gsub(/[[:space:],]/, "", lang)
        if (lang == "ignore" || lang == "compile_fail") inblk = 1
        next
      }
      inblk = 0; next
    }
    inblk {
      line = $0
      sub(/^[[:space:]]*\/\/[\/!][[:space:]]?/, "", line)
      print line
    }
  ' "$1"
}

dangling=()
stale_recipe=0
deleted_documented=()

example_body=$(unchecked_examples "$file_path" 2>/dev/null)

if [ -n "$example_body" ]; then
  # -- 4a.1: names the example imports from a workspace crate. -------
  claimed=$(
    {
      printf '%s\n' "$example_body" \
        | grep -oE 'outl_[a-z0-9_]+::\{[^}]*\}' \
        | sed -E 's/^[^{]*\{//; s/\}$//' \
        | tr ',' '\n'
      printf '%s\n' "$example_body" \
        | grep -oE 'outl_[a-z0-9_]+::[A-Za-z_][A-Za-z0-9_]*' \
        | sed -E 's/^[^:]*:://'
    } 2>/dev/null \
      | sed -E 's/[[:space:]]//g; s/\bas[A-Za-z0-9_]*$//' \
      | grep -E '^[A-Za-z_][A-Za-z0-9_]*$' \
      | grep -vxE 'self|super|crate' \
      | sort -u
  )
  # -- 4a.2: `Type::method(` where Type belongs to this workspace. ---
  # Skipped for methods a derive or std trait can supply.
  derive_provided='default|from|try_from|into|try_into|clone|fmt|parse|to_string|to_owned|as_ref|as_mut|borrow|eq|ne|cmp|partial_cmp|hash|drop|next|into_iter|deserialize|serialize'
  calls=$(printf '%s\n' "$example_body" \
    | grep -oE '\b[A-Z][A-Za-z0-9_]*::[a-z_][A-Za-z0-9_]*[[:space:]]*\(' \
    | sed -E 's/[[:space:]]*\($//' | sort -u)
  call_types=$(sed -E 's/::.*//' <<< "$calls")

  # One batch resolves both 4a.1's imports and 4a.2's receiver types
  # (`Box::new`, `PathBuf::from`, … drop out here and never reach the
  # per-type sweep below).
  unresolved=$(undefined_names "$(printf '%s\n%s\n' "$claimed" "$call_types")")

  while IFS= read -r sym; do
    [ -z "$sym" ] && continue
    grep -qxF "$sym" <<< "$unresolved" \
      && dangling+=("${sym} (imported from a workspace crate, defined nowhere)")
  done <<< "$claimed"

  while IFS= read -r call; do
    [ -z "$call" ] && continue
    ty=${call%%::*}
    method=${call##*::}
    grep -qxE "$derive_provided" <<< "$method" && continue
    grep -qxF "$ty" <<< "$unresolved" && continue  # not ours — unresolvable
    ty_file=$(type_file "$ty")
    [ -z "$ty_file" ] && continue
    owning_crate=$(printf '%s' "$ty_file" | sed -E 's|^.*(crates/[^/]+)/.*|\1|')
    # Warn only when the method is absent from the ENTIRE owning crate:
    # a blanket or trait impl can legitimately live away from `impl Type`.
    if ! grep -rqE "fn[[:space:]]+${method}\b" "$repo_root/$owning_crate/src" 2>/dev/null; then
      dangling+=("${ty}::${method}() — no \`fn ${method}\` anywhere in ${owning_crate}")
    fi
  done <<< "$calls"

  # -- 4b: module-level recipe left unread while the API moved. ------
  if [ -n "$(unchecked_examples "$file_path" module 2>/dev/null)" ] \
     && git ls-files --error-unmatch -- "$file_path" >/dev/null 2>&1; then
    diff_body=$(git diff --no-color -U0 -- "$file_path" 2>/dev/null)
    pub_moved=$(printf '%s\n' "$diff_body" \
      | grep -cE '^[+-](pub use |pub(\([^)]*\))? (fn|struct|enum|trait|const|type) )' || true)
    docs_moved=$(printf '%s\n' "$diff_body" \
      | grep -cE '^[+-][[:space:]]*//[/!]' || true)
    if [ "${pub_moved:-0}" -gt 0 ] && [ "${docs_moved:-0}" -eq 0 ]; then
      stale_recipe=1
    fi
  fi
fi

# -- 4c: a pub item this diff deleted that docs still name. ----------
if git ls-files --error-unmatch -- "$file_path" >/dev/null 2>&1; then
  removed=$(git diff --no-color -U0 -- "$file_path" 2>/dev/null \
    | grep -oE '^-pub(\([^)]*\))? (async )?(fn|struct|enum|trait|const|type) [A-Za-z_][A-Za-z0-9_]*' \
    | sed -E 's/.* //' | sort -u)
  bt='`'
  # Only names that survive nowhere in the workspace: a `pub fn` that moved
  # file or lost `pub` is a refactor, and its docs are still accurate.
  while IFS= read -r sym; do
    [ -z "$sym" ] && continue
    mention=$(ws_grep -l "^[[:space:]]*//[/!].*(${bt}${sym}${bt}|\b${sym}\()" \
      | head -3 | sed "s|^${repo_root}/||")
    [ -n "$mention" ] && deleted_documented+=("${sym} — still named in: $(printf '%s' "$mention" | tr '\n' ' ')")
  done <<< "$(undefined_names "$removed")"
fi

if [ ${#dangling[@]} -gt 0 ]; then
  msg="rule 4a — dangling symbol in an unchecked doc example: %s has a \`\`\`ignore / \`\`\`compile_fail block referencing:\n"
  for d in "${dangling[@]}"; do
    msg+="    - ${d}\n"
  done
  msg+="  why it matters: rustdoc never compiles those blocks, so \`cargo test --doc\` is blind to them.\n"
  msg+="  fix: update the example to the current API, or drop the reference. If the symbol is genuinely\n"
  msg+="       external (re-exported from a dependency), say so and continue."
  warnings+=("$(printf "$msg" "$rel")")
fi

if [ "$stale_recipe" = "1" ]; then
  msg="rule 4b — unchecked module doc may be stale: %s changed its public surface (pub use / pub item)\n"
  msg+="  but no doc-comment line in the file changed, and its \`//! \`\`\`ignore\` example is a recipe rustdoc\n"
  msg+="  never compiles.\n"
  msg+="  why it matters: outl-sync-iroh's Quick start kept teaching the hand-assembled iroh transport after\n"
  msg+="  \`build_default_transport\` became the one owner. Every symbol in it still existed and was still pub,\n"
  msg+="  so nothing mechanical flagged it — the example just taught the anti-pattern the change removed.\n"
  msg+="  fix: re-read the module example against the new public surface and update it, OR state explicitly\n"
  msg+="       that the example is still the recommended path and continue."
  warnings+=("$(printf "$msg" "$rel")")
fi

if [ ${#deleted_documented[@]} -gt 0 ]; then
  msg="rule 4c — deleted symbol still documented: %s removed public item(s) that exist nowhere in the\n"
  msg+="  workspace anymore, yet doc comments still name them:\n"
  for d in "${deleted_documented[@]}"; do
    msg+="    - ${d}\n"
  done
  msg+="  fix: update or delete those doc references in the same change."
  warnings+=("$(printf "$msg" "$rel")")
fi

# --------------------------------------------------------------------
# Emit.
# --------------------------------------------------------------------

[ ${#warnings[@]} -eq 0 ] && exit 0

printf 'DOC DRIFT WARNING — %s was edited without matching doc updates.\n' "$rel" >&2
printf '\n' >&2
for w in "${warnings[@]}"; do
  printf '%s\n\n' "$w" >&2
done
printf 'CLAUDE.md / per-crate CLAUDE.md / .github/copilot-instructions.md / docs/*.md\n' >&2
printf 'are how the next contributor (human or LLM) learns this codebase.\n' >&2
printf 'Treat doc maintenance as part of the same change, not a separate chore.\n' >&2
exit 2
