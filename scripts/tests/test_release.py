"""Exercise release retries and integrity failures without accessing GitHub."""

import contextlib
import hashlib
import importlib.util
import io
import os
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "release", Path(__file__).resolve().parents[1] / "publish-release.py"
)
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)
TAG = "v1.2.3"
TREE = "a" * 40
COMMIT = "b" * 40


def write_pair(directory, data=b"synthetic installer"):
    setup, checksum = release.asset_names(TAG)
    (directory / setup).write_bytes(data)
    (directory / checksum).write_text(f"{hashlib.sha256(data).hexdigest()}  {setup}\n")


class FakeGitHub:
    def __init__(self, existing=None):
        self.existing = existing
        self.commands = []
        self.download_data = b"synthetic installer"
        self.corrupt_download = False

    def commit(self, repo, tag):
        return {"sha": COMMIT, "commit": {"tree": {"sha": TREE}}}

    def release(self, repo, tag):
        return self.existing

    def download(self, repo, tag, directory):
        write_pair(directory, self.download_data)
        if self.corrupt_download:
            (directory / release.asset_names(TAG)[0]).write_bytes(b"corrupted")

    def run(self, *args):
        self.commands.append(args)
        if args[:2] == ("release", "create"):
            self.existing = {"draft": True, "body": f"<!-- opencrate-source-tree: {TREE} -->"}
        elif args[:2] == ("release", "upload"):
            self.existing["assets"] = [
                {"name": n, "state": "uploaded"} for n in release.asset_names(TAG)
            ]
        elif args[:2] == ("release", "edit"):
            self.existing["draft"] = False


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.notes = self.directory / "notes.md"
        self.notes.write_text("Version <version>\n")
        write_pair(self.directory)
        self.environment = patch.dict(os.environ, {"GITHUB_SHA": COMMIT})
        self.environment.start()
        self.addCleanup(self.environment.stop)

    def publish(self, gh):
        with contextlib.redirect_stdout(io.StringIO()):
            release.publish("example/project", TAG, self.directory, self.notes, gh)

    def test_new_release_is_verified_before_publishing(self):
        gh = FakeGitHub()
        self.publish(gh)
        self.assertEqual([command[1] for command in gh.commands], ["create", "upload", "edit"])
        self.assertFalse(gh.existing["draft"])

    def test_completed_release_is_idempotent_even_if_rebuild_hash_differs(self):
        gh = FakeGitHub(
            {
                "draft": False,
                "body": "Legacy release",
                "assets": [{"name": n, "state": "uploaded"} for n in release.asset_names(TAG)],
            }
        )
        gh.download_data = b"previous verified build with a different timestamp"
        self.publish(gh)
        self.assertEqual(gh.commands, [])

    def test_partial_draft_can_resume(self):
        gh = FakeGitHub(
            {"draft": True, "body": f"<!-- opencrate-source-tree: {TREE} -->", "assets": []}
        )
        self.publish(gh)
        self.assertEqual([command[1] for command in gh.commands], ["upload", "edit"])

    def test_corrupt_uploaded_asset_never_becomes_public(self):
        gh = FakeGitHub()
        gh.corrupt_download = True
        with self.assertRaisesRegex(ValueError, "checksum mismatch"):
            self.publish(gh)
        self.assertTrue(gh.existing["draft"])
        self.assertFalse(any(command[1] == "edit" for command in gh.commands))

    def test_unrelated_draft_or_source_tree_is_not_overwritten(self):
        for body in ("unrelated", f"<!-- opencrate-source-tree: {'c' * 40} -->"):
            gh = FakeGitHub({"draft": True, "body": body})
            with self.assertRaises(ValueError):
                self.publish(gh)
            self.assertEqual(gh.commands, [])

    def test_invalid_tag_and_checksum_path_are_rejected(self):
        with self.assertRaises(ValueError):
            release.asset_names("../../unexpected")
        checksum = self.directory / release.asset_names(TAG)[1]
        checksum.write_text("0" * 64 + "  ../other-file.exe\n")
        with self.assertRaisesRegex(ValueError, "unexpected file"):
            self.publish(FakeGitHub())

    def test_moved_tag_cannot_publish_stale_build(self):
        with patch.dict(os.environ, {"GITHUB_SHA": "d" * 40}):
            with self.assertRaisesRegex(ValueError, "tag moved"):
                self.publish(FakeGitHub())

    def test_api_not_found_is_distinct_from_authentication_failure(self):
        for status in (404, 403, 500):
            response = SimpleNamespace(returncode=1, stdout=f'{{"status":"{status}"}}')
            with patch.object(release.subprocess, "run", return_value=response):
                if status == 404:
                    self.assertIsNone(release.GitHub().release("example/project", TAG))
                else:
                    with self.assertRaises(RuntimeError):
                        release.GitHub().release("example/project", TAG)

    def test_published_release_with_missing_assets_fails_without_mutation(self):
        gh = FakeGitHub({"draft": False, "body": "Legacy release", "assets": []})
        with self.assertRaisesRegex(ValueError, "exactly the installer"):
            self.publish(gh)
        self.assertEqual(gh.commands, [])


if __name__ == "__main__":
    unittest.main()
