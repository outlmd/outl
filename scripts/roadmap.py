#!/usr/bin/env python3
"""The roadmap issue is generated from the project board, never written by hand.

Two things used to hold the same fact and disagree about it. The board
(`users/avelino/projects/2`, field `Status`) and the body of the roadmap issue
each said where a piece of work stood, and each was edited separately. By
2026-11-21 they had drifted all the way apart: the issue listed eight shipped
items as current, had `#110` and `#36` in Later while the board had them in
Now, and had `#257` / `#290` in Now while the board had them in Later. Two of
the open issues (`#148`, `#153`) were on neither.

So the board owns `Status` and this script renders it. The issue body is
output. Editing it by hand is pointless: the next run overwrites it.

What stays here rather than on the board is the **prose** — the section
intros and the theme names in `THEMES`. A generated list of 70 bullets is
accurate and unreadable; the editorial judgment of what the sections mean is
the part a board field cannot carry. Change the wording here, change the
contents by moving a card.

Modes:

  `--render`      Print the issue body to stdout. Reads nothing but the board,
                  writes nothing. Use it to see a change before it ships.

  `--publish`     Render, then update the roadmap issue, but only when the
                  body actually changed. A no-op run makes no API call and
                  leaves no `updated_at` churn, which matters because the cron
                  runs whether or not anyone touched the board.

  `--sync`        Hold the board against the repo, idempotently:
                  an open issue missing from the board is added to Later
                  (the safe default — Later means "not triaged into a lane"),
                  a closed issue is moved to Shipped, and a pull request with
                  no status is dropped. A PR is not a roadmap item; 256 of
                  them had accumulated with no status, 52% of the board.

  `--check`       Exit non-zero on drift, for CI. Fails when an open issue is
                  off the board, when a closed issue is not Shipped, or when
                  the published body is stale against the board. This is the
                  check that would have caught all of the above.

  `--seed-status` One-off, kept for the record rather than for reuse: the
                  initial classification of 70 open issues from their
                  `priority:` / `phase:` labels. After this, `Status` is set
                  by a human moving a card and the labels no longer decide it.

Usage:
  scripts/roadmap.py --render
  scripts/roadmap.py --publish [--dry-run]
  scripts/roadmap.py --sync [--dry-run]
  scripts/roadmap.py --check
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from dataclasses import dataclass, field

OWNER = "avelino"
PROJECT_NUMBER = 2
REPO = "outlmd/outl"
ROADMAP_ISSUE = 376
BOARD_URL = f"https://github.com/users/{OWNER}/projects/{PROJECT_NUMBER}/views/1"

# Board status names, in the order the roadmap reads. `Shipped` is deliberately
# absent: the roadmap says where work is going, and the changelog already owns
# what landed.
LANES = ["🔨 In Progress", "🔜 Now", "⏭️ Next", "🔭 Later"]
SHIPPED = "✅ Shipped"

LANE_TITLE = {
    "🔨 In Progress": "In progress",
    "🔜 Now": "Now",
    "⏭️ Next": "Next",
    "🔭 Later": "Later",
}

LANE_INTRO = {
    "🔨 In Progress": "Being written right now.",
    "🔜 Now": (
        "Getting to a first non-beta release. Two halves: the bugs that reach a user "
        "on the first day, and making the TUI hold up as a first-class client rather "
        "than a thing that mostly works."
    ),
    "⏭️ Next": (
        "Queued behind Now, and mostly about reach: the same capability on every client, "
        "and the graph being navigable at the size a real one reaches."
    ),
    "🔭 Later": (
        "Real, wanted, and not scheduled. Several of these need a design before they "
        "need an implementation, which is what the `status:needs-design` label marks."
    ),
}

# Themes group the bullets inside a lane. First match wins, so the order is the
# editorial call, and it is made from the reader's side rather than the crate
# layout's: someone reading this uses one client, so `area:mobile` outranks
# `area:actions` even though the work lands in `outl-actions`. A chrome bug goes
# under the audit, not under "TUI". `None` means "everything left over".
THEMES: list[tuple[str | None, str]] = [
    ("phase:4", "The TUI audit"),
    ("parity", "Parity across clients"),
    ("area:remind", "Reminders"),
    ("area:sync", "Sync and storage"),
    ("area:core", "Sync and storage"),
    ("area:md", "Markdown and the graph"),
    ("area:tui", "TUI"),
    ("area:mobile", "Mobile"),
    ("area:desktop", "Desktop"),
    ("area:cli", "CLI and MCP"),
    ("area:mcp", "CLI and MCP"),
    ("area:actions", "Shared behaviour"),
    ("area:plugins", "Plugins"),
    ("area:config", "Config"),
    ("area:docs", "Docs"),
    (None, "Everything else"),
]

# Conventional-commit prefixes read as noise in a roadmap bullet: the lane and
# the theme already say where the work is, so `feat(mobile):` repeats one and
# `fix:` repeats nothing. Stripped for display only, never on the issue itself.
COMMIT_PREFIX = re.compile(
    r"^(feat|fix|perf|refactor|chore|docs|test|build|ci|style|revert)(\([^)]*\))?!?:\s*"
)

HEADER = """<!-- Generated by scripts/roadmap.py from the project board. Do not edit by hand: the next run overwrites it. -->

This is where outl is going, in plain words.

It is **generated from the [project board]({board})** every few hours, so a line here exists only
because a card exists there. Moving a card moves this. The board also carries the day-to-day detail
this page leaves out: who is on it, which pull request is open against it, what the priority is.

Want to help? [`good first issue`](https://github.com/{repo}/labels/good%20first%20issue) and
[`help wanted`](https://github.com/{repo}/labels/help%20wanted) are the way in, and questions go to
[Discussions](https://github.com/{repo}/discussions).

What already shipped is in [`CHANGELOG.md`](https://github.com/{repo}/blob/main/CHANGELOG.md),
not here.
"""

FOOTER = """---

**{open_count} open items on the board**, {lane_summary}.

Generated from the board at {board}. The generator, the section wording and the theme names are in
[`scripts/roadmap.py`](https://github.com/{repo}/blob/main/scripts/roadmap.py).
"""


@dataclass
class Item:
    item_id: str
    status: str | None
    priority: str | None
    kind: str  # "Issue" | "PullRequest" | "DraftIssue"
    number: int | None = None
    title: str = ""
    state: str = ""  # OPEN | CLOSED | MERGED
    url: str = ""
    labels: list[str] = field(default_factory=list)

    @property
    def is_open_issue(self) -> bool:
        return self.kind == "Issue" and self.state == "OPEN"

    @property
    def needs_design(self) -> bool:
        return "status:needs-design" in self.labels

    @property
    def priority_label(self) -> str | None:
        """Priority as the repo states it, which is the label and not the field.

        230 of 233 issues had an empty `Priority` field while carrying a
        `priority:` label, so reading the field answered "unset" for almost
        everything. `--sync` copies the label into the field; until it has run
        over an issue, the label is the only one of the two that is populated,
        and it stays the source either way.
        """
        for label in self.labels:
            if label.startswith("priority:"):
                return label.split(":", 1)[1]
        return None

    def theme(self) -> str:
        for label, name in THEMES:
            if label is None or label in self.labels:
                return name
        return "Everything else"


def gh(args: list[str], stdin: str | None = None) -> str:
    out = subprocess.run(
        ["gh", *args],
        capture_output=True,
        text=True,
        input=stdin,
        check=False,
    )
    if out.returncode != 0:
        sys.exit(f"gh {' '.join(args[:2])} failed:\n{out.stderr.strip()}")
    return out.stdout


def graphql(query: str, **variables: object) -> dict:
    args = ["api", "graphql", "-f", f"query={query}"]
    for key, value in variables.items():
        flag = "-F" if isinstance(value, (int, bool)) else "-f"
        args += [flag, f"{key}={value}"]
    payload = json.loads(gh(args))
    if "errors" in payload:
        sys.exit("graphql errors: " + json.dumps(payload["errors"], indent=2))
    return payload["data"]


BOARD_QUERY = """
query($owner: String!, $number: Int!, $cursor: String) {
  user(login: $owner) {
    projectV2(number: $number) {
      id
      items(first: 100, after: $cursor) {
        pageInfo { hasNextPage endCursor }
        nodes {
          id
          status: fieldValueByName(name: "Status") {
            ... on ProjectV2ItemFieldSingleSelectValue { name }
          }
          priority: fieldValueByName(name: "Priority") {
            ... on ProjectV2ItemFieldSingleSelectValue { name }
          }
          content {
            __typename
            ... on Issue {
              number title state url
              labels(first: 30) { nodes { name } }
            }
            ... on PullRequest { number title state url }
          }
        }
      }
    }
  }
}
"""

FIELDS_QUERY = """
query($owner: String!, $number: Int!) {
  user(login: $owner) {
    projectV2(number: $number) {
      id
      status: field(name: "Status") {
        ... on ProjectV2SingleSelectField { id options { id name } }
      }
      priority: field(name: "Priority") {
        ... on ProjectV2SingleSelectField { id options { id name } }
      }
    }
  }
}
"""

SET_FIELD = """
mutation($project: ID!, $item: ID!, $field: ID!, $option: String!) {
  updateProjectV2ItemFieldValue(input: {
    projectId: $project, itemId: $item, fieldId: $field,
    value: { singleSelectOptionId: $option }
  }) { projectV2Item { id } }
}
"""


def fetch_board() -> tuple[str, list[Item]]:
    items: list[Item] = []
    project_id = ""
    cursor = None
    while True:
        args: dict[str, object] = {"owner": OWNER, "number": PROJECT_NUMBER}
        if cursor:
            args["cursor"] = cursor
        page = graphql(BOARD_QUERY, **args)["user"]["projectV2"]
        project_id = page["id"]
        for node in page["items"]["nodes"]:
            content = node.get("content") or {}
            items.append(
                Item(
                    item_id=node["id"],
                    status=(node.get("status") or {}).get("name"),
                    priority=(node.get("priority") or {}).get("name"),
                    kind=content.get("__typename", "DraftIssue"),
                    number=content.get("number"),
                    title=content.get("title", ""),
                    state=content.get("state", ""),
                    url=content.get("url", ""),
                    labels=[
                        label["name"]
                        for label in (content.get("labels") or {}).get("nodes", [])
                    ],
                )
            )
        if not page["items"]["pageInfo"]["hasNextPage"]:
            return project_id, items
        cursor = page["items"]["pageInfo"]["endCursor"]


@dataclass
class Field:
    field_id: str
    options: dict[str, str]


def fields() -> tuple[str, Field, Field]:
    project = graphql(FIELDS_QUERY, owner=OWNER, number=PROJECT_NUMBER)["user"][
        "projectV2"
    ]

    def one(key: str) -> Field:
        return Field(
            field_id=project[key]["id"],
            options={o["name"]: o["id"] for o in project[key]["options"]},
        )

    return project["id"], one("status"), one("priority")


def open_issues() -> dict[int, list[str]]:
    rows = json.loads(
        gh(
            [
                "issue",
                "list",
                "--repo",
                REPO,
                "--state",
                "open",
                "--limit",
                "500",
                "--json",
                "number,labels",
            ]
        )
    )
    return {r["number"]: [label["name"] for label in r["labels"]] for r in rows}


# A title's first word is only safe to capitalise when it is plainly a word.
# `search_blocks` and `iOS:` are not, and capitalising them produced
# `Search_blocks` and `IOS:`, which is a roadmap stating an identifier that
# does not exist. `outl` is a word by this test and still must not be touched.
PLAIN_WORD = re.compile(r"^[a-z]+(?=[\s,:.]|$)")
KEEP_LOWER = {"outl"}


def headline(title: str) -> str:
    stripped = COMMIT_PREFIX.sub("", title).strip()
    if not stripped:
        return title
    match = PLAIN_WORD.match(stripped)
    if match and match.group() not in KEEP_LOWER:
        stripped = stripped[0].upper() + stripped[1:]
    return stripped


def bullet(item: Item) -> str:
    tail = " · needs design" if item.needs_design else ""
    return f"- [#{item.number}]({item.url}) {headline(item.title)}{tail}"


def render(items: list[Item]) -> str:
    # The roadmap issue is an item on its own board. It describes the board, so
    # listing itself would be the one bullet that is never work.
    roadmap = [i for i in items if i.is_open_issue and i.number != ROADMAP_ISSUE]
    parts = [HEADER.format(board=BOARD_URL, repo=REPO)]

    counts: list[str] = []
    for lane in LANES:
        lane_items = [i for i in roadmap if i.status == lane]
        if not lane_items:
            continue
        counts.append(f"{len(lane_items)} in {LANE_TITLE[lane]}")
        parts.append(f"## {LANE_TITLE[lane]}\n\n{LANE_INTRO[lane]}\n")

        grouped: dict[str, list[Item]] = {}
        for item in lane_items:
            grouped.setdefault(item.theme(), []).append(item)
        # Themes in THEMES order, biggest-first within the leftovers.
        order = []
        for _, name in THEMES:
            if name in grouped and name not in order:
                order.append(name)
        for theme in order:
            rows = sorted(grouped[theme], key=lambda i: -(i.number or 0))
            parts.append(f"**{theme}**\n")
            parts += [bullet(i) for i in rows]
            parts.append("")

    untriaged = [i for i in roadmap if i.status is None]
    if untriaged:
        parts.append(
            "## Not triaged\n\n"
            "On the board with no lane. `--sync` puts a new issue in Later, so a run of these "
            "means someone cleared the field by hand.\n"
        )
        parts += [bullet(i) for i in sorted(untriaged, key=lambda i: -(i.number or 0))]
        parts.append("")

    parts.append(
        FOOTER.format(
            open_count=len(roadmap),
            lane_summary=", ".join(counts) if counts else "none in a lane",
            board=BOARD_URL,
            repo=REPO,
        )
    )
    return "\n".join(parts).rstrip() + "\n"


def published_body() -> str:
    data = json.loads(
        gh(["issue", "view", str(ROADMAP_ISSUE), "--repo", REPO, "--json", "body"])
    )
    return data["body"]


def publish(items: list[Item], dry_run: bool) -> int:
    body = render(items)
    if body.strip() == published_body().strip():
        print(f"#{ROADMAP_ISSUE} already matches the board, nothing to do")
        return 0
    if dry_run:
        print(f"would update #{ROADMAP_ISSUE} ({len(body)} bytes)")
        return 0
    gh(
        ["issue", "edit", str(ROADMAP_ISSUE), "--repo", REPO, "--body-file", "-"],
        stdin=body,
    )
    print(f"updated #{ROADMAP_ISSUE}")
    return 0


def sync(items: list[Item], dry_run: bool) -> int:
    project_id, status, priority = fields()
    on_board = {i.number for i in items if i.kind == "Issue"}
    actions: list[str] = []

    def set_field(item_id: str, fld: Field, option: str, note: str) -> None:
        actions.append(note)
        if dry_run:
            return
        graphql(
            SET_FIELD,
            project=project_id,
            item=item_id,
            field=fld.field_id,
            option=fld.options[option],
        )

    for item in items:
        if item.kind == "Issue" and item.state == "CLOSED" and item.status != SHIPPED:
            set_field(
                item.item_id, status, SHIPPED, f"#{item.number} status -> {SHIPPED}"
            )
        elif item.is_open_issue and item.status is None:
            set_field(
                item.item_id, status, "🔭 Later", f"#{item.number} status -> 🔭 Later"
            )
        elif item.kind == "PullRequest" and item.status is None:
            actions.append(f"PR #{item.number} dropped from the board")
            if not dry_run:
                graphql(
                    """
                    mutation($project: ID!, $item: ID!) {
                      deleteProjectV2Item(input: { projectId: $project, itemId: $item }) {
                        deletedItemId
                      }
                    }
                    """,
                    project=project_id,
                    item=item.item_id,
                )
            continue

        # The label is where priority is decided; the field is what the board can
        # group and sort by. Copying keeps the board usable without making it a
        # second place to set the value. Only for open issues: the priority of
        # something already shipped sorts nothing, and filling it in for 163
        # closed issues is an API call each for a column nobody reads.
        label = item.priority_label
        if item.is_open_issue and label in priority.options and item.priority != label:
            set_field(
                item.item_id,
                priority,
                label,
                f"#{item.number} priority -> {label} (from label)",
            )

    missing = sorted(set(open_issues()) - on_board)
    for number in missing:
        actions.append(f"#{number} added to the board (Later)")
        if dry_run:
            continue
        node = json.loads(
            gh(["issue", "view", str(number), "--repo", REPO, "--json", "id"])
        )["id"]
        new_item = graphql(
            """
            mutation($project: ID!, $content: ID!) {
              addProjectV2ItemById(input: { projectId: $project, contentId: $content }) {
                item { id }
              }
            }
            """,
            project=project_id,
            content=node,
        )["addProjectV2ItemById"]["item"]["id"]
        graphql(
            SET_FIELD,
            project=project_id,
            item=new_item,
            field=status.field_id,
            option=status.options["🔭 Later"],
        )

    for line in actions:
        print(("would: " if dry_run else "") + line)
    if not actions:
        print("board already matches the repo")
    return 0


def check(items: list[Item]) -> int:
    problems: list[str] = []
    on_board = {i.number for i in items if i.kind == "Issue"}

    missing = sorted(set(open_issues()) - on_board)
    if missing:
        problems.append(
            "open issues off the board: "
            + ", ".join(f"#{n}" for n in missing)
            + "\n  fix: scripts/roadmap.py --sync"
        )

    stale = sorted(
        i.number
        for i in items
        if i.kind == "Issue" and i.state == "CLOSED" and i.status != SHIPPED
    )
    if stale:
        problems.append(
            "closed issues not marked Shipped: "
            + ", ".join(f"#{n}" for n in stale)
            + "\n  fix: scripts/roadmap.py --sync"
        )

    if render(items).strip() != published_body().strip():
        problems.append(
            f"#{ROADMAP_ISSUE} is stale against the board"
            "\n  fix: scripts/roadmap.py --publish"
        )

    if not problems:
        print("roadmap and board agree")
        return 0
    for problem in problems:
        print(f"drift: {problem}", file=sys.stderr)
    return 1


def seed_status(items: list[Item], dry_run: bool) -> int:
    """One-off: the initial lane for each open issue, from its labels.

    Run once, on 2026-11-21. `Status` is a human judgment from here on, so this
    exists to show how the first classification was reached, not to be re-run.
    `In Progress` is never touched: something being written is not a label.
    """
    project_id, status, _ = fields()
    for item in items:
        if not item.is_open_issue or item.number == ROADMAP_ISSUE:
            continue
        if item.status == "🔨 In Progress":
            continue
        if item.priority_label == "P1" or "phase:4" in item.labels:
            lane = "🔜 Now"
        elif item.priority_label == "P2":
            lane = "⏭️ Next"
        else:
            lane = "🔭 Later"
        if item.status == lane:
            continue
        print(
            ("would: " if dry_run else "") + f"#{item.number} {item.status} -> {lane}"
        )
        if dry_run:
            continue
        graphql(
            SET_FIELD,
            project=project_id,
            item=item.item_id,
            field=status.field_id,
            option=status.options[lane],
        )
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--render", action="store_true")
    mode.add_argument("--publish", action="store_true")
    mode.add_argument("--sync", action="store_true")
    mode.add_argument("--check", action="store_true")
    mode.add_argument("--seed-status", action="store_true")
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()

    _, items = fetch_board()

    if args.render:
        sys.stdout.write(render(items))
        return 0
    if args.publish:
        return publish(items, args.dry_run)
    if args.sync:
        return sync(items, args.dry_run)
    if args.check:
        return check(items)
    return seed_status(items, args.dry_run)


if __name__ == "__main__":
    sys.exit(main())
