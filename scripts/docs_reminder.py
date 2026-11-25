#!/usr/bin/env python3
"""Decide whether a PR owes the user a docs update, and write the nudge.

`.github/workflows/docs-reminder.yml` is a thin shell around this file: it
lists the PR's changed files, calls this script, and posts what comes back.
Everything that takes judgement lives here so it can be unit tested
(`scripts/tests/test_docs_reminder.py`) without a runner.

The path map is the "If your PR changes..." table in `docs/contributing.md`
seen from the user's side: a path on the left, the page that owns the fact on
the right. When a row there changes, change `RULES` below.

Usage (stdlib only):

    docs_reminder.py --files files.txt --title "feat: ..." \
        --labels "a,b" --actor someone --body-out body.md

Prints `action=<comment|resolved|skip>` and `reason=<why>` on stdout, one per
line, so the workflow can append it to `$GITHUB_OUTPUT`.
"""

from __future__ import annotations

import argparse
import os
import pathlib
import re
import sys
from dataclasses import dataclass

MARKER = "<!-- outl-docs-reminder -->"

LABEL_SKIP = "no-docs-needed"
LABEL_FORCE = "needs-docs"

# Titles that rarely change what a user can do. `needs-docs` overrides.
QUIET_TITLE = re.compile(
    r"^(chore\(deps[^)]*\)|build\(deps[^)]*\)|ci|test|refactor)(\([^)]*\))?!?:",
    re.IGNORECASE,
)

BOTS = {"dependabot[bot]", "dependabot-preview[bot]", "github-actions[bot]"}

# Code that is not user-visible even inside a user-visible crate.
NOT_USER_VISIBLE = re.compile(
    r"(^|/)(tests?|benches|fixtures|__tests__)/"
    r"|(^|/)tests?\.rs$"
    r"|_tests?\.rs$"
    r"|\.(test|spec)\.[jt]sx?$"
    r"|(^|/)CLAUDE\.md$"
)


@dataclass(frozen=True)
class Rule:
    area: str
    pattern: re.Pattern[str]
    pages: tuple[str, ...]
    # Which clients can see this surface, so the checklist names the verdicts
    # the author owes (invariant 12). Empty means the area is not per-client.
    clients: tuple[str, ...] = ()


def _r(area: str, pattern: str, pages: tuple[str, ...], clients: tuple[str, ...] = ()) -> Rule:
    return Rule(area, re.compile(pattern), pages, clients)


# Order matters: the first rule whose pattern matches a file claims it, so the
# MCP registry has to come before the rest of `outl-cli`.
RULES: tuple[Rule, ...] = (
    _r("MCP tools", r"^crates/outl-cli/src/mcp/", ("docs/mcp.md", "docs/mcp-tools.md")),
    _r("CLI", r"^crates/outl-cli/src/", ("docs/cli.md", "docs/cli-recipes.md")),
    _r("TUI", r"^crates/outl-tui/src/", ("docs/tui.md", "docs/tui-commands.md", "docs/shortcuts.md"), ("TUI",)),
    _r(
        "Desktop",
        r"^crates/outl-desktop/(src|src-tauri/src)/",
        ("docs/desktop.md", "docs/desktop-ux.md", "docs/desktop-keybindings.md"),
        ("Desktop",),
    ),
    _r("Mobile", r"^crates/outl-mobile/(src|src-tauri/src)/", ("docs/mobile.md", "docs/mobile-capture.md"), ("Mobile",)),
    _r(
        "Shared frontend",
        r"^crates/(outl-frontend-shared|outl-tauri-shared)/src/",
        ("docs/desktop.md", "docs/mobile.md"),
        ("Desktop", "Mobile"),
    ),
    _r(
        "Shortcuts",
        r"^crates/outl-shortcuts/src/",
        ("docs/shortcuts.md", "docs/client-parity.md"),
        ("TUI", "Desktop", "Mobile"),
    ),
    _r("Config", r"^crates/outl-config/src/", ("docs/config.md",)),
    _r("Themes", r"^(crates/outl-theme/(src|themes)/|themes/)", ("docs/theming.md",)),
    _r("Plugins", r"^crates/outl-plugins/src/", ("docs/plugins.md", "docs/plugin-api.md")),
    _r("Markdown", r"^crates/outl-md/src/", ("docs/markdown-format.md",)),
    _r("Sync", r"^crates/outl-sync-iroh/src/", ("docs/sync.md", "docs/sync-devices.md")),
    _r("Code blocks", r"^crates/outl-exec/src/", ("docs/exec.md", "docs/markdown-format.md")),
    _r("Import", r"^crates/outl-import/src/", ("docs/import.md",)),
    _r(
        "Workspace actions",
        r"^crates/outl-actions/src/",
        ("docs/clients.md", "docs/concepts.md"),
        ("TUI", "Desktop", "Mobile"),
    ),
)


def is_docs_file(path: str) -> bool:
    """A file that counts as the user hearing about the change."""
    return path.startswith("docs/") or path in ("README.md", "CHANGELOG.md")


def user_visible_areas(files: list[str]) -> list[Rule]:
    """The rules claimed by at least one non-test file, in rule order."""
    hit: dict[str, Rule] = {}
    for f in files:
        if NOT_USER_VISIBLE.search(f):
            continue
        for rule in RULES:
            if rule.pattern.search(f):
                hit.setdefault(rule.area, rule)
                break
    return list(hit.values())


def decide(files: list[str], title: str, labels: set[str], actor: str) -> tuple[str, str, list[Rule]]:
    """Returns (action, reason, areas). Action is comment, resolved or skip."""
    if LABEL_SKIP in labels:
        return "skip", f"label {LABEL_SKIP}", []
    if actor in BOTS:
        return "skip", "bot author", []
    if QUIET_TITLE.search(title) and LABEL_FORCE not in labels:
        return "skip", "quiet title prefix", []
    areas = user_visible_areas(files)
    if not areas:
        return "skip", "no user-visible path touched", []
    if any(is_docs_file(f) for f in files):
        return "resolved", "docs touched", areas
    return "comment", "user-visible change without docs", areas


def render(action: str, areas: list[Rule], repo: str = "outlmd/outl") -> str:
    if action == "resolved":
        return f"{MARKER}\n**Docs:** this PR touches the docs now. Thanks.\n"
    pages = sorted({p for a in areas for p in a.pages})
    clients = sorted({c for a in areas for c in a.clients})
    lines = [
        MARKER,
        "**Docs check:** this PR changes something a user can see (" + ", ".join(a.area for a in areas) + ") and touches no `docs/`, `README.md` or `CHANGELOG.md`.",
        "",
        "Code the user cannot find does not exist for them. Four questions before merge:",
        "",
        "- [ ] **What can the user do now** that they could not before? One sentence, in the PR body.",
        "- [ ] **Which page owns it?** Likely " + ", ".join(f"`{p}`" for p in pages) + ". Edit the page that owns the fact, do not add a new one.",
    ]
    if clients:
        lines.append(
            "- [ ] **Per-client verdict** for " + ", ".join(clients) + ": works, works differently, or does not work and why. A gap nobody wrote down is a bug (invariant 12)."
        )
    else:
        lines.append("- [ ] **Per-client verdict:** does every client behave the same? If not, say which one differs and why.")
    lines += [
        "- [ ] **`CHANGELOG.md` `[Unreleased]`** entry, written for the person who updates, not for the reviewer.",
        "",
        "Page bar: goal, steps, a runnable example, per-client table, troubleshooting. See [Documentation standard](https://github.com/" + repo + "/blob/main/docs/contributing.md#documentation-standard).",
        "",
        "Not user-facing? Add the `no-docs-needed` label and this goes away.",
    ]
    return "\n".join(lines) + "\n"


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--files", required=True, help="file with one changed path per line")
    ap.add_argument("--title", default="")
    ap.add_argument("--labels", default="", help="comma separated")
    ap.add_argument("--actor", default="")
    ap.add_argument("--body-out", required=True)
    args = ap.parse_args(argv)

    files = [l.strip() for l in pathlib.Path(args.files).read_text().splitlines() if l.strip()]
    labels = {l.strip() for l in args.labels.split(",") if l.strip()}
    action, reason, areas = decide(files, args.title, labels, args.actor)
    body = render(action, areas, os.environ.get("GITHUB_REPOSITORY", "outlmd/outl")) if action != "skip" else ""
    pathlib.Path(args.body_out).write_text(body)
    print(f"action={action}")
    print(f"reason={reason}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
