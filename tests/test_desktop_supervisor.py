"""Keep the desktop supervisor inside the desktop binary.

The restart loop used to be a shell script, which meant the session paid for a
second interpreter to keep one window owner resident. Its behaviour is covered
by `cargo test -p uflinux-desktop` (run by the package `check()`); what is
asserted here is that the session still reaches that code and that nothing
reintroduces the script.
"""
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1]
AUTOSTART = ROOT / 'aports/uflinux/uflinux-autostart'
RECIPE = ROOT / 'aports/uflinux-settings/APKBUILD'
MAIN = ROOT / 'settings/desktop/src/main.rs'
SUPERVISOR = ROOT / 'settings/desktop/src/supervisor.rs'
LEGACY = ROOT / 'settings/desktop/data/uflinux-desktop-supervisor'


class DesktopSupervisorTests(unittest.TestCase):
    def test_the_script_is_not_brought_back(self):
        self.assertFalse(LEGACY.exists(), 'the shell supervisor was replaced by --supervise')
        self.assertNotIn('uflinux-desktop-supervisor', RECIPE.read_text(),
                         'the recipe packages a supervisor that no longer exists')

    def test_the_session_uses_the_builtin_supervisor(self):
        autostart = AUTOSTART.read_text()
        self.assertIn('uflinux-desktop --supervise &', autostart,
                      'the session starts the desktop without supervision')
        # Supervision must not hold the session open past logout.
        self.assertIn('wait', autostart)

    def test_supervision_runs_before_any_display_connection(self):
        main = MAIN.read_text()
        self.assertIn('mod supervisor;', main)
        body = main.split('fn main() {', 1)[1]
        self.assertLess(body.index('--supervise'), body.index('Taskbar::default()'),
                        'supervision initializes GTK or the taskbar first')
        self.assertIn('supervisor::run()', body)

    def test_supervision_is_reachable_from_the_packaged_binary(self):
        recipe = RECIPE.read_text()
        self.assertIn('install -Dm755 "$target/uflinux-desktop" "$pkgdir/usr/bin/uflinux-desktop"',
                      recipe)
        self.assertIn('amove usr/bin/uflinux-desktop', recipe,
                      'the desktop binary is not in the package the session runs')
        self.assertTrue(SUPERVISOR.is_file())


if __name__ == '__main__':
    unittest.main()
