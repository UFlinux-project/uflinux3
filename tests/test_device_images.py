"""Device disk images are laid out, described and checked deterministically."""
import contextlib
import io
import json
from pathlib import Path
import os
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
import builder
from builder import (DEVICES, device_environment, device_image_name, device_partitions,
                     device_partition_script, device_packages, device_repositories,
                     device_target, device_uuids, device_verify_image)


class DeviceLayoutTests(unittest.TestCase):
    def test_geometry_is_sector_aligned(self):
        layout = device_partitions(6144)
        self.assertEqual(layout["esp_start"], builder.ESP_START)
        self.assertEqual(layout["root_start"], builder.ESP_START + builder.ESP_SECTORS)
        self.assertEqual(layout["total"], 6144 * 2048)
        self.assertEqual(layout["esp_size"], builder.ESP_SECTORS)
        # A 1 MiB backup GPT is reserved at the end of the disk.
        self.assertEqual(layout["root_size"], layout["total"] - layout["root_start"] - 2048)
        self.assertLess(layout["root_start"] + layout["root_size"], layout["total"])
        with self.assertRaises(ValueError):
            device_partitions(256)

    def test_uuids_are_stable_and_well_formed(self):
        first = device_uuids("0.1.0:rpi:aarch64")
        self.assertEqual(first, device_uuids("0.1.0:rpi:aarch64"))
        self.assertNotEqual(first["disk"], device_uuids("0.1.0:vm:aarch64")["disk"])
        for role in ("disk", "esp", "root"):
            value = first[role]
            self.assertEqual(len(value), 36, role)
            self.assertEqual(value, value.upper(), role)
            self.assertEqual([len(part) for part in value.split("-")], [8, 4, 4, 4, 12], role)
        self.assertEqual(len(first["serial"]), 8)
        self.assertEqual(first["serial"], first["serial"].lower())

    def test_partition_script_pins_guids_and_types(self):
        uuids = device_uuids("0.1.0:rpi:aarch64")
        script = device_partition_script(device_partitions(6144), uuids)
        self.assertIn("label: gpt", script)
        self.assertIn("label-id: " + uuids["disk"], script)
        self.assertIn("type=" + builder.GPT_ESP_TYPE, script)
        self.assertIn("type=" + builder.GPT_LINUX_TYPE, script)
        self.assertIn("uuid=" + uuids["esp"], script)
        self.assertIn("uuid=" + uuids["root"], script)
        self.assertIn("name=ESP, bootable", script)

    def test_image_names_match_the_release_assets(self):
        self.assertEqual(device_image_name("0.1.0", "rpi", "aarch64"),
                         "uflinux-0.1.0-rpi-aarch64.img.zst")

    def test_architecture_without_a_target_is_rejected(self):
        self.assertIs(device_target(DEVICES, "rpi", "aarch64"), DEVICES["rpi"])
        with self.assertRaises(ValueError):
            device_target(DEVICES, "rpi", "x86_64")
        with self.assertRaises(ValueError):
            device_target(DEVICES, "spacemit-k1", "aarch64")


class DeviceRootfsTests(unittest.TestCase):
    def test_build_uses_the_copied_signed_repository(self):
        lines = device_repositories("/repo", "aarch64").split()
        self.assertEqual(lines[0], "/repo")
        self.assertEqual(len(lines), 3, lines)
        self.assertTrue(all(line == "/repo" or line.startswith("https://") for line in lines),
                        lines)

    def test_shipped_image_points_at_the_published_repository(self):
        lines = device_repositories("https://example.invalid/uflinux/0.1.0", "aarch64").split()
        self.assertEqual(lines[0], "https://example.invalid/uflinux/0.1.0")
        self.assertTrue(all(line.startswith("https://") for line in lines), lines)
        self.assertEqual(len(lines), 3, lines)

    def test_nura_suite_carries_no_architecture(self):
        nura = {"mirror": "https://mirror.nura.eco/postmarketos", "branch": "v26.06"}
        lines = device_repositories("/repo", "aarch64", nura).split()
        self.assertEqual(lines[-1], "https://mirror.nura.eco/postmarketos/v26.06")

    def test_shared_desktop_and_boot_tools_are_installed(self):
        packages = device_packages(DEVICES["vm"], "x86_64")
        for required in ("uflinux-desktop-meta", "uflinux-device", "alpine-base", "zstd",
                         "e2fsprogs-extra", "cloud-utils-growpart", "linux-virt",
                         "musl-locales", "labwc-lang", "firefox", "vulkan-loader"):
            self.assertIn(required, packages)
        self.assertNotIn("uflinux-live", packages)
        self.assertEqual(packages.count("efibootmgr"), 1)
        self.assertIn("dtbloader", device_packages(DEVICES["qcs6490"], "aarch64"))
        self.assertNotIn("efibootmgr", device_packages(DEVICES["spacemit-k1"], "riscv64"))

    def test_only_f2fs_devices_pull_in_the_f2fs_tools(self):
        # resize.f2fs is what grows an f2fs root on first boot; the XFS vm image
        # already carries xfsprogs for the shared desktop set.
        for name in ("qcs6490", "rpi", "spacemit-k1"):
            arch = DEVICES[name]["arches"][0]
            self.assertIn("f2fs-tools", device_packages(DEVICES[name], arch), name)
        self.assertNotIn("f2fs-tools", device_packages(DEVICES["vm"], "x86_64"))
        self.assertIn("xfsprogs", device_packages(DEVICES["vm"], "x86_64"))

    def test_environment_covers_the_profile_contract(self):
        env = device_environment("rpi", DEVICES["rpi"], "aarch64", "0.1.0",
                                 "01234567-89AB-4CDE-8F01-23456789ABCD", "/device-esp")
        self.assertEqual(env["UFLINUX_DEVICE"], "rpi")
        self.assertEqual(env["UFLINUX_BOOT"], "rpi")
        self.assertEqual(env["UFLINUX_SERIAL"], "ttyAMA0")
        self.assertEqual(env["UFLINUX_ROOT_PARTUUID"],
                         "01234567-89AB-4CDE-8F01-23456789ABCD")
        self.assertEqual(env["UFLINUX_ESP"], "/device-esp")
        self.assertEqual(env["UFLINUX_ROOTFS"], "f2fs")
        self.assertNotIn("UFLINUX_DTB", env)
        self.assertNotIn("UFLINUX_NURA_PACKAGES", env)
        qcom = device_environment("qcs6490", DEVICES["qcs6490"], "aarch64", "0.1.0",
                                  "01234567-89AB-4CDE-8F01-23456789ABCD", "/device-esp")
        self.assertEqual(qcom["UFLINUX_NURA_PACKAGES"], "dtbloader")
        self.assertEqual(qcom["UFLINUX_DTBLOADER"], "/usr/share/dtbloader/dtbloader.efi")
        k1 = device_environment("spacemit-k1", DEVICES["spacemit-k1"], "riscv64", "0.1.0",
                                "01234567-89AB-4CDE-8F01-23456789ABCD", "/device-esp")
        self.assertEqual(k1["UFLINUX_DTB"], DEVICES["spacemit-k1"]["dtb"])
        self.assertEqual(k1["UFLINUX_SERIAL"], "ttyS0")
        vm = device_environment("vm", DEVICES["vm"], "x86_64", "0.1.0",
                                "01234567-89AB-4CDE-8F01-23456789ABCD", "/device-esp")
        self.assertEqual(vm["UFLINUX_ROOTFS"], "xfs")


class DeviceImageCheckTests(unittest.TestCase):
    def report(self, boot, esp_files, initramfs=b"\x28\xb5\x2f\xfd" + b"initramfs",
               rootfs="f2fs"):
        """Run the checker against a synthetic disk whose ESP is faked."""
        uuids = device_uuids("0.1.0:check:" + boot)
        layout = device_partitions(1024)
        with tempfile.TemporaryDirectory() as temporary:
            image = Path(temporary) / "disk"
            offset, magic, _ = builder.ROOTFS_SIGNATURES[rootfs]
            image.write_bytes(b"\0" * (layout["root_start"] * builder.SECTOR + offset))
            with image.open("ab") as handle:
                handle.write(magic)
            stage = image.parent / (image.name + "-esp")

            def extract(source, destination, esp_start=builder.ESP_START):
                destination.mkdir(parents=True, exist_ok=True)
                for name, content in esp_files.items():
                    target = destination / name
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_bytes(content)

            def geometry(source):
                return {"disk": uuids["disk"],
                        "esp": {"start": layout["esp_start"], "size": layout["esp_size"],
                                "uuid": uuids["esp"]},
                        "root": {"start": layout["root_start"], "size": layout["root_size"],
                                 "uuid": uuids["root"]}}

            saved_layout, saved_extract = builder.device_layout, builder.device_extract
            builder.device_layout, builder.device_extract = geometry, extract
            try:
                report = device_verify_image(image, {"boot": boot, "rootfs": rootfs},
                                             "aarch64", uuids, layout)
            finally:
                builder.device_layout, builder.device_extract = saved_layout, saved_extract
            self.assertFalse(stage.exists(), "the ESP extraction must be cleaned up")
            return report

    def test_report_records_geometry_and_boot_payload(self):
        report = self.report("efi", {"boot/vmlinuz": b"kernel", "boot/initramfs": b"\x28\xb5\x2f\xfd" + b"x",
                                     "EFI/boot/bootaa64.efi": b"MZ"})
        self.assertEqual(report["device_image_format"], "gpt-f2fs")
        self.assertEqual(report["initramfs_compression"], "zstd")
        self.assertEqual(report["boot"], "efi")
        self.assertEqual(report["partitions"]["root"]["uuid"],
                         device_uuids("0.1.0:check:efi")["root"])
        self.assertEqual(report["vmlinuz_bytes"], len(b"kernel"))

    def test_a_gzip_initramfs_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "Zstandard"):
            self.report("efi", {"boot/vmlinuz": b"kernel",
                                "boot/initramfs": b"\x1f\x8b\x08\x00gzip"})

    def test_report_accepts_the_root_filesystem_of_the_device(self):
        esp = {"boot/vmlinuz": b"kernel", "boot/initramfs": b"\x28\xb5\x2f\xfd" + b"x",
               "EFI/boot/bootaa64.efi": b"MZ"}
        for rootfs in ("ext4", "f2fs", "xfs"):
            with self.subTest(rootfs=rootfs):
                report = self.report("efi", esp, rootfs=rootfs)
                self.assertEqual(report["device_image_format"], "gpt-" + rootfs)

    def test_a_root_partition_of_the_wrong_type_is_rejected(self):
        layout = device_partitions(1024)
        with tempfile.TemporaryDirectory() as temporary:
            image = Path(temporary) / "disk"
            image.write_bytes(b"\0" * (layout["root_start"] * builder.SECTOR + 4096))
            with image.open("r+b") as handle:
                handle.seek(layout["root_start"] * builder.SECTOR)
                handle.write(b"XFSB")
            # The image holds an XFS superblock, so an f2fs device must not pass.
            with self.assertRaisesRegex(ValueError, "not f2fs"):
                builder.device_verify_rootfs(image, layout["root_start"], "f2fs")
            builder.device_verify_rootfs(image, layout["root_start"], "xfs")

    def test_missing_boot_files_are_reported(self):
        with self.assertRaisesRegex(ValueError, "initramfs"):
            self.report("efi", {"boot/vmlinuz": b"kernel"})
        with self.assertRaisesRegex(ValueError, "bootaa64.efi"):
            self.report("efi", {"boot/vmlinuz": b"kernel",
                                "boot/initramfs": b"\x28\xb5\x2f\xfd"})
        with self.assertRaisesRegex(ValueError, "cmdline.txt"):
            self.report("rpi", {"boot/vmlinuz": b"kernel",
                                "boot/initramfs": b"\x28\xb5\x2f\xfd"})

    def test_geometry_drift_is_rejected(self):
        uuids = device_uuids("0.1.0:check:drift")
        layout = device_partitions(1024)
        with tempfile.TemporaryDirectory() as temporary:
            image = Path(temporary) / "disk"
            image.write_bytes(b"")
            saved = builder.device_layout
            builder.device_layout = lambda source: {
                "disk": uuids["disk"],
                "esp": {"start": layout["esp_start"] + 1, "uuid": uuids["esp"]},
                "root": {"start": layout["root_start"], "uuid": uuids["root"]}}
            try:
                with self.assertRaisesRegex(ValueError, "ESP starts at sector"):
                    device_verify_image(image, {"boot": "efi"}, "aarch64", uuids, layout)
            finally:
                builder.device_layout = saved

    def test_lock_pins_every_device_index(self):
        for name, device in DEVICES.items():
            if not device["nura"]:
                continue
            self.assertTrue(set(device["arches"]) <= set(builder.DEVICE_LOCK["nura"]["arches"]),
                            name)


class DeviceStageCliTests(unittest.TestCase):
    """The device stage is reachable only as --stage device --device <target>."""

    def rejected(self, *argv):
        with contextlib.redirect_stderr(io.StringIO()) as errors:
            with self.assertRaises(SystemExit) as caught:
                builder.main(list(argv))
        self.assertEqual(caught.exception.code, 2)
        return errors.getvalue()

    def test_device_requires_the_device_stage(self):
        self.assertIn("--device", self.rejected("--arch", "aarch64", "--stage", "image",
                                                "--device", "rpi"))

    def test_device_is_rejected_by_the_sign_stage(self):
        # The device job runs sign separately, without a board, so that the shared
        # repository is identical to the one the ISO job publishes.
        self.assertIn("--device", self.rejected("--arch", "aarch64", "--stage", "sign",
                                                "--device", "rpi"))

    def test_device_stage_requires_a_device(self):
        self.assertIn("--device", self.rejected("--arch", "aarch64", "--stage", "device"))

    def test_every_stage_reaches_a_builder_method(self):
        # The dispatcher looks stages up in STAGE_METHODS, so a stage without a
        # target only fails once CI runs it.
        for stage in ("bootstrap", "packages", "sign", "image", "device", "verify"):
            method = getattr(builder.Builder, builder.STAGE_METHODS.get(stage, stage), None)
            self.assertTrue(callable(method), stage)

    def test_device_must_support_the_architecture(self):
        self.assertIn("does not support x86_64",
                      self.rejected("--arch", "x86_64", "--stage", "device", "--device", "rpi"))

    def test_unknown_device_is_rejected(self):
        self.rejected("--arch", "aarch64", "--stage", "device", "--device", "bananapi")

    @unittest.skipIf(os.geteuid() == 0, "a root host would start a real build")
    def test_device_is_a_known_stage(self):
        # Past argument validation the builder insists on a root build host.
        stderr = self.rejected("--arch", "aarch64", "--stage", "device", "--device", "rpi")
        self.assertIn("build host", stderr)

    @unittest.skipIf(os.geteuid() == 0, "a root host would start a real build")
    def test_device_is_accepted_by_the_all_stage(self):
        stderr = self.rejected("--arch", "aarch64", "--stage", "all", "--device", "rpi")
        self.assertIn("build host", stderr)


if __name__ == "__main__":
    unittest.main()
