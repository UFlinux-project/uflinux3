"""Exercise the early-init wrapper without mounting or starting a daemon."""
import re
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WRAPPER = ROOT / "aports/uflinux/uflinux-initramfs-init"
DEVICE_PROFILE = ROOT / "profiles/device-rootfs.sh"


class InitramfsWrapperTest(unittest.TestCase):
    def run_wrapper(self, source):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            original = directory / "alpine-init"
            patched = directory / "patched-init"
            busybox = directory / "busybox"
            original.write_text(source)
            busybox.write_text('#!/bin/sh\nexec "$@"\n')
            busybox.chmod(0o755)
            wrapper = WRAPPER.read_text().replace('/bin/busybox', str(busybox))
            wrapper = wrapper.replace('/usr/share/mkinitfs/initramfs-init', str(original))
            wrapper = wrapper.replace('/tmp/uflinux-init', str(patched))
            subprocess.run(['sh', '-c', wrapper], check=True)
            return patched.read_text() if patched.exists() else None

    def test_injects_both_boot_paths_and_handoffs(self):
        script = '''exit 0
# shared memory area (later system will need it)
# read the kernel options.
# workaround for vmware
if [ "$SINGLEMODE" = "yes" ]; then
 :
fi
cat "$ROOT"/proc/mounts 2>/dev/null | while read -r _dev DIR _type _opts ; do :; done
if [ "$SINGLEMODE" = "yes" ]; then
 :
fi
cat "$ROOT"/proc/mounts 2>/dev/null | while read -r _dev DIR _type _opts ; do :; done
'''
        patched = self.run_wrapper(script)
        self.assertEqual(patched.count('uflinux-initramfs-splash prepare'), 1)
        self.assertEqual(patched.count('uflinux-initramfs-splash start'), 3)
        self.assertEqual(patched.count('uflinux-initramfs-splash handoff'), 2)
        self.assertLess(patched.index('uflinux-initramfs-splash prepare'),
                        patched.index('uflinux-initramfs-splash start'))

    def test_unknown_alpine_init_boots_without_injection(self):
        self.assertIsNone(self.run_wrapper('exit 0\n# upstream structure changed\n'))

    def test_device_profile_ships_what_the_wrapper_execs(self):
        # mkinitfs -i replaces /init but never copies Alpine's initramfs-init,
        # and device images do not use the feature that normally provides it,
        # so the profile has to list the wrapper's own dependencies.
        profile = DEVICE_PROFILE.read_text()
        listed = re.search(r'^initramfs_boot_files="([^"]*)"', profile, re.M)
        self.assertIsNotNone(listed, "profile must name the wrapper's files")
        files = listed.group(1).split()
        self.assertIn("usr/share/mkinitfs/initramfs-init", files)
        for hook in re.findall(r"/usr/libexec/[a-z-]+", WRAPPER.read_text()):
            self.assertIn(hook.lstrip("/"), files, hook)
        # Listing them is only half of it: mkinitfs drops missing entries
        # silently, so the profile has to fail the build instead.
        self.assertIn("$initramfs_boot_files", profile.split("for file in")[1])


if __name__ == '__main__':
    unittest.main()
