"""Check publishable files without printing matched private values.

This is a guard against common leaks, not a substitute for reviewing new files.
Only allowlisted assets may contain binary data in the source tree. Third-party
license attribution remains intact. Use --payload to inspect unpacked releases.
"""

import argparse
import os
import re
import struct
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ROOT_FILES = {
    ".gitignore",
    ".gitattributes",
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "README.md",
    "LICENSE",
    ".editorconfig",
    "CONTRIBUTING.md",
    "SECURITY.md",
    "CODE_OF_CONDUCT.md",
    "deny.toml",
    "pyproject.toml",
    ".markdownlint-cli2.jsonc",
}
ROOT_DIRS = {".github", "assets", "crates", "installer", "rev", "scripts"}
BINARY_ASSETS = {
    "assets/branding/opencrate-logo.png",
    "assets/branding/opencrate-icon.png",
    "assets/branding/opencrate-icon-256.png",
    "assets/branding/opencrate-icon-48.png",
    "assets/branding/opencrate-icon.ico",
    "assets/fonts/NotoSansSC-Regular.ttf",
    "assets/screenshots/settings-light.png",
    "assets/screenshots/settings-dark.png",
    "assets/screenshots/fans-dark.png",
    "assets/readme/opencrate-lighting.png",
}
PRIVATE_SUFFIXES = {
    ".log",
    ".dmp",
    ".pcap",
    ".pcapng",
    ".idb",
    ".i64",
    ".til",
    ".pdb",
    ".pem",
    ".key",
    ".pfx",
    ".p12",
    ".exe",
    ".dll",
}
PRIVATE_CHUNKS = {b"tEXt", b"zTXt", b"iTXt", b"tIME", b"eXIf", b"caBX"}
PATTERNS = {
    "absolute user directory": re.compile(r"(?i)[a-z]:[\\/]+Users[\\/]+[^\s\\/\"']+"),
    "absolute Unix home": re.compile(r"/(?:home|Users)/[a-zA-Z0-9_.-]+/"),
    "private key": re.compile(r"-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----"),
    "GitHub token": re.compile(r"\b(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{30,})\b"),
    "AWS access key": re.compile(r"\b(?:AKIA|ASIA)[A-Z0-9]{16}\b"),
    "credential in URL": re.compile(r"https?://[^\s/@]+:[^\s/@]+@"),
}


def png_metadata(data):
    offset = 8
    while offset + 12 <= len(data):
        size = struct.unpack_from(">I", data, offset)[0]
        kind = data[offset + 4 : offset + 8]
        if kind in PRIVATE_CHUNKS:
            return True
        offset += size + 12
    return False


def image_metadata(path, data):
    if path.suffix == ".png":
        return png_metadata(data)
    if path.suffix == ".ico":
        count = struct.unpack_from("<H", data, 4)[0]
        for i in range(count):
            size, start = struct.unpack_from("<II", data, 6 + i * 16 + 8)
            image = data[start : start + size]
            if image.startswith(b"\x89PNG") and png_metadata(image):
                return True
    return False


def without_tls_parser_literals(view):
    """Ignore exact adjacent parser constants, never a PEM header plus key data.

    Reviewed in native-tls 0.2.18 src/imp/schannel.rs and schannel 0.1.29
    src/crypt_prov.rs. Rust concatenates these immutable strings in the PE.
    Source files and non-executable payload files retain the strict marker check.
    """
    begin = "-----BEGIN " + "PRIVATE KEY-----"
    end = "-----END " + "PRIVATE KEY-----"
    for literal in (
        f"expected '{begin}'and '{end}' PEM guards",
        begin + "not a PKCS#8 key",
        begin + end,
    ):
        view = view.replace(literal, "")
    return view


def inspect(path, label, payload=False):
    findings = []
    data = path.read_bytes()
    if image_metadata(path, data):
        findings.append("image contains nonessential metadata")
    # Inspect text and UTF-16 Windows strings in executables and font tables.
    views = [data.decode("utf-8", errors="replace")]
    if b"\0" in data:
        views += [
            data.decode("utf-16-le", errors="ignore"),
            data[1:].decode("utf-16-le", errors="ignore"),
        ]
    for description, pattern in PATTERNS.items():
        checked_views = views
        if payload and path.suffix.lower() == ".exe" and description == "private key":
            checked_views = [without_tls_parser_literals(view) for view in views]
        if any(pattern.search(view) for view in checked_views):
            findings.append(description)
    if payload:
        roots = [str(ROOT), os.environ.get("USERPROFILE"), os.environ.get("CARGO_HOME")]
        for root in filter(None, roots):
            for value in {root, root.replace("\\", "/")}:
                if any(value.casefold() in view.casefold() for view in views):
                    findings.append("build-machine path in release payload")
    for finding in sorted(set(findings)):
        print(f"FAIL {label}: {finding}")
    return bool(findings)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--payload", type=Path, help="Check an unpacked installer payload")
    args = parser.parse_args()
    failed = False
    if args.payload:
        expected = {"OpenCrate.exe", "README.md", "LICENSE.txt", "THIRD-PARTY-NOTICES.txt"}
        paths = list(args.payload.rglob("*"))
        actual = {p.relative_to(args.payload).as_posix() for p in paths if p.is_file()}
        if actual != expected:
            print("FAIL release payload file list differs from the package allowlist")
            failed = True
        files = [(p, p.name) for p in paths if p.is_file()]
    else:
        result = subprocess.run(
            [
                "git",
                "-c",
                f"safe.directory={ROOT.as_posix()}",
                "ls-files",
                "--cached",
                "--others",
                "--exclude-standard",
                "-z",
            ],
            cwd=ROOT,
            check=True,
            stdout=subprocess.PIPE,
        )
        names = sorted(set(result.stdout.decode("utf-8").split("\0")) - {""})
        files = []
        for name in names:
            path = Path(name)
            allowed = name in ROOT_FILES or path.parts[0] in ROOT_DIRS
            private = path.suffix.lower() in PRIVATE_SUFFIXES or path.name == "settings.json"
            private |= any(
                part in {"upstream", "__pycache__"} or part.startswith(".env")
                for part in path.parts
            )
            if not allowed or private:
                print(f"FAIL {name}: file is outside the publishable source allowlist")
                failed = True
            full = ROOT / path
            if not full.is_file():
                print(f"FAIL {name}: missing staged file")
                failed = True
                continue
            if name not in BINARY_ASSETS and b"\0" in full.read_bytes():
                print(f"FAIL {name}: unreviewed binary source asset")
                failed = True
            files.append((full, name))
    for path, label in files:
        failed |= inspect(path, label, payload=args.payload is not None)
    if not failed:
        print(f"Privacy check passed: {len(files)} files.")
    return int(failed)


if __name__ == "__main__":
    sys.exit(main())
