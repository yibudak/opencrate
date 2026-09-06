"""Publish verified installer assets; safely resume drafts and repeated runs."""

import argparse
import hashlib
import json
import os
import re
import subprocess
import tempfile
from pathlib import Path

TAG = re.compile(r"v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)")
TREE_MARKER = re.compile(r"<!-- opencrate-source-tree: ([0-9a-f]{40}) -->")


def asset_names(tag):
    if not TAG.fullmatch(tag):
        raise ValueError("Release tag must be v<major>.<minor>.<patch>.")
    setup = f"OpenCrate-{tag[1:]}-windows-x64-setup.exe"
    return setup, f"{setup}.sha256"


def verify_pair(directory, tag):
    setup, checksum = asset_names(tag)
    fields = (directory / checksum).read_text(encoding="utf-8-sig").split()
    if len(fields) != 2 or not re.fullmatch(r"[0-9a-fA-F]{64}", fields[0]):
        raise ValueError("Malformed release checksum.")
    if fields[1] != setup:
        raise ValueError("Checksum refers to an unexpected file.")
    data = (directory / setup).read_bytes()
    digest = hashlib.sha256(data).hexdigest()
    if not data or digest != fields[0].lower():
        raise ValueError("Release checksum mismatch or empty installer.")
    return digest


class GitHub:
    def run(self, *args):
        result = subprocess.run(
            ["gh", *map(str, args)], capture_output=True, text=True, encoding="utf-8"
        )
        if result.returncode:
            raise RuntimeError(f"GitHub CLI failed: {result.stderr.strip()}")
        return result.stdout

    def release(self, repo, tag):
        result = subprocess.run(
            ["gh", "api", f"repos/{repo}/releases/tags/{tag}"],
            capture_output=True,
            text=True,
            encoding="utf-8",
        )
        try:
            data = json.loads(result.stdout)
        except json.JSONDecodeError as error:
            raise RuntimeError("Cannot read GitHub release response.") from error
        if result.returncode:
            if str(data.get("status")) == "404":
                return None
            raise RuntimeError("Cannot query release; authentication or API request failed.")
        return data

    def commit(self, repo, tag):
        return json.loads(self.run("api", f"repos/{repo}/commits/{tag}"))

    def download(self, repo, tag, directory):
        setup, checksum = asset_names(tag)
        self.run(
            "release",
            "download",
            tag,
            "--repo",
            repo,
            "--dir",
            directory,
            "--pattern",
            setup,
            "--pattern",
            checksum,
        )


def verify_remote(gh, repo, tag, release, expected_digest=None):
    expected = set(asset_names(tag))
    assets = release.get("assets", [])
    if {asset["name"] for asset in assets} != expected:
        raise ValueError("Release must contain exactly the installer and its checksum.")
    if any(asset.get("state") != "uploaded" for asset in assets):
        raise ValueError("Release asset upload is incomplete.")
    with tempfile.TemporaryDirectory(prefix="opencrate-release-check-") as temporary:
        directory = Path(temporary)
        gh.download(repo, tag, directory)
        digest = verify_pair(directory, tag)
    if expected_digest is not None and digest != expected_digest:
        raise ValueError("Uploaded installer differs from the tested build.")


def publish(repo, tag, directory, notes_path, gh=None):
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repo):
        raise ValueError("Expected a GitHub owner/repository name.")
    setup, checksum = asset_names(tag)
    local_digest = verify_pair(directory, tag)
    gh = gh or GitHub()
    commit = gh.commit(repo, tag)
    if os.environ.get("GITHUB_SHA") and commit["sha"] != os.environ["GITHUB_SHA"]:
        raise ValueError("Release tag moved while the workflow was running.")
    tree = commit["commit"]["tree"]["sha"]
    if not re.fullmatch(r"[0-9a-f]{40}", tree):
        raise ValueError("Invalid source tree identifier.")
    release = gh.release(repo, tag)
    if release:
        marker = TREE_MARKER.search(release.get("body") or "")
        if marker and marker[1] != tree:
            raise ValueError("Existing release belongs to another source tree; use a new version.")
        if not release["draft"]:
            verify_remote(gh, repo, tag, release)
            print(f"Verified existing {tag}; published assets are unchanged.")
            return
        if not marker:
            raise ValueError("Refusing to replace an unrelated draft without source provenance.")
    else:
        notes = notes_path.read_text(encoding="utf-8").replace("<version>", tag[1:])
        notes += f"\n<!-- opencrate-source-tree: {tree} -->\n"
        with tempfile.TemporaryDirectory(prefix="opencrate-release-notes-") as temporary:
            body = Path(temporary) / "notes.md"
            body.write_text(notes, encoding="utf-8")
            gh.run(
                "release",
                "create",
                tag,
                "--repo",
                repo,
                "--verify-tag",
                "--draft",
                "--title",
                f"OpenCrate {tag}",
                "--notes-file",
                body,
            )
    # Only drafts can reach this path. A retry can repair a partial upload.
    gh.run(
        "release",
        "upload",
        tag,
        directory / setup,
        directory / checksum,
        "--repo",
        repo,
        "--clobber",
    )
    release = gh.release(repo, tag)
    if not release or not release["draft"]:
        raise ValueError("Release draft state changed during upload.")
    verify_remote(gh, repo, tag, release, local_digest)
    gh.run("release", "edit", tag, "--repo", repo, "--draft=false", "--latest")
    print(f"Published verified installer for {tag}.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", required=True)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--dist", type=Path, required=True)
    parser.add_argument(
        "--notes",
        type=Path,
        default=Path(__file__).resolve().parent.parent / "installer/release-notes.md",
    )
    args = parser.parse_args()
    publish(args.repo, args.tag, args.dist, args.notes)


if __name__ == "__main__":
    main()
