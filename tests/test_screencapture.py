"""Static wiring checks for the Print Screen capture tool and its portal defaults."""
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1]
UFLINUX = ROOT / 'aports/uflinux'
SETTINGS = ROOT / 'aports/uflinux-settings'


def read(path):
    return path.read_text(encoding='utf-8')


class KeybindTests(unittest.TestCase):
    def setUp(self):
        self.rc = read(UFLINUX / 'labwc-rc.xml')

    def test_print_opens_the_capture_tool(self):
        self.assertIn('<keybind key="Print"><action name="Execute" command="uflinux-screencapture"/></keybind>', self.rc)

    def test_modified_print_starts_a_recording(self):
        for key in ('S-Print', 'C-Print'):
            with self.subTest(key=key):
                self.assertIn('<keybind key="%s"><action name="Execute" command="uflinux-screencapture --record"/></keybind>' % key, self.rc)


class PortalTests(unittest.TestCase):
    """Window capture reaches the wlr portal through two packaged drop-ins."""

    def setUp(self):
        self.apkbuild = read(UFLINUX / 'APKBUILD')

    def test_both_drop_ins_are_shipped(self):
        for name in ('labwc-portals.conf', 'xdg-desktop-portal-wlr-labwc'):
            with self.subTest(name=name):
                self.assertIn(name, self.apkbuild.split('build()')[0])

    def test_drop_ins_land_in_the_xdg_directories(self):
        self.assertIn('install -Dm644 "$srcdir/labwc-portals.conf" "$pkgdir/etc/xdg/xdg-desktop-portal/labwc-portals.conf"', self.apkbuild)
        # Installed under the desktop-specific name (not "config"): Alpine's
        # xdg-desktop-portal-wlr package owns /etc/xdg/xdg-desktop-portal-wlr/config.
        self.assertIn('install -Dm644 "$srcdir/xdg-desktop-portal-wlr-labwc" "$pkgdir/etc/xdg/xdg-desktop-portal-wlr/labwc"', self.apkbuild)

    def test_screen_cast_is_pinned_to_the_wlr_backend(self):
        # XDG_CURRENT_DESKTOP is labwc, and the file name must match it in lower case.
        conf = read(UFLINUX / 'labwc-portals.conf')
        self.assertIn('[preferred]', conf)
        self.assertIn('org.freedesktop.impl.portal.ScreenCast=wlr', conf)

    def test_portal_frame_rate_is_capped(self):
        self.assertIn('max_fps=60', read(UFLINUX / 'xdg-desktop-portal-wlr-labwc'))

    def test_desktop_meta_pulls_in_the_capture_stack(self):
        desktop = self.apkbuild.split('desktop() {')[1]
        for package in ('uflinux-screencapture', 'gpu-screen-recorder', 'xdg-desktop-portal', 'xdg-desktop-portal-wlr'):
            with self.subTest(package=package):
                self.assertIn(package, desktop)


class PackagingTests(unittest.TestCase):
    def test_the_tool_is_a_settings_subpackage(self):
        apkbuild = read(SETTINGS / 'APKBUILD')
        self.assertIn('uflinux-screencapture:screencapture', apkbuild.split('options=')[0])
        self.assertIn('install -Dm755 "$target/uflinux-screencapture" "$pkgdir/usr/bin/uflinux-screencapture"', apkbuild)
        self.assertIn('amove usr/bin/uflinux-screencapture usr/share/applications/org.uflinux.ScreenCapture.desktop', apkbuild)

    def test_the_desktop_entry_launches_the_tool(self):
        entry = read(ROOT / 'settings/data/org.uflinux.ScreenCapture.desktop')
        self.assertIn('Exec=uflinux-screencapture', entry)
        # The tool is a key-triggered utility, not a launcher entry.
        self.assertIn('NoDisplay=true', entry)


class PageTests(unittest.TestCase):
    """The Screen Capture page has to be reachable and covered by the tests."""

    def test_page_is_registered(self):
        self.assertIn('("screen-capture", "Screen Capture")', read(ROOT / 'settings/ui/src/app.rs'))
        self.assertIn('"screen-capture" => screen_capture::page(app, parent)',
                      read(ROOT / 'settings/ui/src/pages/mod.rs'))

    def test_smoke_and_vm_tests_cover_the_page(self):
        self.assertIn('screen-capture', read(ROOT / 'tests/settings-smoke.sh'))
        self.assertIn("'screen-capture'", read(ROOT / 'tests/settings-installed.py'))

    def test_documentation_lists_the_page(self):
        self.assertIn('screen-capture', read(ROOT / 'docs/settings.md'))
        self.assertIn('screen-capture', read(ROOT / 'settings/README.md'))


if __name__ == '__main__':
    unittest.main()
