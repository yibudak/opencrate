"""Regression checks for source and release privacy checks using synthetic data."""

import contextlib
import importlib.util
import io
import struct
import tempfile
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "privacy", Path(__file__).resolve().parents[1] / "check-privacy.py"
)
privacy = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(privacy)


class PrivacyTests(unittest.TestCase):
    def inspect(self, data, suffix=".bin", payload=False):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / f"fixture{suffix}"
            path.write_bytes(data)
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                failed = privacy.inspect(path, path.name, payload=payload)
            return failed, output.getvalue()

    def test_clean_source_and_public_license_attribution(self):
        failed, _ = self.inspect(b"Copyright Example Contributors, contact@example.invalid")
        self.assertFalse(failed)

    def test_user_paths_are_detected_in_ascii_and_both_utf16_alignments(self):
        path = "C:" + "\\" + "Users" + "\\" + "Synthetic User" + "\\source.rs"
        for data in (path.encode(), path.encode("utf-16-le"), b"x" + path.encode("utf-16-le")):
            with self.subTest(encoding=data[:4]):
                failed, output = self.inspect(data)
                self.assertTrue(failed)
                self.assertNotIn("Synthetic User", output)

    def test_credentials_are_detected_without_echoing_values(self):
        secrets = ["ghp_" + "x" * 40, "https://" + "sample:synthetic-password@host.invalid"]
        for secret in secrets:
            failed, output = self.inspect(secret.encode())
            self.assertTrue(failed)
            self.assertNotIn(secret, output)

    def test_release_rejects_build_root_even_outside_user_directory(self):
        failed, _ = self.inspect(str(privacy.ROOT).encode(), payload=True)
        self.assertTrue(failed)

    def test_png_and_embedded_ico_metadata_are_detected(self):
        chunk = b"tEXt"
        png = b"\x89PNG\r\n\x1a\n" + struct.pack(">I", 0) + chunk + bytes(4)
        self.assertTrue(self.inspect(png, ".png")[0])
        ico = struct.pack("<HHH", 0, 1, 1) + bytes(8) + struct.pack("<II", len(png), 22) + png
        self.assertTrue(self.inspect(ico, ".ico")[0])


if __name__ == "__main__":
    unittest.main()
