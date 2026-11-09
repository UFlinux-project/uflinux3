import importlib.machinery
import importlib.util
import json
from pathlib import Path
import stat
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

PACKAGE = Path(__file__).resolve().parents[1] / "aports/uflinux/uflinux-install"
loader = importlib.machinery.SourceFileLoader("uflinux_installer", str(PACKAGE))
spec = importlib.util.spec_from_loader(loader.name, loader)
installer = importlib.util.module_from_spec(spec)
loader.exec_module(installer)


class MountTests(unittest.TestCase):
    def test_wrong_filesystem_rejected(self):
        result = {"filesystems": [{"target": "/mnt", "source": "/dev/vda3", "fstype": "ext4", "options": "rw", "uuid": "123"}]}
        with patch.object(installer, "output", return_value=json.dumps(result)):
            with self.assertRaisesRegex(installer.InstallError, "xfs"):
                installer.mounted(Path("/mnt"), "xfs")

    def test_readonly_root_rejected(self):
        result = {"filesystems": [{"target": "/mnt", "source": "/dev/vda3", "fstype": "xfs", "options": "ro", "uuid": "123"}]}
        with patch.object(installer, "output", return_value=json.dumps(result)):
            with self.assertRaisesRegex(installer.InstallError, "writable"):
                installer.mounted(Path("/mnt"), "xfs")

    def test_parent_mount_is_not_accepted(self):
        result = {"filesystems": [{"target": "/", "source": "/dev/vda3", "fstype": "xfs", "options": "rw", "uuid": "123"}]}
        with patch.object(installer, "output", return_value=json.dumps(result)):
            with self.assertRaises(installer.InstallError):
                installer.mounted(Path("/mnt"), "xfs")


class InstallerTests(unittest.TestCase):
    def args(self, target, **overrides):
        return SimpleNamespace(**dict(dict(target=str(target), user="alice", hostname="uflinux", arch="x86_64",
                                          boot_mode="native", boot_disk=None, dry_run=True), **overrides))

    def test_live_root_rejected_before_commands(self):
        with patch.object(installer.os, "geteuid", return_value=0), patch.object(installer, "run") as run:
            with self.assertRaises(installer.InstallError):
                installer.preflight(self.args("/"))
            run.assert_not_called()

    def test_reserved_user_rejected(self):
        with patch.object(installer.os, "geteuid", return_value=0):
            with self.assertRaises(installer.InstallError):
                installer.preflight(self.args("/mnt", user="live"))

    def test_non_root_rejected(self):
        with patch.object(installer.os, "geteuid", return_value=1000):
            with self.assertRaises(installer.InstallError):
                installer.preflight(self.args("/mnt"))

    def test_nonempty_root_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            (Path(tmp) / "important.txt").write_text("must survive")
            with patch.object(installer.os, "geteuid", return_value=0), patch.object(installer, "mounted", side_effect=[{"source": "/dev/vda3"}, {"source": "/dev/vda2"}]):
                with self.assertRaisesRegex(installer.InstallError, "empty"):
                    installer.preflight(self.args(tmp))
            self.assertEqual((Path(tmp) / "important.txt").read_text(), "must survive")

    def test_bios_requires_explicit_disk(self):
        with tempfile.TemporaryDirectory() as tmp, patch.object(installer.os, "geteuid", return_value=0), \
             patch.object(installer, "mounted", side_effect=[{"source": "/dev/vda3"}, {"source": "/dev/vda2"}]), \
             patch.object(installer, "output", return_value="x86_64"):
            with self.assertRaisesRegex(installer.InstallError, "--boot-disk"):
                installer.preflight(self.args(tmp))

    def test_arm_native_rejected(self):
        with tempfile.TemporaryDirectory() as tmp, patch.object(installer.os, "geteuid", return_value=0), \
             patch.object(installer, "mounted", side_effect=[{"source": "/dev/vda3"}, {"source": "/dev/vda2"}]), \
             patch.object(installer, "output", return_value="aarch64"):
            with self.assertRaisesRegex(installer.InstallError, "UEFI"):
                installer.preflight(self.args(tmp))

    def test_dry_run_never_installs_or_prompts(self):
        with patch.object(installer, "preflight", return_value={"target": "/mnt"}), \
             patch.object(installer, "install") as install, patch("builtins.input") as prompt:
            self.assertEqual(installer.main(["--target", "/mnt", "--user", "alice", "--dry-run"]), 0)
            install.assert_not_called()
            prompt.assert_not_called()

    def test_clean_overlay_excludes_live_credentials(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            original_read = Path.read_text
            def read(path, *args, **kwargs):
                if str(path) == "/usr/share/uflinux/repositories":
                    return "https://example.org/apk\n"
                return original_read(path, *args, **kwargs)
            with patch.object(Path, "read_text", read), patch.object(Path, "glob", return_value=[]):
                installer.make_overlay(root, self.args("/mnt"))
            self.assertEqual((root / "etc/shadow").read_text(), "root:!:19000:0:99999:7:::\n")
            world = (root / "etc/apk/world").read_text().splitlines()
            self.assertNotIn("live", world)
            self.assertIn("linux-firmware", world)
            self.assertNotIn("linux-firmware-none", world)
            self.assertFalse((root / "etc/runlevels/default/uflinux-live").exists())
            self.assertEqual((root / "etc/runlevels/default/greetd").readlink(), Path("/etc/init.d/greetd"))

    def test_bios_syslinux_requires_only_boot_partition_marked(self):
        table = {"label": "gpt", "partitions": [
            {"node": "/dev/vda1", "attrs": "RequiredPartition, LegacyBIOSBootable"},
            {"node": "/dev/vda2"}]}
        self.assertEqual(installer.bios_mbr_code(table, "/dev/vda1"), "/usr/share/syslinux/gptmbr.bin")
        with self.assertRaisesRegex(installer.InstallError, "LegacyBIOSBootable"):
            installer.bios_mbr_code(table, "/dev/vda2")
        table["partitions"][1]["attrs"] = "LegacyBIOSBootable"
        with self.assertRaises(installer.InstallError):
            installer.bios_mbr_code(table, "/dev/vda1")

    def test_bios_mbr_write_preserves_partition_table(self):
        with tempfile.TemporaryDirectory() as tmp:
            disk, boot_code = Path(tmp) / "disk", Path(tmp) / "mbr"
            disk.write_bytes(b"d" * 4096)
            boot_code.write_bytes(b"b" * 440)
            installer.install_bios_mbr(disk, boot_code)
            self.assertEqual(disk.read_bytes(), b"b" * 440 + b"d" * (4096 - 440))
            boot_code.write_bytes(b"too short")
            with self.assertRaisesRegex(installer.InstallError, "size"):
                installer.install_bios_mbr(disk, boot_code)

    def test_bios_dos_uses_active_partition(self):
        table = {"label": "dos", "partitions": [{"node": "/dev/vda1", "bootable": True}]}
        self.assertEqual(installer.bios_mbr_code(table, "/dev/vda1"), "/usr/share/syslinux/mbr.bin")
        table["partitions"][0]["node"] = "/dev/vda5"
        with self.assertRaisesRegex(installer.InstallError, "primary partition"):
            installer.bios_mbr_code(table, "/dev/vda5")
        table["partitions"][0]["node"] = "/dev/vda1"
        table["partitions"][0]["bootable"] = False
        with self.assertRaisesRegex(installer.InstallError, "bootable MBR flag"):
            installer.bios_mbr_code(table, "/dev/vda1")

    def test_uefi_boot_entry_uses_esp_and_root_uuid(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            binary = root / "usr/lib/systemd/boot/efi/systemd-bootx64.efi"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"efi test binary")
            plan = {"arch": "x86_64", "root": {"uuid": "1234-abcd"}}
            installer.install_systemd_boot(root, plan)
            self.assertEqual((root / "boot/EFI/BOOT/BOOTX64.EFI").read_bytes(), b"efi test binary")
            entry = (root / "boot/loader/entries/uflinux.conf").read_text()
            self.assertIn("root=UUID=1234-abcd", entry)
            self.assertIn("linux /vmlinuz-lts", entry)
            self.assertIn("initrd /initramfs-lts", entry)

    def test_uefi_overlay_installs_systemd_boot(self):
        with tempfile.TemporaryDirectory() as tmp, patch.object(Path, "glob", return_value=[]):
            root = Path(tmp)
            installer.make_overlay(root, self.args("/mnt"), "https://example.org/apk\n", uefi=True)
            self.assertIn("systemd-boot\n", (root / "etc/apk/world").read_text())

    def test_repositories_restored_after_failed_install(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "repositories"
            path.write_text("https://original.example/apk\n")
            with self.assertRaises(RuntimeError):
                with installer.temporary_repositories(path, "/media/cdrom/apks\n"):
                    self.assertEqual(path.read_text(), "/media/cdrom/apks\n")
                    raise RuntimeError("simulated install failure")
            self.assertEqual(path.read_text(), "https://original.example/apk\n")


if __name__ == "__main__":
    unittest.main()
