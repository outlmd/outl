#!/usr/bin/env bash
# PostToolUse: catch cross-file references to section titles that no
# longer exist.
#
# Why this exists: `doc-sync-guard.sh` asks "did the doc file change
# when the code file did?" — it reasons about *files touched*. A rename
# inside a doc passes that gate clean, because the doc *was* touched.
# What it leaves behind is a title quoted in six other files that now
# points at nothing.
#
# The canonical incident: renaming two headings in
# `crates/outl-sync-iroh/CLAUDE.md` ("One endpoint per identity" →
# "... , elected not assigned"; "Passive writers vs the MCP peer" →
# "Who ends up with the endpoint") left dangling quotes in
# docs/clients.md, docs/mcp.md, outl-actions/src/sync.rs, both clients'
# commands/peers.rs, outl-frontend-shared types.ts and three CLAUDE.md
# files. Three separate code-review findings, one root cause.
#
# ── What it recognises ───────────────────────────────────────────────
#
#   1. ARROW form — a markdown path token immediately before `→ "…"`:
#        `docs/cli.md` → "outl recover"
#        [`x/CLAUDE.md`](../x/CLAUDE.md) → "Force-sync trigger (`sync_now`)"
#        //! see `outl-core/CLAUDE.md` → "This crate's
#        //! dependency graph is public surface"      (comment wrap: joined)
#      Also `→ "A", "B"` chains.
#      Applies in .md, and in // /// //! and * comments in .rs/.ts/.tsx.
#
#   2. SEE form — `see "Title"` inside markdown only, optionally
#      redirected with ``in `path.md` ``. Target defaults to the file
#      the reference lives in.
#
# ── What it deliberately does NOT try to catch ───────────────────────
#
#   * `→ "…"` with no `.md` path anchored right before the arrow. That
#     is overwhelmingly UI prose (`long-press → "Run code"`,
#     `right-click → "Open"`, `` `![alt](url)` → "expand" ``) and a
#     regex cannot tell it from a reference. Requiring the anchor is
#     what keeps this hook quiet.
#   * `see "…"` in code. `assert_eq!(b.as_string(), "see ");` matches
#     any reasonable regex, and code comments in this repo use the
#     arrow form anyway.
#   * Bare `CLAUDE.md → "…"` with no backticks (one Swift file). No
#     backticks means no reliable path token.
#   * A rename that only APPENDS to the old title (the "One endpoint
#     per identity" half of the incident above). The old quote is still
#     a legitimate shorthand prefix of the new heading, and this repo
#     uses that shorthand on purpose elsewhere (`outl-core/CLAUDE.md` →
#     "Actor id is device-local" points at a heading that continues
#     ", and the workspace cannot hold it"). Flagging prefixes would
#     mean flagging correct references, so prefixes pass.
#     Renames that REPLACE or REWORD a title — the common case, and the
#     other half of the same incident — are caught.
#
# ── How a title is matched ───────────────────────────────────────────
#
# Both sides are normalised to lowercase alphanumerics, so backticks,
# punctuation and emphasis never cause a miss. A reference resolves if
# it matches, or prefixes, ANY anchor in the target file:
#   * a heading, and the heading minus a trailing `(parenthetical)`
#   * a `**bold**` run (the repo anchors bullets that way)
#   * the first cell of a table row (decision tables get referenced)
# A path token is tried against several roots (source-relative,
# repo-relative, `crates/<token>`, the source's own crate) and matching
# in ANY of them is enough. Ambiguity resolves to silence, always.
#
# ── Scope ────────────────────────────────────────────────────────────
#
#   * default: the file from tool_input.file_path. If it is markdown,
#     ALSO every reference in the repo that points AT it — a rename is
#     an edit to the target, not to the referrers.
#   * `--all`: the whole repo. Run on demand.
#
# Non-blocking: exit 2 with a structured stderr message.

set -uo pipefail

# --------------------------------------------------------------------
# Mode + repo root.
# --------------------------------------------------------------------

mode="hook"
if [ "${1:-}" = "--all" ]; then
  mode="all"
else
  event_json=$(cat)
  file_path=$(printf '%s' "$event_json" | sed -n 's/.*"file_path"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')
fi

repo_root="${CLAUDE_PROJECT_DIR:-$(git rev-parse --show-toplevel 2>/dev/null)}"
[ -z "$repo_root" ] && exit 0
[ -d "$repo_root" ] || exit 0

edited_rel=""
if [ "$mode" = "hook" ]; then
  [ -z "${file_path:-}" ] && exit 0
  [ -f "$file_path" ] || exit 0
  case "$file_path" in
    *.md|*.rs|*.ts|*.tsx) ;;
    *) exit 0 ;;
  esac
  edited_rel=${file_path#"${repo_root}/"}
  case "$edited_rel" in
    */target/*|*/node_modules/*|note-example/*|*/note-example/*|*/fixtures/*) exit 0 ;;
    CHANGELOG.md) exit 0 ;;
  esac
fi

# --------------------------------------------------------------------
# Which files do we read references OUT of?
#
# A code edit can only break its own outgoing references. A markdown
# edit can break every reference pointing at it, so it costs a
# repo-wide sweep — cheap, because ripgrep pre-filters to the ~40 files
# that carry a candidate at all.
# --------------------------------------------------------------------

sweep_repo=0
[ "$mode" = "all" ] && sweep_repo=1
case "$edited_rel" in *.md) sweep_repo=1 ;; esac

sources=()
if [ "$sweep_repo" = "1" ]; then
  while IFS= read -r f; do
    [ -n "$f" ] && sources+=("$f")
  done < <(
    # `.` is not optional: with stdin redirected (which it always is in
    # hook mode — the event JSON arrives there) ripgrep searches stdin
    # instead of the working directory and silently finds nothing.
    cd "$repo_root" && rg -l --no-messages \
      -e '→[[:space:]]*"' -e '[Ss]ee[[:space:]]+"' \
      --glob '*.md' --glob '*.rs' --glob '*.ts' --glob '*.tsx' \
      --glob '!target/**' --glob '!**/target/**' \
      --glob '!node_modules/**' --glob '!**/node_modules/**' \
      --glob '!note-example/**' --glob '!**/fixtures/**' \
      --glob '!CHANGELOG.md' \
      . 2>/dev/null | sed 's|^\./||' | sort
  )
else
  sources+=("$edited_rel")
fi

[ ${#sources[@]} -eq 0 ] && exit 0

# --------------------------------------------------------------------
# Extractor. Emits, per reference:
#   <src-rel> TAB <line> TAB <raw title> TAB <normalised title> TAB <path token>
# An empty path token means "look in the file the reference lives in".
# --------------------------------------------------------------------

extract_awk='
function nrm(x,   y) { y = x; gsub(/[^0-9A-Za-z]/, "", y); return tolower(y) }

function stripc(l) {
  sub(/^[[:space:]]+/, "", l)
  sub(/^(\/\/!|\/\/\/|\/\/)[[:space:]]?/, "", l)
  sub(/^\*[[:space:]]?/, "", l)
  sub(/^[[:space:]]+/, "", l)
  return l
}

# 1 when the text after the LAST arrow has an unbalanced quote, i.e. the
# title was wrapped onto the following comment line.
function unterm(s,   k, tail, c) {
  tail = s
  k = index(tail, "→")
  if (k == 0) return 0
  while (k > 0) { tail = substr(tail, k + 3); k = index(tail, "→") }
  c = gsub(/"/, "\"", tail)
  return (c % 2 == 1)
}

# The path token sitting immediately before an arrow: the last
# `backticked` run, or the target of a [label](link). Anything else
# means this arrow is prose, not a reference.
function anchor(b,   n, last, inner, i, q) {
  sub(/[[:space:]]+$/, "", b)
  n = length(b)
  if (n == 0) return ""
  last = substr(b, n, 1)
  inner = substr(b, 1, n - 1)
  q = 0
  if (last == "`") {
    for (i = length(inner); i >= 1; i--) if (substr(inner, i, 1) == "`") { q = i; break }
  } else if (last == ")") {
    for (i = length(inner); i >= 1; i--) if (substr(inner, i, 1) == "(") { q = i; break }
  } else {
    return ""
  }
  if (q == 0) return ""
  return substr(inner, q + 1)
}

# Neutralise quotes inside inline code spans, keeping every byte offset
# intact. A doc that *documents* this pattern writes `` `x.md` → "Title" ``
# inside a code span, and without this the guard reports its own prose.
# Backtick runs pair CommonMark-style (a run of N closes on the next run
# of exactly N), so a title carrying `code` is left alone.
function mask(s,   out, i, n, k, r, j, r2, cl, inner, delim) {
  n = length(s)
  out = ""
  i = 1
  while (i <= n) {
    k = index(substr(s, i), "`")
    if (k == 0) { out = out substr(s, i); break }
    out = out substr(s, i, k - 1)
    i = i + k - 1
    r = 0
    while (i + r <= n && substr(s, i + r, 1) == "`") r++
    delim = substr(s, i, r)
    j = i + r
    cl = 0
    while (j <= n) {
      if (substr(s, j, 1) == "`") {
        r2 = 0
        while (j + r2 <= n && substr(s, j + r2, 1) == "`") r2++
        if (r2 == r) { cl = j; break }
        j += r2
      } else {
        j++
      }
    }
    if (cl == 0) { out = out delim; i += r; continue }
    inner = substr(s, i + r, cl - (i + r))
    gsub(/"/, ".", inner)
    out = out delim inner delim
    i = cl + r
  }
  return out
}

function emit(ln, title, tok,   nt) {
  if (title == "" || length(title) > 140) return
  nt = nrm(title)
  if (nt == "") return
  printf "%s\t%d\t%s\t%s\t%s\n", REL, ln, title, nt, tok
}

{
  line[NR] = $0
  if (ISMD && $0 ~ /^[[:space:]]*```/) { fence = !fence; fenced[NR] = 1 }
  else fenced[NR] = fence
}

END {
  for (i = 1; i <= NR; i++) {
    if (ISMD && fenced[i]) continue
    s = line[i]
    if (!ISMD && s !~ /^[[:space:]]*(\/\/|\*|\/\*)/) continue

    # Join wrapped comment continuations so a title split across two
    # `//!` lines still resolves.
    if (!ISMD) {
      j = i
      while (j < NR && j - i < 3 && unterm(s)) { j++; s = s " " stripc(line[j]) }
    }

    if (index(s, "`") > 0 && index(s, "\"") > 0) s = mask(s)

    # --- arrow form -------------------------------------------------
    rest = s
    while ((p = index(rest, "→")) > 0) {
      before = substr(rest, 1, p - 1)
      rest = substr(rest, p + 3)
      a = rest
      sub(/^[[:space:]]+/, "", a)
      if (substr(a, 1, 1) != "\"") continue
      tok = anchor(before)
      if (tok !~ /\.md$/) continue
      if (tok ~ /[[:space:]`\[\]]/) continue
      rem = substr(a, 2)
      while (1) {
        e = index(rem, "\"")
        if (e == 0) break
        emit(i, substr(rem, 1, e - 1), tok)
        tail = substr(rem, e + 1)
        if (tail !~ /^[[:space:]]*(,|and|, and)[[:space:]]*"/) break
        sub(/^[[:space:]]*(,|and|, and)[[:space:]]*"/, "", tail)
        rem = tail
      }
    }

    # --- see form (markdown only) -----------------------------------
    if (!ISMD) continue
    t = s
    while (match(t, /(^|[^0-9A-Za-z])[Ss]ee[[:space:]]+"/) > 0) {
      rem = substr(t, RSTART + RLENGTH)
      t = rem
      e = index(rem, "\"")
      if (e == 0) break
      title = substr(rem, 1, e - 1)
      tail = substr(rem, e + 1)
      if (substr(title, 1, 1) !~ /[A-Z`]/) continue
      tok2 = ""
      if (match(tail, /^[[:space:]]*in[[:space:]]+`[^`]+`/) > 0) {
        cap = substr(tail, RSTART, RLENGTH)
        sub(/^[[:space:]]*in[[:space:]]+`/, "", cap)
        sub(/`$/, "", cap)
        if (cap ~ /\.md$/) tok2 = cap
      }
      emit(i, title, tok2)
    }
  }
}
'

# --------------------------------------------------------------------
# Anchor dump for a target file: every string a reference may quote.
# --------------------------------------------------------------------

anchors_awk='
function nrm(x,   y) { y = x; gsub(/[^0-9A-Za-z]/, "", y); return tolower(y) }
function out(raw,   n) { n = nrm(raw); if (n != "") print n "\t" raw }

/^[[:space:]]*#/ {
  h = $0
  sub(/^[[:space:]]*#+[[:space:]]*/, "", h)
  sub(/[[:space:]]*#+[[:space:]]*$/, "", h)
  out(h)
  h2 = h
  if (sub(/[[:space:]]*\([^()]*\)[[:space:]]*$/, "", h2)) out(h2)
}

# Emphasised label lines. When a referenced paragraph is not a heading
# this repo anchors it with bold — sometimes the whole line
# (`**Second hard rule: level 3 says *what* to delete...**`), sometimes
# a run inside it (`Functions **never**:`). Both are quoted verbatim as
# they RENDER, so the anchor is the line with emphasis stripped, plus
# each bold run on its own.
index($0, "**") > 0 {
  L = $0
  sub(/^[[:space:]]*>[[:space:]]*/, "", L)
  sub(/^[[:space:]]*([-+]|[0-9]+\.)[[:space:]]+/, "", L)
  gsub(/\*\*/, "", L)
  gsub(/__/, "", L)
  sub(/^[[:space:]]+/, "", L)
  sub(/[[:space:]]+$/, "", L)
  out(L)

  b = $0
  while ((s1 = index(b, "**")) > 0) {
    b = substr(b, s1 + 2)
    s2 = index(b, "**")
    if (s2 == 0) break
    out(substr(b, 1, s2 - 1))
    b = substr(b, s2 + 2)
  }
}

# First cell of a table row — decision tables get referenced by row.
/^[[:space:]]*\|/ {
  if ($0 ~ /^[[:space:]]*\|[[:space:]]*:?-+/) next
  row = $0
  sub(/^[[:space:]]*\|/, "", row)
  c = index(row, "|")
  if (c > 0) {
    cell = substr(row, 1, c - 1)
    gsub(/^[[:space:]]+|[[:space:]]+$/, "", cell)
    out(cell)
  }
}
'

# --------------------------------------------------------------------
# Path helpers.
# --------------------------------------------------------------------

# Everything below is deliberately subshell-free on the hot path: a
# markdown edit resolves ~90 references, and a handful of forked
# `sed`/`dirname`/`sort` per reference is the whole runtime budget.

normalize_path() {
  local p=$1 prev
  while [ "$p" != "${p//\/.\//\/}" ]; do p=${p//\/.\//\/}; done
  p=${p#./}
  while :; do
    prev=$p
    p=$(printf '%s' "$p" | sed -e 's|[^/][^/]*/\.\./||')
    [ "$p" = "$prev" ] && break
  done
  printf '%s' "$p"
}

# Appends $1 to CANDS when it exists on disk and isn't already listed.
add_cand() {
  local c=$1
  [ -f "$repo_root/$c" ] || return 0
  case "
$CANDS" in *"
$c
"*) return 0 ;; esac
  CANDS="$CANDS$c
"
}

# Sets CANDS to the newline-terminated existing targets for token $1
# seen in source file $2. Matching in ANY of them counts as resolved.
candidates_for() {
  local tok=$1 src=$2 srcdir crate joined
  CANDS=""
  srcdir=${src%/*}
  if [ "$srcdir" != "$src" ]; then
    joined="$srcdir/$tok"
    case "$joined" in
      *'/../'*|*'/./'*) joined=$(normalize_path "$joined") ;;
    esac
    add_cand "$joined"
  fi
  add_cand "$tok"
  add_cand "crates/$tok"
  case "$src" in
    crates/*/*)
      crate=${src#crates/}
      add_cand "crates/${crate%%/*}/$tok"
      ;;
  esac
}

# --------------------------------------------------------------------
# Sweep.
# --------------------------------------------------------------------

cache_dir=$(mktemp -d "${TMPDIR:-/tmp}/outl-secref.XXXXXX") || exit 0
trap 'rm -rf "$cache_dir"' EXIT

# Anchor dumps are memoised: a repo sweep hits `outl-sync-iroh/CLAUDE.md`
# a dozen times.
anchors_of() {
  ANCHOR_FILE="$cache_dir/${1//\//_}"
  [ -f "$ANCHOR_FILE" ] && return 0
  # stderr is deliberately NOT suppressed: a typo in the embedded awk
  # program would otherwise disable this hook silently, which is how it
  # spent one round-trip reporting nothing at all.
  LC_ALL=C awk "$anchors_awk" "$repo_root/$1" > "$ANCHOR_FILE"
}

findings=()

for src in "${sources[@]}"; do
  [ -f "$repo_root/$src" ] || continue
  ismd=0
  case "$src" in *.md) ismd=1 ;; esac

  while IFS=$'\t' read -r r_src r_line r_title r_norm r_tok; do
    [ -z "${r_norm:-}" ] && continue

    if [ -z "${r_tok:-}" ]; then
      CANDS="$src
"
    else
      candidates_for "$r_tok" "$src"
    fi
    # Unresolvable target → stay quiet. Ambiguity is not evidence.
    [ -z "$CANDS" ] && continue

    # On a markdown edit we swept the repo to find inbound references;
    # keep only the ones actually aimed at the edited file.
    if [ "$mode" = "hook" ] && [ "$sweep_repo" = "1" ] && [ "$r_src" != "$edited_rel" ]; then
      case "
$CANDS" in *"
$edited_rel
"*) ;; *) continue ;; esac
    fi

    verdict="broken"
    while IFS= read -r cand; do
      [ -z "$cand" ] && continue
      anchors_of "$cand"
      # Exact anchor, or the reference is a prefix of one (legitimate
      # shorthand — see the header note on append-only renames).
      if LC_ALL=C awk -F'\t' -v w="$r_norm" '
           index($1, w) == 1 { hit = 1; exit }
           END { exit(hit ? 0 : 1) }
         ' "$ANCHOR_FILE" 2>/dev/null; then
        verdict="ok"
        break
      fi
    done <<< "$CANDS"

    [ "$verdict" = "ok" ] && continue

    targets=${CANDS//$'\n'/ }
    findings+=("${r_src}:${r_line}"$'\t'"${r_title}"$'\t'"${targets% }")
  done < <(LC_ALL=C awk -v REL="$src" -v ISMD="$ismd" "$extract_awk" "$repo_root/$src")
done

[ ${#findings[@]} -eq 0 ] && exit 0

{
  printf 'DANGLING SECTION REFERENCE — %d reference(s) quote a section title that does not exist.\n' "${#findings[@]}"
  printf '\n'
  for f in "${findings[@]}"; do
    loc=$(printf '%s' "$f" | cut -f1)
    ttl=$(printf '%s' "$f" | cut -f2)
    tgt=$(printf '%s' "$f" | cut -f3)
    printf '  %s\n' "$loc"
    printf '      quotes: "%s"\n' "$ttl"
    printf '      looked in: %s\n' "$tgt"
  done
  printf '\n'
  printf 'A renamed heading leaves its old title quoted in every file that\n'
  printf 'pointed at it. doc-sync-guard.sh cannot see this: the doc *was*\n'
  printf 'touched, so it passes that gate clean.\n'
  printf '\n'
  printf 'fix: update each quote to the current heading, or restore the old\n'
  printf '     heading. If the reference is intentional prose and not a\n'
  printf '     section pointer, drop the quotes so it stops reading as one.\n'
  printf '     Full-repo sweep: .claude/hooks/section-ref-guard.sh --all\n'
} >&2

exit 2
