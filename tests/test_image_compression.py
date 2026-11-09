"""Reject images silently reverting to Alpine's gzip/XZ defaults."""
from pathlib import Path
import struct
import os
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
from builder import image_compression, omit_modloop_firmware, check_modloop_firmware_listing


class CompressionTests(unittest.TestCase):
    def test_modloop_firmware_removed_before_compression(self):
        marker = "# Refuse an image that its selected kernel cannot decompress."
        result = omit_modloop_firmware("prepare\n" + marker + "\ncompress\n")
        self.assertLess(result.index('find "$MODLOOP/modules/firmware"'), result.index(marker))
        self.assertIn("! -name 'regulatory.db*'", result)
        for invalid in ("no compression marker", marker + marker):
            with self.assertRaises(ValueError):
                omit_modloop_firmware(invalid)

    def test_cleanup_preserves_regulatory_database(self):
        with tempfile.TemporaryDirectory() as temporary:
            modloop = Path(temporary)
            firmware = modloop / "modules/firmware"
            (firmware / "amdgpu").mkdir(parents=True)
            (firmware / "amdgpu/gpu.bin.zst").write_bytes(b"firmware")
            (firmware / "regulatory.db").write_bytes(b"database")
            (firmware / "regulatory.db.p7s").write_bytes(b"signature")
            marker = "# Refuse an image that its selected kernel cannot decompress."
            script = omit_modloop_firmware(marker)
            subprocess.run(["sh", "-e", "-c", script], env=dict(os.environ, MODLOOP=str(modloop)), check=True)
            self.assertEqual({p.name for p in firmware.iterdir()}, {"regulatory.db", "regulatory.db.p7s"})

    def test_modloop_listing_rejects_device_firmware(self):
        check_modloop_firmware_listing("drwx squashfs-root/modules/firmware\n"
                                       "-rw- squashfs-root/modules/firmware/regulatory.db\n"
                                       "-rw- squashfs-root/modules/firmware/regulatory.db.p7s\n")
        for name in ("amdgpu", "amdgpu/file.bin.zst", "other.zst"):
            with self.subTest(name=name), self.assertRaises(ValueError):
                check_modloop_firmware_listing("-rw- squashfs-root/modules/firmware/" + name)

    def test_formats_and_block_size_are_checked(self):
        for mode in ("valid", "gzip", "xz", "wrong-block", "truncated"):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as temporary:
                initramfs, modloop = Path(temporary) / "initramfs", Path(temporary) / "modloop"
                initramfs.write_bytes(b"\x1f\x8b\x08\x00" if mode == "gzip" else b"\x28\xb5\x2f\xfd")
                header = bytearray(32)
                header[:4] = b"hsqs"
                struct.pack_into("<I", header, 12, 1048576 if mode == "wrong-block" else 131072)
                struct.pack_into("<H", header, 20, 4 if mode == "xz" else 6)
                modloop.write_bytes(header[:10] if mode == "truncated" else header)
                if mode == "valid":
                    self.assertEqual(image_compression(initramfs, modloop)["modloop_compression"], "zstd")
                else:
                    with self.assertRaises(ValueError):
                        image_compression(initramfs, modloop)
