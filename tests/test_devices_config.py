"""Device targets must stay consistent with the build matrix and their boot paths."""
import json
from pathlib import Path
import unittest

root = Path(__file__).resolve().parents[1]
devices = json.loads((root / "config/devices.json").read_text())
arches = json.loads((root / "config/architectures.json").read_text())
lock = json.loads((root / "config/devices.lock.json").read_text())


class DevicesConfigTests(unittest.TestCase):
    def test_expected_targets_are_present(self):
        self.assertEqual(sorted(devices), ["qcs6490", "rpi", "spacemit-k1", "vm"])

    def test_every_target_uses_a_supported_architecture(self):
        for device_id, device in devices.items():
            self.assertTrue(device["arches"], device_id)
            self.assertTrue(set(device["arches"]) <= set(arches), device_id)

    def test_kernel_flavour_matches_the_boot_path(self):
        for device_id, device in devices.items():
            if device["boot"] == "rpi":
                self.assertEqual(device["kernel"], "rpi", device_id)
            if device["boot"] == "extlinux":
                self.assertEqual(device["arches"], ["riscv64"], device_id)

    def test_serial_console_matches_the_architecture(self):
        for device_id, device in devices.items():
            for arch in device["arches"]:
                if arch == "aarch64" and device["boot"] != "efi":
                    self.assertEqual(device["serial"], "ttyAMA0", device_id)
                if arch == "riscv64":
                    self.assertEqual(device["serial"], "ttyS0", device_id)

    def test_only_qcom_uses_the_nura_repository(self):
        self.assertEqual([name for name, device in devices.items() if device["nura"]], ["qcs6490"])
        self.assertEqual(lock["nura"]["arches"], ["aarch64"])
        self.assertEqual(set(lock["nura"]["index_sha256"]), {"aarch64"})

    def test_supported_targets_ship_a_bootloader_package(self):
        for device_id, device in devices.items():
            if device["status"] != "supported":
                continue
            if device["boot"] == "rpi":
                self.assertIn("raspberrypi-bootloader", device["packages"], device_id)
            elif device["boot"] == "efi":
                self.assertIn("systemd-boot", device["packages"], device_id)

    def test_images_leave_room_for_the_root_filesystem(self):
        for device_id, device in devices.items():
            self.assertGreaterEqual(device["img_size_mib"], 4096, device_id)

    def test_boards_use_f2fs_and_virtual_machines_use_xfs(self):
        # f2fs suits the wear-levelling of SD cards and eMMC; a virtual disk has
        # no such constraint and matches the XFS of the installed ISO target.
        for device_id, device in devices.items():
            if device_id == "vm":
                self.assertEqual(device["rootfs"], "xfs", device_id)
            else:
                self.assertEqual(device["rootfs"], "f2fs", device_id)


if __name__ == "__main__":
    unittest.main()
