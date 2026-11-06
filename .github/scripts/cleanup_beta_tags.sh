#!/usr/bin/env bash
# Prune old `vX.Y.Z-beta.N` tags + GitHub Releases, keeping the
# `$KEEP` most recently generated betas in the whole repo.
#
# Driven by `.github/workflows/cleanup-tags.yml`, which `release.yml`
# calls from `needs: tag` — once per tag created, not once per finished
# release.
#
# Env vars (all required unless noted):
#
#     GH_TOKEN              GitHub token with `contents: write`
#     GITHUB_REPOSITORY     owner/name (set by Actions runner)
#     KEEP                  how many recent betas to keep in total (default: 2)
#     DRY_RUN               "true" to log without deleting (default: false)
#
# Retention is GLOBAL, not per base version. The old rule kept `$KEEP`
# per base, which on 17 base versions is 34 tags, and those older lines
# are dead weight: nothing installs from them (every package manifest
# tracks the newest beta) and the commits stay reachable from `main`.
#
# Scope (intentional):
# - Touches ONLY `vX.Y.Z-beta.N` tags. GA tags like `v0.5.3`
#   (no `-beta.` suffix) are never considered for deletion.
# - Deletes both the GitHub Release (with its built binaries) and
#   the git tag — `gh release delete --cleanup-tag` does both atomically.
# - Orphan tags (tag exists but the release is already gone) get
#   pushed as `:refs/tags/<tag>` to drop the ref alone.
# - A tag a shipped package manifest still references is never pruned,
#   whatever the count says. See the `PROTECTED` block below.

set -euo pipefail

KEEP="${KEEP:-2}"
DRY_RUN="${DRY_RUN:-false}"

if [[ -z "${GITHUB_REPOSITORY:-}" ]]; then
  echo "::error::GITHUB_REPOSITORY is not set"
  exit 1
fi

# Local tags can be stale; trust origin.
git fetch --tags --prune --prune-tags --force origin

# Build prune list:
#   1. Filter to `vX.Y.Z-beta.N` tags only (skip GA + malformed).
#   2. Emit "<base> <beta_num> <full_tag>".
#   3. Group by base, sort beta_num desc, drop the top KEEP.
TO_DELETE_FILE="$(mktemp)"
PRUNE_ALL="$(mktemp)"
PROTECTED="$(mktemp)"
trap 'rm -f "$TO_DELETE_FILE" "$PRUNE_ALL" "$PROTECTED"' EXIT

# Keep `vX.Y.Z-beta.N` only (GA and malformed tags are not candidates),
# sort newest first, skip the newest $KEEP, prune the rest.
#
# `N` is `github.run_number`, a repo-global monotonic counter, so it is
# generation order even across base versions (`v0.12.0-beta.241`
# predates `v0.13.0-beta.250`). Sorting on the version tuple instead
# would agree today and disagree the moment a base version is re-cut.
git tag --list 'v*-beta.*' |
  awk -F'-beta\.' '
    NF == 2 && $1 ~ /^v[0-9]+\.[0-9]+\.[0-9]+$/ && $2 ~ /^[0-9]+$/ {
      print $2 " " $0
    }
  ' |
  sort -k1,1nr |
  tail -n +"$((KEEP + 1))" |
  cut -d' ' -f2 > "$PRUNE_ALL"

# Never prune a tag a shipped package manifest still points at. `brew
# install` and `scoop install` resolve through these files, so dropping
# their tag breaks installs for everyone until the next bump.
#
# This is load-bearing, not decoration. The job no longer waits for
# `Release` to go green (see the workflow), so it can observe a run that
# pushed `beta.N` and `beta.N-1` but died before bumping the manifests —
# leaving the newest $KEEP tags ahead of the version users install. The
# count rule alone would delete exactly the tag still in the formula.
for manifest in Formula/outl-beta.rb Casks/outl-desktop-beta.rb bucket/outl-beta.json; do
  [[ -f "$manifest" ]] || continue
  grep -oE '[0-9]+\.[0-9]+\.[0-9]+-beta\.[0-9]+' "$manifest" || true
done | sed 's/^/v/' | sort -u > "$PROTECTED"

if [[ -s "$PROTECTED" ]]; then
  echo "::notice::Pinned by a shipped manifest, never pruned: $(tr '\n' ' ' < "$PROTECTED")"
  # `|| true`: grep exits 1 when every candidate was protected, which is
  # a legitimate "nothing to do", not a failure.
  grep -vxF -f "$PROTECTED" "$PRUNE_ALL" > "$TO_DELETE_FILE" || true
else
  echo "::warning::No beta version found in any package manifest; relying on the count rule alone."
  cp "$PRUNE_ALL" "$TO_DELETE_FILE"
fi

if [[ ! -s "$TO_DELETE_FILE" ]]; then
  echo "::notice::Nothing to prune (keep=$KEEP newest betas in total)."
  exit 0
fi

COUNT=$(wc -l < "$TO_DELETE_FILE" | tr -d ' ')
echo "::notice::Will prune ${COUNT} tag(s) (keep=$KEEP newest betas in total):"
sed 's/^/  - /' "$TO_DELETE_FILE"

if [[ "$DRY_RUN" == "true" ]]; then
  echo "::notice::Dry run: nothing deleted."
  exit 0
fi

FAILED=0
while IFS= read -r tag; do
  [[ -z "$tag" ]] && continue
  echo "::group::Delete $tag"
  if gh release view "$tag" --repo "$GITHUB_REPOSITORY" >/dev/null 2>&1; then
    # `--cleanup-tag` drops the git ref alongside the release.
    if ! gh release delete "$tag" \
      --repo "$GITHUB_REPOSITORY" \
      --cleanup-tag \
      --yes; then
      echo "::error::Failed to delete release $tag"
      FAILED=$((FAILED + 1))
    fi
  else
    # Release already gone; nuke the orphan tag directly.
    echo "No release for $tag; deleting orphan tag ref."
    if ! git push origin ":refs/tags/$tag"; then
      echo "::error::Failed to delete tag $tag"
      FAILED=$((FAILED + 1))
    fi
  fi
  echo "::endgroup::"
done < "$TO_DELETE_FILE"

if [[ "$FAILED" -gt 0 ]]; then
  echo "::error::Cleanup finished with $FAILED failure(s)."
  exit 1
fi
echo "::notice::Pruned ${COUNT} tag(s) successfully."
