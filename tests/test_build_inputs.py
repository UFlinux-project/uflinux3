import gzip
import importlib.machinery
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
import builder
from sbom import apk_metadata, generate


def stream(name, contents):
    data = io.BytesIO()
    with tarfile.open(fileobj=data, mode="w") as tar:
        entry = tarfile.TarInfo(name)
        entry.size = len(contents)
        tar.addfile(entry, io.BytesIO(contents))
    return gzip.compress(data.getvalue(), mtime=0)


class ApkTests(unittest.TestCase):
    def test_resigning_preserves_control_and_data(self):
        control = stream(".PKGINFO", b"pkgname = ironbar\npkgver = 0.19.1-r0\n")
        data = stream("usr/bin/ironbar", b"test ELF payload")
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "test.apk"
            path.write_bytes(stream(".SIGN.RSA256.build.ec.pub", b"signature") + control + data)
            self.assertEqual(builder.split_apk(path), (control, data))
            before = builder.unsigned_digest(path)
            path.write_bytes(stream(".SIGN.RSA256.release.ec.pub", b"other signature") + control + data)
            self.assertEqual(builder.unsigned_digest(path), before)
            self.assertEqual(apk_metadata(path)["pkgname"], "ironbar")

    def test_unknown_stream_is_not_treated_as_signature(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "test.apk"
            path.write_bytes(stream("malicious", b"payload") + stream(".PKGINFO", b"pkgname = test") + stream("data", b"x"))
            with self.assertRaisesRegex(ValueError, "Unexpected"):
                builder.split_apk(path)

    def test_signature_names_lists_every_signature_stream(self):
        control = stream(".PKGINFO", b"pkgname = ironbar\n")
        data = stream("usr/bin/ironbar", b"payload")
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "test.apk"
            stream_name = ".SIGN." + builder.SIGN_STREAM + "." + builder.SIGNING_KEY + ".pub"
            path.write_bytes(stream(stream_name, b"signature") + control + data)
            self.assertEqual(builder.signature_names(path), [stream_name])
            path.write_bytes(stream(stream_name, b"one") + stream(".SIGN.RSA512.other.pub", b"two")
                             + control + data)
            self.assertEqual(builder.signature_names(path), [stream_name, ".SIGN.RSA512.other.pub"])

    def test_streaming_reader_matches_concatenated_gzip(self):
        data = stream(".SIGN.RSA256.key", b"sig") + stream(".PKGINFO", b"pkgname = demo\n") + stream("file", bytes(range(256)) * 400)
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "demo-1.apk"
            path.write_bytes(data)
            self.assertEqual(list(builder.gzip_file_members(path)), list(builder.gzip_members(data)))
            cache = Path(tmp) / "cache"
            cache.mkdir()
            builder.promote_apk(path, cache)
            self.assertEqual(len(list(cache.glob("demo-1.*.apk"))), 1)

    def test_missing_data_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "test.apk"
            path.write_bytes(stream(".PKGINFO", b"pkgname = test"))
            with self.assertRaises(ValueError):
                builder.split_apk(path)

    def test_truncated_gzip_rejected(self):
        with self.assertRaises((ValueError, EOFError)):
            list(builder.gzip_members(gzip.compress(b"payload")[:-6]))

    def test_repository_input_does_not_accept_shell_or_http(self):
        for value in ["http://example.org/apk", "https://example.org/apk\nother", "https://user@example.org/apk", "https://:token@example.org/apk", "https://example.org/$(id)"]:
            with self.subTest(value=value), self.assertRaises(ValueError):
                builder.repository_url(value)

    def test_spdx_ids_and_vendor_license_annotations(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            path = root / "example_name-1.0_rc1.apk"
            path.write_bytes(stream(".PKGINFO", b"pkgname = example_name\npkgver = 1.0_rc1\nlicense = Public Domain\n") + stream("file", b"data"))
            generate(root, root / "sbom.json", "0.1.0", "x86_64", 0)
            package = json.loads((root / "sbom.json").read_text())["packages"][0]
            self.assertRegex(package["SPDXID"], r"^SPDXRef-[A-Za-z0-9.-]+$")
            self.assertEqual(package["licenseDeclared"], "NOASSERTION")
            self.assertIn("Public Domain", package["licenseComments"])


loader = importlib.machinery.SourceFileLoader("publisher", str(Path(builder.PROJECT) / "scripts/publish-repository.py"))
spec = importlib.util.spec_from_loader(loader.name, loader)
publisher = importlib.util.module_from_spec(spec)
loader.exec_module(publisher)


class PublicationTests(unittest.TestCase):
    def test_cleanup_refuses_to_traverse_active_mounts(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "build"
            root.mkdir()
            sentinel = root / "sentinel"
            sentinel.write_text("keep")
            with patch.object(builder, "mounted_paths", return_value=[root / "dev"]):
                with self.assertRaisesRegex(RuntimeError, "active mounts"):
                    builder.remove_tree(root)
            self.assertEqual(sentinel.read_text(), "keep")

    def test_retention_keeps_current_and_two_previous_generations(self):
        releases = publisher.selection({"version": "0.4.0"}, [{"version": v} for v in ["0.3.0", "0.2.0", "0.1.0"]])
        self.assertEqual([r["version"] for r in releases], ["0.4.0", "0.3.0", "0.2.0"])

    def test_rerun_does_not_duplicate_generation(self):
        releases = publisher.selection({"version": "0.4.0"}, [{"version": "0.4.0"}, {"version": "0.3.0"}])
        self.assertEqual([r["version"] for r in releases], ["0.4.0", "0.3.0"])

    def test_snapshot_symlinks_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            archive = Path(tmp) / "snapshot.tar.gz"
            with tarfile.open(archive, "w:gz") as tar:
                entry = tarfile.TarInfo("x86_64/escape")
                entry.type = tarfile.SYMTYPE
                entry.linkname = "/etc"
                tar.addfile(entry)
            with self.assertRaisesRegex(ValueError, "regular files"):
                publisher.safe_extract(archive, Path(tmp) / "extract")

    def test_snapshot_path_traversal_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            archive = Path(tmp) / "snapshot.tar.gz"
            with tarfile.open(archive, "w:gz") as tar:
                entry = tarfile.TarInfo("../../escape")
                tar.addfile(entry, io.BytesIO())
            with self.assertRaises(tarfile.FilterError):
                publisher.safe_extract(archive, Path(tmp) / "extract")


if __name__ == "__main__":
    unittest.main()
