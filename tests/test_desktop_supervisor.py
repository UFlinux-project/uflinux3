import os
from pathlib import Path
import subprocess
import tempfile
import time
import unittest


SUPERVISOR = Path(__file__).resolve().parents[1] / 'settings/desktop/data/uflinux-desktop-supervisor'


class DesktopSupervisorTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.compositor = subprocess.Popen(['sleep', '60'])
        self.addCleanup(self.stop, self.compositor)
        self.env = dict(os.environ, LABWC_PID=str(self.compositor.pid),
                        PATH=str(self.root) + os.pathsep + os.environ['PATH'],
                        FIXTURE=str(self.root / 'starts'))

    @staticmethod
    def stop(process):
        if process.poll() is None:
            process.terminate()
            process.wait(timeout=5)

    def desktop(self, body):
        path = self.root / 'uflinux-desktop'
        path.write_text('#!/bin/sh\n' + body + '\n')
        path.chmod(0o755)

    def supervisor(self):
        process = subprocess.Popen(['sh', str(SUPERVISOR)], env=self.env)
        self.addCleanup(self.stop, process)
        return process

    def wait_for_starts(self, count):
        deadline = time.monotonic() + 8
        while time.monotonic() < deadline:
            if (self.root / 'starts').exists() and len((self.root / 'starts').read_text().splitlines()) >= count:
                return
            time.sleep(.02)
        self.fail(f'desktop did not start {count} times')

    def test_restarts_failures_then_stops_after_clean_exit(self):
        self.desktop('echo start >> "$FIXTURE"\n[ "$(wc -l < "$FIXTURE")" -ge 3 ]')
        start = time.monotonic()
        process = self.supervisor()
        self.assertEqual(process.wait(timeout=8), 0)
        self.assertGreaterEqual(time.monotonic() - start, 2.8)
        self.assertEqual(len((self.root / 'starts').read_text().splitlines()), 3)

    def test_stops_resident_when_compositor_exits(self):
        self.desktop('echo start >> "$FIXTURE"\nexec sleep 60')
        process = self.supervisor()
        self.wait_for_starts(1)
        self.compositor.terminate()
        self.compositor.wait(timeout=5)
        self.assertEqual(process.wait(timeout=4), 0)
        self.assertEqual(len((self.root / 'starts').read_text().splitlines()), 1)

    def test_requires_compositor_pid(self):
        environment = dict(self.env)
        environment.pop('LABWC_PID')
        process = subprocess.run(['sh', str(SUPERVISOR)], env=environment, capture_output=True)
        self.assertNotEqual(process.returncode, 0)


if __name__ == '__main__':
    unittest.main()
