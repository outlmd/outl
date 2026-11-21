# The roadmap is generated from the project board

[Issue #376](https://github.com/outlmd/outl/issues/376) is **output**, not a document.

The source is the [project board](https://github.com/users/avelino/projects/2/views/1) and its `Status` field.
[`scripts/roadmap.py`](../scripts/roadmap.py) renders one from the other.
Editing the issue body by hand accomplishes nothing, because the next run overwrites it.
To move a line in the roadmap, move the card.

## Why it works this way

The board and the issue body used to hold the same fact and be edited separately.
By 2026-11-21 they had drifted all the way apart.

The issue listed eight shipped items as current work (`#302`, `#285`, `#292`, `#16`, `#286`, `#289`, `#299`, `#356`).
It put `#110` and `#36` in Later while the board had them in Now, and `#257` and `#290` in Now while the board had them in Later.
Two open issues, `#148` and `#153`, were on neither.
None of that was visible from either side, because neither side could see the other.

That is [one owner per fact](contributing.md#one-owner-per-fact--link-dont-duplicate) applied to project state.
The board won because `Status` is structured: it can be filtered, grouped and dragged, and a markdown list cannot.

## Modes

| Mode | What it does |
|---|---|
| `./scripts/roadmap.py --render` | Print the body to stdout. Reads the board, writes nothing |
| `./scripts/roadmap.py --publish` | Update the issue, and only when the body actually changed |
| `./scripts/roadmap.py --sync` | Hold the board against the repo (see below) |
| `./scripts/roadmap.py --check` | Exit non-zero on drift. Runs in `hygiene.yml` |

`--dry-run` works on `--publish` and `--sync`.

`--publish` compares against what is already on the issue and makes no API call when they match.
That matters because the cron runs whether or not anyone touched the board, and an unconditional write would bump `updated_at` four times a day for nothing.

`--sync` is idempotent and does four things:

- An **open issue missing from the board** is added to Later.
  Later is the safe default: it means "not triaged into a lane", not "deprioritised".
- A **closed issue** moves to Shipped.
- A **`priority:` label** is copied into the `Priority` field, for open issues only.
  230 of 233 issues had an empty field and a populated label, so the label is the source; the field exists so the board can sort by it.
- A **pull request with no status** is dropped.
  A PR is not a roadmap item, and 256 of them had accumulated with no status, 52% of the board.

## What stays editorial

Two things live in the script rather than on the board, because a board field cannot carry them:

- **`LANE_INTRO`** — the paragraph under each heading.
- **`THEMES`** — the groupings inside a lane, and their order.

`THEMES` is ordered from the reader's side rather than the crate layout's.
`area:mobile` outranks `area:actions` because someone reading the roadmap uses a phone and does not know which crate the work lands in.
`phase:4` outranks every area, so a TUI chrome bug files under the audit it belongs to instead of being one of nineteen bullets under "TUI".

Changing that wording is a code change with a test.
Changing the contents is a drag on the board.

`headline()` also strips a conventional-commit prefix for display.
That one has a regression worth knowing about.
The first version capitalised the first character unconditionally, so `search_blocks` rendered as `Search_blocks` and `iOS:` as `IOS:` — a roadmap naming identifiers that do not exist.
`scripts/tests/test_roadmap.py` pins every case.

## The workflow

[`roadmap.yml`](../.github/workflows/roadmap.yml) runs the script on three triggers.

**A card moved on the board fires nothing**, and that is a platform limit rather than a choice.
Projects v2 emits no event a repository workflow can subscribe to.
`project`, `project_card` and `project_column` are the *classic* Projects API, and `projects_v2_item` is available only to a GitHub App with an organization webhook.
So a board edit reaches the issue on the next cron tick.

| Trigger | Covers |
|---|---|
| `schedule`, every 6 hours | A moved card. A quiet tick costs one read, since `--publish` short-circuits |
| `issues` (`opened`, `closed`, `reopened`, `labeled`, `unlabeled`) | The repo side, where events do exist. A new issue reaches the board in a minute instead of waiting for the tick, which is what kept `#148` and `#153` off it |
| `workflow_dispatch` | After editing the generator's prose |

A push to the generator deliberately does **not** trigger it.
Publishing is how a change reaches people, so a workflow that published from a branch would do it before review.
`hygiene.yml` runs `--check` on the PR instead, and the cron picks up new wording once it is on main.

Runs are serialized with `concurrency: roadmap` and `cancel-in-progress: false`.
Two racing runs would both read the board and the loser's `--sync` would re-add a card the winner had just moved; a half-applied `--sync` is the one state worth queueing to avoid.

## Setup: `ROADMAP_PROJECT_TOKEN`

The workflow needs a PAT stored as the repository secret `ROADMAP_PROJECT_TOKEN`, with **both** scopes:

| Scope | Needed by | Without it |
|---|---|---|
| `project` | `--sync`, and every read | Nothing works: the board cannot be read |
| `repo` (or `public_repo`) | `--publish` | The board updates and the issue does not |

The default `GITHUB_TOKEN` **cannot** reach a user-owned Projects v2 board at any permission level, so there is no `permissions:` block that avoids this.

```sh
# Create a classic PAT with `project` + `repo` at
# https://github.com/settings/tokens, then:
gh secret set ROADMAP_PROJECT_TOKEN --repo outlmd/outl
```

**Both halves are checked up front**, because the first token set carried `project` without `repo` and the failure was not where it looked.
`--sync` succeeded, so the board was correct; `--publish` failed on `updateIssue` with a GraphQL scope error two steps later.
The `check for the project token` step now reads `X-OAuth-Scopes` and names the missing scope before either half runs.
A fine-grained PAT reports no such header, so an empty value continues rather than failing.

Without the secret every step skips and the drift check in `hygiene.yml` is bypassed, rather than failing.
That keeps forks and the pre-setup window green instead of red on every issue anyone opens.

Both steps run under `shell: bash` for its `-o pipefail`.
They pipe into `tee` to reach the run summary, and the default `bash -e` takes the exit status of the last command in a pipe, which is `tee`.
The scope failure above printed the whole GitHub error and the run was reported green.
