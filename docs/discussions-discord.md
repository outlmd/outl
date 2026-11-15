# Discussions mirror to Discord

[`discussions-discord.yml`](../.github/workflows/discussions-discord.yml) mirrors GitHub Discussions into a Discord forum channel.
It is one way: GitHub to Discord.
Nothing is written back to GitHub, and the job holds only `contents: read`, `actions: read` and `discussions: read`.
The logic lives in [`.github/scripts/discussions_to_discord.py`](../.github/scripts/discussions_to_discord.py), tested by `test_discussions_to_discord.py` next to it (run by [`hygiene.yml`](../.github/workflows/hygiene.yml)).

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
It finds the newest unexpired one through the Actions API and re-uploads it on every event with 90 days of retention.

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

## Limitation

If a discussion has no activity for more than 90 days, its artifact expires.
The next comment then opens a new Discord post with the same `#<number>` prefix, so the old one is still easy to find by hand.

## Backfill

To post a discussion that existed before the mirror, or whose mapping expired:
Actions, then Discussions to Discord, then Run workflow, and enter the discussion number.
It posts the discussion if it has no mapped thread.
If it already has one, it prints a notice and does nothing.
