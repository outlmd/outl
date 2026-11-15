#!/usr/bin/env python3
"""Pins for `discussions_to_discord.py`, with the HTTP layer replaced.

These fail if a payload can ping someone, a post can pass Discord's length
limits, a comment can open a second thread for a discussion that has one, or
the artifact lookup stops preferring the newest live one.

Run: python3 .github/scripts/test_discussions_to_discord.py
"""

import contextlib
import importlib.util
import io
import json
import os
import tempfile
import unittest
import zipfile
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location(
    "discussions_to_discord", os.path.join(HERE, "discussions_to_discord.py")
)
d2d = importlib.util.module_from_spec(spec)
spec.loader.exec_module(d2d)

WEBHOOK = "https://discord.com/api/webhooks/123/secret-token"
THREAD = "555000111222333444"
NEW_THREAD = "999888777666555444"

DISCUSSION = {
    "number": 42,
    "title": "How do I sync two devices?",
    "body": "Hello @everyone and @here, ping <@&123> please.",
    "html_url": "https://github.com/outlmd/outl/discussions/42",
    "user": {"login": "alice"},
    "category": {"name": "Q&A"},
}
COMMENT = {
    "html_url": "https://github.com/outlmd/outl/discussions/42#discussioncomment-7",
    "body": "Try `outl peer`.",
    "user": {"login": "bob"},
}


def reply(status=200, body=None, headers=None):
    raw = json.dumps(body).encode() if body is not None else b""
    return d2d.Response(status, headers or {}, raw)


def artifact_zip(number, thread_id):
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as zf:
        zf.writestr("thread.json", json.dumps({"discussion": number, "thread_id": thread_id}))
    return buf.getvalue()


class FakeNet:
    """Routes the calls the script makes and records them."""

    def __init__(self, artifacts=None, mapped=None, discord=None):
        self.calls = []
        self.artifacts = artifacts if artifacts is not None else []
        self.zips = mapped or {}
        self.discord = list(discord or [])

    def __call__(self, method, url, headers=None, body=None):
        self.calls.append((method, url, headers or {}, body))
        if url.startswith(WEBHOOK):
            return self.discord.pop(0) if self.discord else reply(200, {"channel_id": NEW_THREAD})
        if "/actions/artifacts?" in url:
            return reply(200, {"artifacts": self.artifacts})
        if url.startswith("https://api.github.com/artifact-zip/"):
            return d2d.Response(302, {"Location": "https://blob.example/" + url[-1]}, b"")
        if url.startswith("https://blob.example/"):
            return d2d.Response(200, {}, self.zips[url[-1]])
        raise AssertionError("unexpected call " + url)

    def discord_calls(self):
        return [c for c in self.calls if c[1].startswith(WEBHOOK)]

    def discord_bodies(self):
        return [json.loads(c[3]) for c in self.discord_calls()]


def live_artifact(ident, created_at, expired=False):
    return {
        "id": ident,
        "created_at": created_at,
        "expired": expired,
        "archive_download_url": "https://api.github.com/artifact-zip/" + str(ident),
    }


class Harness(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.out = os.path.join(self.tmp.name, "output")
        open(self.out, "w").close()
        patcher = mock.patch.object(d2d, "sleep")
        self.sleep = patcher.start()
        self.addCleanup(patcher.stop)

    def env(self, event_name, event, webhook=WEBHOOK):
        path = os.path.join(self.tmp.name, "event.json")
        with open(path, "w") as fh:
            json.dump(event, fh)
        env = {
            "GITHUB_EVENT_NAME": event_name,
            "GITHUB_EVENT_PATH": path,
            "GITHUB_OUTPUT": self.out,
            "GITHUB_REPOSITORY": "outlmd/outl",
            "GITHUB_TOKEN": "ghs_token",
            "RUNNER_TEMP": self.tmp.name,
        }
        if webhook is not None:
            env["DISCORD_DISCUSSIONS_WEBHOOK"] = webhook
        return env

    def run_script(self, env, net):
        buf = io.StringIO()
        with mock.patch.object(d2d, "http_request", net), contextlib.redirect_stdout(buf):
            code = d2d.run(env)
        return code, buf.getvalue()

    def outputs(self):
        with open(self.out) as fh:
            return dict(line.rstrip("\n").split("=", 1) for line in fh if "=" in line)

    def saved(self):
        with open(os.path.join(self.tmp.name, "discord-thread", "thread.json")) as fh:
            return json.load(fh)


class TextLimits(unittest.TestCase):
    def test_thread_name_is_cut_to_100(self):
        name = d2d.thread_name(7, "x" * 300)
        self.assertEqual(len(name), 100)
        self.assertTrue(name.startswith("#7 xxx"))
        self.assertTrue(name.endswith(d2d.ELLIPSIS))

    def test_short_thread_name_is_untouched_and_one_line(self):
        self.assertEqual(d2d.thread_name(7, "a\nb"), "#7 a b")

    def test_content_never_exceeds_2000_and_keeps_the_link(self):
        for body in ("y" * 50000, "😀" * 5000, "", "line\n" * 900):
            payload = d2d.discussion_payload({**DISCUSSION, "body": body})
            content = payload["content"]
            self.assertLessEqual(d2d._units(content), 2000)
            self.assertTrue(content.endswith("Reply on GitHub: <%s>" % DISCUSSION["html_url"]))

    def test_comment_content_keeps_its_link_when_the_excerpt_is_cut(self):
        payload = d2d.comment_payload({**COMMENT, "body": "z" * 9000})
        self.assertLessEqual(d2d._units(payload["content"]), 2000)
        self.assertTrue(payload["content"].endswith("Comment on GitHub: <%s>" % COMMENT["html_url"]))
        self.assertIn(d2d.ELLIPSIS, payload["content"])

    def test_a_reply_says_so(self):
        self.assertIn("replied", d2d.comment_payload({**COMMENT, "parent_id": 3})["content"])
        self.assertIn("commented", d2d.comment_payload(COMMENT)["content"])

    def test_mentions_in_the_body_stay_in_the_text(self):
        content = d2d.discussion_payload(DISCUSSION)["content"]
        self.assertIn("@everyone", content)
        self.assertEqual(d2d.discussion_payload(DISCUSSION)["allowed_mentions"], {"parse": []})

    def test_only_the_forum_post_carries_a_thread_name(self):
        self.assertIn("thread_name", d2d.discussion_payload(DISCUSSION))
        self.assertNotIn("thread_name", d2d.comment_payload(COMMENT))


class Flow(Harness):
    def test_missing_secret_skips_with_a_warning(self):
        net = FakeNet()
        code, out = self.run_script(self.env("discussion", {"discussion": DISCUSSION}, webhook=""), net)
        self.assertEqual(code, 0)
        self.assertIn("::warning::", out)
        self.assertEqual(net.calls, [])

    def test_created_posts_a_forum_thread_and_saves_its_id(self):
        net = FakeNet()
        code, _ = self.run_script(self.env("discussion", {"discussion": DISCUSSION}), net)
        self.assertEqual(code, 0)
        (call,) = net.discord_calls()
        self.assertIn("wait=true", call[1])
        self.assertNotIn("thread_id", call[1])
        body = json.loads(call[3])
        self.assertEqual(body["thread_name"], "#42 How do I sync two devices?")
        self.assertIn("alice", body["content"])
        self.assertIn("Q&A", body["content"])
        self.assertEqual(self.saved(), {"discussion": 42, "thread_id": NEW_THREAD})
        outs = self.outputs()
        self.assertEqual(outs["save"], "true")
        self.assertEqual(outs["artifact_name"], "discord-thread-42")

    def test_created_for_a_mapped_discussion_posts_nothing(self):
        net = FakeNet(artifacts=[live_artifact(1, "2026-01-01T00:00:00Z")],
                      mapped={"1": artifact_zip(42, THREAD)})
        code, out = self.run_script(self.env("discussion", {"discussion": DISCUSSION}), net)
        self.assertEqual(code, 0)
        self.assertEqual(net.discord_calls(), [])
        self.assertIn("::notice::", out)
        self.assertNotIn("save", self.outputs())

    def test_comment_without_a_mapping_creates_the_post_then_comments_in_it(self):
        net = FakeNet()
        event = {"discussion": DISCUSSION, "comment": COMMENT}
        code, _ = self.run_script(self.env("discussion_comment", event), net)
        self.assertEqual(code, 0)
        first, second = net.discord_calls()
        self.assertNotIn("thread_id", first[1])
        self.assertIn("thread_id=" + NEW_THREAD, second[1])
        self.assertNotIn("thread_name", json.loads(second[3]))
        self.assertIn(COMMENT["html_url"], json.loads(second[3])["content"])
        self.assertEqual(self.saved()["thread_id"], NEW_THREAD)

    def test_comment_with_a_mapping_posts_into_that_thread_only(self):
        net = FakeNet(artifacts=[live_artifact(1, "2026-01-01T00:00:00Z")],
                      mapped={"1": artifact_zip(42, THREAD)})
        event = {"discussion": DISCUSSION, "comment": COMMENT}
        code, _ = self.run_script(self.env("discussion_comment", event), net)
        self.assertEqual(code, 0)
        (call,) = net.discord_calls()
        self.assertIn("thread_id=" + THREAD, call[1])
        # Saved again so the retention clock restarts.
        self.assertEqual(self.saved(), {"discussion": 42, "thread_id": THREAD})
        self.assertEqual(self.outputs()["save"], "true")

    def test_a_failed_comment_still_saves_the_new_thread(self):
        net = FakeNet(discord=[reply(200, {"channel_id": NEW_THREAD}), reply(500, {"message": "boom"})])
        event = {"discussion": DISCUSSION, "comment": COMMENT}
        with self.assertRaises(d2d.SyncError):
            self.run_script(self.env("discussion_comment", event), net)
        self.assertEqual(self.saved()["thread_id"], NEW_THREAD)

    def test_every_payload_blocks_mentions(self):
        net = FakeNet()
        event = {"discussion": DISCUSSION, "comment": COMMENT}
        self.run_script(self.env("discussion_comment", event), net)
        bodies = net.discord_bodies()
        self.assertEqual(len(bodies), 2)
        for body in bodies:
            self.assertEqual(body["allowed_mentions"], {"parse": []})

    def test_dispatch_fetches_the_discussion_and_posts_it(self):
        graphql = {"data": {"repository": {"discussion": {
            "number": 42, "title": "Old one", "body": "text", "url": DISCUSSION["html_url"],
            "author": {"login": "carol"}, "category": {"name": "Ideas"}}}}}
        net = FakeNet()
        inner = net.__call__

        def route(method, url, headers=None, body=None):
            if url == "https://api.github.com/graphql":
                net.calls.append((method, url, headers or {}, body))
                return reply(200, graphql)
            return inner(method, url, headers, body)

        code, _ = self.run_script(
            self.env("workflow_dispatch", {"inputs": {"discussion": "42"}}), route
        )
        self.assertEqual(code, 0)
        (body,) = net.discord_bodies()
        self.assertEqual(body["thread_name"], "#42 Old one")
        self.assertIn("carol", body["content"])
        self.assertEqual(self.saved()["thread_id"], NEW_THREAD)

    def test_the_webhook_never_reaches_the_error_output(self):
        env = self.env("discussion", {"discussion": DISCUSSION})
        net = FakeNet(discord=[reply(400, {"message": "bad"})])
        buf = io.StringIO()
        with mock.patch.object(d2d, "http_request", net), \
                mock.patch.dict(os.environ, env, clear=True), \
                contextlib.redirect_stdout(buf):
            code = d2d.main()
        self.assertEqual(code, 1)
        self.assertIn("HTTP 400", buf.getvalue())
        self.assertNotIn("secret-token", buf.getvalue().replace("::add-mask::" + WEBHOOK, ""))


class Discord(Harness):
    def test_429_is_retried_once_after_retry_after(self):
        net = FakeNet(discord=[reply(429, {"retry_after": 2.5}), reply(200, {"channel_id": THREAD})])
        with mock.patch.object(d2d, "http_request", net):
            message = d2d.post_to_discord(WEBHOOK, d2d.payload("hi"))
        self.assertEqual(message["channel_id"], THREAD)
        self.assertEqual(len(net.discord_calls()), 2)
        self.sleep.assert_called_once_with(2.5)

    def test_a_second_429_fails(self):
        net = FakeNet(discord=[reply(429, {"retry_after": 1}), reply(429, {"retry_after": 1})])
        with mock.patch.object(d2d, "http_request", net):
            with self.assertRaises(d2d.SyncError) as ctx:
                d2d.post_to_discord(WEBHOOK, d2d.payload("hi"))
        self.assertIn("429", str(ctx.exception))
        self.assertEqual(len(net.discord_calls()), 2)

    def test_other_errors_fail_without_the_url(self):
        net = FakeNet(discord=[reply(404, {"message": "Unknown Webhook"})])
        with mock.patch.object(d2d, "http_request", net):
            with self.assertRaises(d2d.SyncError) as ctx:
                d2d.post_to_discord(WEBHOOK, d2d.payload("hi"))
        self.assertIn("404", str(ctx.exception))
        self.assertNotIn("secret-token", str(ctx.exception))
        self.assertEqual(self.sleep.call_count, 0)

    def test_a_response_without_a_thread_id_is_an_error(self):
        net = FakeNet(discord=[reply(200, {"id": "1"})])
        with mock.patch.object(d2d, "http_request", net):
            with self.assertRaises(d2d.SyncError):
                d2d.create_thread(WEBHOOK, DISCUSSION)


class Artifacts(Harness):
    def test_the_newest_live_artifact_wins(self):
        picked = d2d.pick_artifact([
            live_artifact(1, "2026-01-01T00:00:00Z"),
            live_artifact(3, "2026-03-01T00:00:00Z", expired=True),
            live_artifact(2, "2026-02-01T00:00:00Z"),
        ])
        self.assertEqual(picked["id"], 2)

    def test_only_expired_artifacts_means_no_mapping(self):
        self.assertIsNone(d2d.pick_artifact([live_artifact(1, "2026-01-01T00:00:00Z", expired=True)]))
        self.assertIsNone(d2d.pick_artifact([]))

    def test_lookup_reads_the_newest_zip_and_drops_auth_on_the_redirect(self):
        net = FakeNet(
            artifacts=[live_artifact(1, "2026-01-01T00:00:00Z"), live_artifact(2, "2026-02-01T00:00:00Z")],
            mapped={"1": artifact_zip(42, "111"), "2": artifact_zip(42, THREAD)},
        )
        with mock.patch.object(d2d, "http_request", net):
            self.assertEqual(d2d.lookup_thread(self.env("discussion", {}), 42), THREAD)
        listing, download, blob = net.calls
        self.assertIn("name=discord-thread-42", listing[1])
        self.assertIn("Authorization", download[2])
        self.assertEqual(blob[1], "https://blob.example/2")
        self.assertNotIn("Authorization", blob[2])

    def test_lookup_with_nothing_stored_returns_none(self):
        with mock.patch.object(d2d, "http_request", FakeNet()):
            self.assertIsNone(d2d.lookup_thread(self.env("discussion", {}), 42))

    def test_a_mapping_for_another_discussion_is_rejected(self):
        net = FakeNet(artifacts=[live_artifact(1, "2026-01-01T00:00:00Z")],
                      mapped={"1": artifact_zip(41, THREAD)})
        with mock.patch.object(d2d, "http_request", net):
            with self.assertRaises(d2d.SyncError):
                d2d.lookup_thread(self.env("discussion", {}), 42)


if __name__ == "__main__":
    unittest.main()
