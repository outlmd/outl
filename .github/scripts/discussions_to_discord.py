#!/usr/bin/env python3
"""Mirror GitHub Discussions into a Discord forum channel, one way.

Run by `.github/workflows/discussions-discord.yml`.
The event comes from `GITHUB_EVENT_PATH` / `GITHUB_EVENT_NAME`, never from
the workflow's `run:` line, so a discussion title cannot inject shell.

Nothing is written back to GitHub.
The only state is the discussion number -> Discord thread id map, kept as one
workflow artifact per discussion (`discord-thread-<number>`):
this script reads it through the Actions API and writes the file the workflow
then uploads.
See `docs/discussions-discord.md` for why an artifact and not something else.

Stdlib only, so the workflow needs no setup step.
"""

import io
import json
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import zipfile

DISCORD_CONTENT_LIMIT = 2000
DISCORD_THREAD_NAME_LIMIT = 100
ELLIPSIS = "…"
USERNAME = "GitHub Discussions"
# Discord's edge rejects the default `Python-urllib` agent with a 403.
USER_AGENT = "outl-discussions-sync (https://github.com/outlmd/outl)"
ARTIFACT_PREFIX = "discord-thread-"
THREAD_FILE = "thread.json"
MAX_RETRY_SLEEP = 60.0
REDIRECTS = (301, 302, 303, 307, 308)


class SyncError(Exception):
    """A failure the step should report and exit 1 on."""


class Response:
    def __init__(self, status, headers, body):
        self.status = status
        self.headers = {k.lower(): v for k, v in headers.items()}
        self.body = body


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    """Hand redirects back to the caller.

    urllib copies the Authorization header onto the redirected request, and the
    blob URL an artifact download redirects to must be fetched without it.
    """

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def http_request(method, url, headers=None, body=None):
    """One HTTP call, no redirects followed, HTTP errors returned not raised."""
    req = urllib.request.Request(url, data=body, method=method)
    req.add_header("User-Agent", USER_AGENT)
    for key, value in (headers or {}).items():
        req.add_header(key, value)
    opener = urllib.request.build_opener(_NoRedirect)
    try:
        with opener.open(req, timeout=30) as resp:
            return Response(resp.status, dict(resp.headers), resp.read())
    except urllib.error.HTTPError as exc:
        return Response(exc.code, dict(exc.headers), exc.read())


def sleep(seconds):
    time.sleep(seconds)


# ---------------------------------------------------------------- text

def _units(text):
    # Discord counts UTF-16 code units, so an emoji is 2.
    return len(text.encode("utf-16-le")) // 2


def truncate(text, limit):
    """Cut `text` to at most `limit` UTF-16 units, ending in an ellipsis."""
    if _units(text) <= limit:
        return text
    if limit <= 1:
        return ELLIPSIS[:limit]
    cut = min(len(text), limit)
    while cut > 0 and _units(text[:cut]) + 1 > limit:
        cut -= 1
    return text[:cut].rstrip() + ELLIPSIS


def thread_name(number, title):
    one_line = " ".join(str(title).split())
    return truncate("#%s %s" % (number, one_line), DISCORD_THREAD_NAME_LIMIT)


def build_content(head, excerpt, tail, limit=DISCORD_CONTENT_LIMIT):
    """`head`, an excerpt and `tail`, never longer than `limit`.

    The excerpt gives way first: the link line is what a reader acts on, so it
    is never the part that gets cut.
    """
    excerpt = excerpt.replace("\r\n", "\n").strip()
    fixed = head + "\n\n" + tail
    budget = limit - _units(fixed) - 2
    if not excerpt or budget < 2:
        return fixed
    return head + "\n\n" + truncate(excerpt, budget) + "\n\n" + tail


def payload(content, name=None):
    body = {
        "username": USERNAME,
        "content": content,
        # Empty `parse` means nothing in the text can ping: not @everyone,
        # @here, a role, or a user quoted from a discussion body.
        "allowed_mentions": {"parse": []},
    }
    if name is not None:
        body["thread_name"] = name
    return body


def _login(user):
    return (user or {}).get("login") or "ghost"


def discussion_payload(discussion):
    category = truncate((discussion.get("category") or {}).get("name") or "General", 100)
    head = "**%s** opened a discussion in **%s**" % (
        truncate(_login(discussion.get("user")), 60),
        category,
    )
    tail = "Reply on GitHub: <%s>" % discussion["html_url"]
    content = build_content(head, discussion.get("body") or "", tail)
    return payload(content, thread_name(discussion["number"], discussion["title"]))


def comment_payload(comment):
    verb = "replied" if comment.get("parent_id") else "commented"
    head = "**%s** %s" % (truncate(_login(comment.get("user")), 60), verb)
    tail = "Comment on GitHub: <%s>" % comment["html_url"]
    return payload(build_content(head, comment.get("body") or "", tail))


# ------------------------------------------------------------- discord

def _add_query(url, params):
    parts = urllib.parse.urlsplit(url)
    query = urllib.parse.parse_qsl(parts.query) + list(params.items())
    return urllib.parse.urlunsplit(parts._replace(query=urllib.parse.urlencode(query)))


def _retry_after(resp):
    try:
        value = json.loads(resp.body or b"{}").get("retry_after")
    except (ValueError, AttributeError):
        value = None
    if value is None:
        value = resp.headers.get("retry-after")
    try:
        return min(max(float(value), 0.0), MAX_RETRY_SLEEP)
    except (TypeError, ValueError):
        return 1.0


def post_to_discord(webhook, body, thread_id=None):
    """POST to the webhook and return the created message as a dict.

    One retry on 429, waiting what Discord asks for.
    Any other failure raises with the status and Discord's reply, never the URL.
    """
    params = {"wait": "true"}
    if thread_id:
        params["thread_id"] = thread_id
    url = _add_query(webhook, params)
    data = json.dumps(body).encode("utf-8")
    headers = {"Content-Type": "application/json"}
    resp = None
    for attempt in (0, 1):
        try:
            resp = http_request("POST", url, headers, data)
        except OSError as exc:
            raise SyncError("Discord request failed: %s" % exc) from None
        if resp.status == 429 and attempt == 0:
            wait = _retry_after(resp)
            print("Discord rate limited the request, retrying in %.1fs" % wait)
            sleep(wait)
            continue
        break
    if not 200 <= resp.status < 300:
        detail = resp.body[:300].decode("utf-8", "replace")
        raise SyncError("Discord returned HTTP %d: %s" % (resp.status, detail))
    try:
        return json.loads(resp.body or b"{}")
    except ValueError:
        raise SyncError("Discord returned a body that is not JSON") from None


def create_thread(webhook, discussion):
    """Open the forum post and return its thread id.

    On a forum webhook with `wait=true` the message's `channel_id` is the id of
    the thread it created.
    """
    message = post_to_discord(webhook, discussion_payload(discussion))
    thread_id = str(message.get("channel_id") or "")
    if not thread_id.isdigit():
        raise SyncError(
            "Discord did not return a thread id; is the webhook on a forum channel?"
        )
    return thread_id


# -------------------------------------------------------------- github

def _github_headers(token):
    headers = {
        "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
    }
    if token:
        headers["Authorization"] = "Bearer " + token
    return headers


def pick_artifact(artifacts):
    """The newest artifact that has not expired, or None."""
    live = [a for a in artifacts if not a.get("expired")]
    if not live:
        return None
    return max(live, key=lambda a: a.get("created_at") or "")


def _read_mapping(zip_bytes, number):
    with zipfile.ZipFile(io.BytesIO(zip_bytes)) as zf:
        data = json.loads(zf.read(THREAD_FILE))
    thread_id = str(data.get("thread_id") or "")
    if data.get("discussion") != number or not thread_id.isdigit():
        raise SyncError("artifact for discussion %s holds an unexpected mapping" % number)
    return thread_id


def lookup_thread(env, number):
    """Thread id for `number`, or None when no live artifact maps it."""
    token = env.get("GITHUB_TOKEN", "")
    api = env.get("GITHUB_API_URL", "https://api.github.com")
    repo = env["GITHUB_REPOSITORY"]
    query = urllib.parse.urlencode(
        {"name": "%s%d" % (ARTIFACT_PREFIX, number), "per_page": 10}
    )
    url = "%s/repos/%s/actions/artifacts?%s" % (api, repo, query)
    resp = http_request("GET", url, _github_headers(token))
    if resp.status != 200:
        raise SyncError("listing artifacts failed with HTTP %d" % resp.status)
    artifact = pick_artifact(json.loads(resp.body).get("artifacts", []))
    if artifact is None:
        return None
    resp = http_request("GET", artifact["archive_download_url"], _github_headers(token))
    if resp.status in REDIRECTS:
        # The blob host rejects a request that still carries our token.
        resp = http_request("GET", resp.headers["location"])
    if resp.status != 200:
        raise SyncError("downloading the artifact failed with HTTP %d" % resp.status)
    return _read_mapping(resp.body, number)


GRAPHQL = """
query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    discussion(number: $number) {
      number title body url
      author { login }
      category { name }
    }
  }
}
"""


def fetch_discussion(env, number):
    """The discussion through GraphQL, shaped like the webhook payload's."""
    owner, name = env["GITHUB_REPOSITORY"].split("/", 1)
    url = env.get("GITHUB_GRAPHQL_URL", "https://api.github.com/graphql")
    body = json.dumps(
        {"query": GRAPHQL, "variables": {"owner": owner, "name": name, "number": number}}
    ).encode("utf-8")
    headers = _github_headers(env.get("GITHUB_TOKEN", ""))
    headers["Content-Type"] = "application/json"
    resp = http_request("POST", url, headers, body)
    if resp.status != 200:
        raise SyncError("GraphQL request failed with HTTP %d" % resp.status)
    found = ((json.loads(resp.body).get("data") or {}).get("repository") or {}).get(
        "discussion"
    )
    if not found:
        raise SyncError("discussion #%d was not found" % number)
    return {
        "number": found["number"],
        "title": found["title"],
        "body": found.get("body") or "",
        "html_url": found["url"],
        "user": found.get("author"),
        "category": found.get("category"),
    }


# --------------------------------------------------------------- state

def _set_outputs(env, outputs):
    path = env.get("GITHUB_OUTPUT")
    if not path:
        return
    with open(path, "a", encoding="utf-8") as fh:
        for key, value in outputs.items():
            fh.write("%s=%s\n" % (key, value))


def save_mapping(env, number, thread_id):
    """Write the artifact file and tell the workflow to upload it.

    Called as soon as a thread exists, before the comment is posted, so a failed
    comment cannot orphan a thread the next run would then duplicate.
    """
    directory = os.path.join(env.get("RUNNER_TEMP") or "/tmp", "discord-thread")
    os.makedirs(directory, exist_ok=True)
    path = os.path.join(directory, THREAD_FILE)
    with open(path, "w", encoding="utf-8") as fh:
        json.dump({"discussion": number, "thread_id": thread_id}, fh)
    _set_outputs(
        env,
        {
            "save": "true",
            "artifact_name": "%s%d" % (ARTIFACT_PREFIX, number),
            "thread_file": path,
        },
    )


# ----------------------------------------------------------------- run

def _load_event(env):
    with open(env["GITHUB_EVENT_PATH"], encoding="utf-8") as fh:
        return json.load(fh)


def _handle_created(env, webhook, number, discussion):
    if lookup_thread(env, number) is not None:
        print("::notice::discussion #%d already has a Discord thread, nothing to do" % number)
        return
    save_mapping(env, number, create_thread(webhook, discussion))
    print("Opened a Discord thread for discussion #%d" % number)


def _handle_comment(env, webhook, number, discussion, comment):
    thread_id = lookup_thread(env, number)
    if thread_id is None:
        thread_id = create_thread(webhook, discussion)
        save_mapping(env, number, thread_id)
    post_to_discord(webhook, comment_payload(comment), thread_id=thread_id)
    # Re-saved on every event so the 90 day retention restarts with activity.
    save_mapping(env, number, thread_id)
    print("Posted a comment on discussion #%d" % number)


def run(env):
    webhook = (env.get("DISCORD_DISCUSSIONS_WEBHOOK") or "").strip()
    if not webhook:
        print("::warning::DISCORD_DISCUSSIONS_WEBHOOK is not set, skipping the Discord mirror")
        return 0
    print("::add-mask::" + webhook)

    event_name = env.get("GITHUB_EVENT_NAME", "")
    event = _load_event(env)
    if event_name == "workflow_dispatch":
        number = int(event["inputs"]["discussion"])
        _handle_created(env, webhook, number, fetch_discussion(env, number))
    elif event_name == "discussion":
        discussion = event["discussion"]
        _handle_created(env, webhook, int(discussion["number"]), discussion)
    elif event_name == "discussion_comment":
        discussion = event["discussion"]
        _handle_comment(env, webhook, int(discussion["number"]), discussion, event["comment"])
    else:
        raise SyncError("unsupported event %r" % event_name)
    return 0


def main():
    env = os.environ
    try:
        return run(env)
    except (SyncError, KeyError, ValueError, zipfile.BadZipFile, OSError) as exc:
        message = "%s: %s" % (type(exc).__name__, exc) if not isinstance(exc, SyncError) else str(exc)
        webhook = env.get("DISCORD_DISCUSSIONS_WEBHOOK", "")
        if webhook:
            message = message.replace(webhook, "***")
        print("::error::" + message)
        return 1


if __name__ == "__main__":
    sys.exit(main())
