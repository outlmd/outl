#!/usr/bin/env bash
# Upload native debug files for one desktop build.
#
# Usage: upload-sentry-debug-files.sh <release> <label> <file> [file...]
#
# <release> is the name the binary reports (`desktop@<version>`).
# <label> is a short phrase for the warning ("the Linux desktop build").
#
# SENTRY_AUTH_TOKEN, SENTRY_ORG and SENTRY_PROJECT come from the environment.
# A missing token, a release that already exists, or a sentry-cli failure
# warns and exits 0. A release must not wait on Sentry. The caller decides
# what a missing file means; this script is not invoked with an empty list.
set -uo pipefail

if [[ $# -lt 3 ]]; then
  echo "::error::usage: upload-sentry-debug-files.sh <release> <label> <file>..." >&2
  exit 2
fi

release="$1"
label="$2"
shift 2

if [[ -z "${SENTRY_AUTH_TOKEN:-}" ]]; then
  note="SENTRY_AUTH_TOKEN is unset, so no debug symbols were uploaded for ${label}. A panic from this build will arrive in Sentry as bare addresses. The token is a secret of the release environment (org outl, project outl, scope project:releases), which these jobs select on main and on v* tags."
  echo "::warning::${note}"
  echo "- symbols: ${note}" >> "${GITHUB_STEP_SUMMARY:-/dev/null}"
  exit 0
fi

export SENTRY_RELEASE="${release}"

# The three desktop jobs create one release. The second and third find it
# already there; that is not a failed upload, and the debug files still go up.
if ! npx --yes @sentry/cli@3.8.0 releases new "${SENTRY_RELEASE}"; then
  echo "::warning::sentry-cli could not create release ${SENTRY_RELEASE} for ${label} (it may already exist). Debug files are still uploaded."
fi

echo "uploading to ${SENTRY_ORG:-?}/${SENTRY_PROJECT:-?} release ${SENTRY_RELEASE}: $*"
# `--include-sources` bundles the files named by the debug info, from this
# checkout. Line tables carry the path and the line; the source is what makes
# a frame readable instead of `mod.rs:42` with no body.
if ! npx --yes @sentry/cli@3.8.0 debug-files upload --include-sources "$@"; then
  note="sentry-cli failed to upload debug symbols for ${label}; the release continues, but crashes from this build will not symbolicate"
  echo "::warning::${note}"
  echo "- symbols: ${note}" >> "${GITHUB_STEP_SUMMARY:-/dev/null}"
fi
