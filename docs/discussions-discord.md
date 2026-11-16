# Discussions mirror to Discord

[`discussions-discord.yml`](../.github/workflows/discussions-discord.yml) mirrors GitHub Discussions into a Discord forum channel.
It is one way: GitHub to Discord.
Nothing is written back to GitHub, and the job holds only `contents: read`, `actions: read` and `discussions: read`.
The logic is the Rust crate [`tools/discussions-discord`](../tools/discussions-discord), a workspace member that the workflow builds with `cargo build --locked -p discussions-discord` and then runs.
Its tests run with the rest of the workspace in [`ci.yml`](../.github/workflows/ci.yml), or on their own with `cargo test -p discussions-discord`.

| Module | Owns |
|---|---|
| `text.rs`, `payload.rs` | Discord's length limits (counted in UTF-16 units) and the payloads |
| `discord.rs` | The webhook calls, the single retry on a 429, redacting the URL |
| `github.rs` | The artifact lookup and its provenance checks, the GraphQL fetch for backfill |
| `artifact_zip.rs` | A small reader for the one-file zip an artifact download returns (stored or deflate, built on `flate2`) |
| `http.rs`, `ureq_http.rs` | The `Http` trait the logic talks through, and its `ureq` implementation (rustls, no redirects followed) |

Everything above `ureq_http.rs` is tested against a fake `Http`, so the tests need no network.
The dependencies (`ureq`, `flate2`, `serde`) were already in `Cargo.lock`; there is no `zip` crate.

## Setup

1. In Discord, open the **forum** channel settings, then Integrations, then Webhooks, then New Webhook.
   Copy the webhook URL.
   It must be a forum channel: the script reads the new thread id from the response, which only a forum webhook returns.
2. In GitHub, open the repo Settings, then Secrets and variables, then Actions, then New repository secret.
   Name it `DISCORD_DISCUSSIONS_WEBHOOK` and paste the URL.

Without the secret the workflow prints a warning and exits green, so forks and unconfigured setups never fail.
The job also only runs on `outlmd/outl`.

## What gets posted

| GitHub event | Discord |
|---|---|
| Discussion created | A new forum post named `#<number> <title>` (cut to Discord's 100 characters), with the author, the category, an excerpt of the body and `Reply on GitHub: <link>` |
| Comment or reply created | A message in that discussion's post with the author, an excerpt and `Comment on GitHub: <link>` |
| Discussion edited | Nothing, on purpose: it would be noise |

Every message stays under Discord's 2000 characters.
The excerpt is what gets cut, never the link.
Every request sends `allowed_mentions: {"parse": []}`, so an `@everyone`, a role or a user quoted in a discussion never pings anyone.

Replies go on GitHub.
A message typed in the Discord post is not sent back.

If a comment arrives for a discussion with no Discord post yet, the post is created first and the comment goes into it.

## Where the thread id is kept

A Discord webhook cannot list or search threads, so the discussion number to thread id map has to be stored.
The script keeps one workflow artifact per discussion, `discord-thread-<number>`, holding `{"discussion": N, "thread_id": "..."}`.
It finds the newest trusted, unexpired one through the Actions API and re-uploads it on every event with 90 days of retention.
The download redirects to a blob URL, which is fetched without the `Authorization` header.

Rejected:

- **A hidden GitHub comment, or a state branch or file.**
  Both write to GitHub and need write permissions this job should not have.
- **A repo variable.**
  `GITHUB_TOKEN` cannot write variables, so it would need an extra personal access token secret.
- **`actions/cache`.**
  Entries are evicted after 7 days without access, and a single shared map races between concurrent runs.

Artifacts are immutable, need only read permissions, and never race across discussions.
Runs for the same discussion are serialized by the workflow's `concurrency` group, so a comment right after creation waits for the post instead of opening a second one.
GitHub keeps only one pending run per group, so a burst of comments during one run can skip a middle one on Discord.
GitHub still has all of them.

## Forged artifacts

Any workflow run in the repository can upload an artifact with any name, and a run started from a fork pull request could upload a forged `discord-thread-<number>`.
That would point a discussion's comments at a thread of the attacker's choosing.
So an artifact is only believed when all of these hold:

1. It has not expired.
2. Its run's `head_repository_id` and `repository_id` both equal `GITHUB_REPOSITORY_ID`, so the run did not come from a fork.
3. Its run's `head_branch` is the default branch, read from `repository.default_branch` in the event payload (`main` if absent).
4. Its run is this workflow: `GET /repos/{repo}/actions/runs/{id}` must report a `path` of `.github/workflows/discussions-discord.yml` (any `@ref` suffix is ignored) and an `event` of `discussion`, `discussion_comment` or `workflow_dispatch`.

The newest artifact that passes wins.
One that fails is skipped with a `::notice::`, so a forged artifact never hides a genuine older one.
The listing asks for 100 artifacts per discussion so a flood of forged ones cannot push the real one off the page.
All of it needs only `actions: read`.
If the workflow file is ever renamed, update `WORKFLOW_PATH` in `tools/discussions-discord/src/github.rs`, or every stored mapping stops being trusted and each active discussion gets a new Discord post.

## Limitation

If a discussion has no activity for more than 90 days, its artifact expires.
The next comment then opens a new Discord post with the same `#<number>` prefix, so the old one is still easy to find by hand.

## Backfill

To post a discussion that existed before the mirror, or whose mapping expired:
Actions, then Discussions to Discord, then Run workflow, and enter the discussion number.
It posts the discussion if it has no mapped thread.
If it already has one, it prints a notice and does nothing.
