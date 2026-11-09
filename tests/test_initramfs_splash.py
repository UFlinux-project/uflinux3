"""Exercise the early-init wrapper without mounting or starting a daemon."""
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WRAPPER = ROOT / "aports/uflinux/uflinux-initramfs-init"


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
        self.assertEqual(patched.count('uflinux-initramfs-splash start'), 2)
        self.assertEqual(patched.count('uflinux-initramfs-splash handoff'), 2)
        self.assertLess(patched.index('uflinux-initramfs-splash prepare'),
                        patched.index('uflinux-initramfs-splash start'))

    def test_unknown_alpine_init_boots_without_injection(self):
        self.assertIsNone(self.run_wrapper('exit 0\n# upstream structure changed\n'))


if __name__ == '__main__':
    unittest.main()
