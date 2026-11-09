"""Ensure abuild's automatic debug override does not reach release builds."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class ReleaseDebugTests(unittest.TestCase):
    def test_abuild_override_is_cleared_before_cargo(self):
        for origin in ('ironbar', 'uflinux-settings'):
            with self.subTest(origin=origin), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                for name, script in {
                    'apk': '#!/bin/sh\nexit 0\n',
                    'cargo': '#!/bin/sh\nprintf "%s" "${CARGO_PROFILE_RELEASE_DEBUG-unset}" > "$CAPTURE"\n',
                }.items():
                    path = root / name
                    path.write_text(script)
                    path.chmod(0o755)
                env = dict(os.environ, PATH=str(root) + ':' + os.environ['PATH'],
                           CARGO_PROFILE_RELEASE_DEBUG='true', CARGO_TARGET_DIR=str(root / 'target'),
                           CAPTURE=str(root / 'captured'), srcdir=str(root), RUSTFLAGS='')
                recipe = ROOT / 'aports' / origin / 'APKBUILD'
                subprocess.run(['sh', '-ec', '. "$1"; build', 'test', str(recipe)], env=env, check=True)
                self.assertEqual((root / 'captured').read_text(), 'unset')


if __name__ == '__main__':
    unittest.main()
