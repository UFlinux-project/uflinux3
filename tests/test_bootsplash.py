"""Exercise the production lifecycle with fake devices/commands, never host DRM."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class SplashTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.state = self.root / 'state'
        (self.root / 'dev/dri').mkdir(parents=True)
        self.cmdline = self.root / 'cmdline'
        self.cmdline.write_text('quiet console=ttyS0')
        self.bin = self.root / 'bin'
        self.bin.mkdir()
        self.log = self.root / 'commands'
        self.log.touch()
        for name in ('plymouthd', 'plymouth', 'start-stop-daemon', 'logger'):
            script = self.bin / name
            script.write_text('''#!/bin/sh
printf '%s %s\\n' "${0##*/}" "$*" >> "$SPLASH_TEST_LOG"
case "${0##*/}:$1" in
 plymouthd:*)
  [ "${FAIL_STAGE:-}" != daemon ] || exit 1
  for arg do case "$arg" in --pid-file=*) printf '99999999\\n' > "${arg#*=}";; esac; done ;;
 plymouth:show-splash) [ "${FAIL_STAGE:-}" != show ] || exit 1 ;;
esac
exit 0
''')
            script.chmod(0o755)
        original = (ROOT / 'aports/uflinux/uflinux-bootsplash').read_text()
        self.helper = self.root / 'helper'
        self.helper.write_text(original.replace('state=/run/uflinux-splash', 'state=' + str(self.state))
                               .replace('/proc/cmdline', str(self.cmdline))
                               .replace('/dev/tty1', str(self.root / 'dev/tty1'))
                               .replace('/dev/dri/', str(self.root / 'dev/dri') + '/')
                               .replace('/dev/fb', str(self.root / 'dev/fb')))
        self.env = dict(os.environ, PATH=str(self.bin) + ':' + os.environ['PATH'], SPLASH_TEST_LOG=str(self.log), FAIL_STAGE='')

    def devices(self):
        # Harmless character-device stand-ins. The renderer itself is mocked.
        (self.root / 'dev/tty1').symlink_to('/dev/null')
        (self.root / 'dev/dri/card0').symlink_to('/dev/null')

    def run_helper(self, action):
        subprocess.run(['sh', str(self.helper), action], env=self.env, check=True)

    def test_explicit_opt_out_never_starts_renderer(self):
        self.devices()
        for option in ('splash=no', 'nosplash', 'plymouth.enable=0'):
            self.cmdline.write_text('quiet ' + option + ' console=ttyS0')
            self.run_helper('start')
        self.assertEqual(self.log.read_text(), '')
        self.assertFalse(self.state.exists())

    def test_serial_only_boot_never_starts_renderer(self):
        self.run_helper('start')
        self.assertEqual(self.log.read_text(), '')
        self.assertFalse(self.state.exists())

    def test_renderer_failures_are_nonfatal_and_cleaned_up(self):
        self.devices()
        for stage in ('daemon', 'show'):
            self.env['FAIL_STAGE'] = stage
            self.run_helper('start')
            self.assertTrue((self.state / 'completed').exists())
            self.assertFalse((self.state / 'started').exists())
            self.assertFalse((self.state / 'pid').exists())

    def test_start_stop_are_idempotent_and_state_is_private(self):
        self.devices()
        self.run_helper('start')
        self.run_helper('start')
        self.assertTrue((self.state / 'started').exists())
        self.assertEqual(self.state.stat().st_mode & 0o777, 0o700)
        self.assertEqual(sum(line.startswith('plymouthd ') for line in self.log.read_text().splitlines()), 1)
        self.run_helper('stop')
        self.run_helper('stop')
        self.assertTrue((self.state / 'completed').exists())
        self.assertFalse((self.state / 'started').exists())
        self.assertFalse((self.state / 'pid').exists())
