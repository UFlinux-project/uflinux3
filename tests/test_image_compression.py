"""Reject images silently reverting to Alpine's gzip/XZ defaults."""
from pathlib import Path
import struct
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
from builder import image_compression


class CompressionTests(unittest.TestCase):
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
