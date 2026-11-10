#!/usr/bin/env python3
"""Verify shipped artwork renders within the reference desktop's memory limits."""
from pathlib import Path
import os
import subprocess
import sys
import tempfile
import gi

gi.require_version('Gtk', '4.0')
gi.require_version('Gdk', '4.0')
gi.require_version('GdkPixbuf', '2.0')
from gi.repository import Gdk, GdkPixbuf, Gtk

# GTK versions cannot share one process. Probe GTK3 in a short-lived child.
subprocess.run([sys.executable, '-c', """
import gi
gi.require_version('Gtk', '3.0')
from gi.repository import Gtk
Gtk.init([])
s = Gtk.Settings.get_default()
actual = s.get_property('gtk-theme-name')
assert actual == 'adw-gtk3-dark', f'GTK3 theme: expected adw-gtk3-dark, got {actual!r}'
assert s.get_property('gtk-icon-theme-name') == 'Papirus'
assert s.get_property('gtk-application-prefer-dark-theme')
"""], check=True)
Gtk.init()
settings = Gtk.Settings.get_default()
assert settings.get_property('gtk-icon-theme-name') == 'Papirus'
actual = settings.get_property('gtk-theme-name')
assert actual == 'adw-gtk3-dark', f'GTK4 theme: expected adw-gtk3-dark, got {actual!r}'
assert settings.get_property('gtk-application-prefer-dark-theme')
# Schema and INI defaults must leave explicit personal preferences in control.
# Isolate both stores so the probes cannot change the session's settings.
with tempfile.TemporaryDirectory() as config:
    directory = Path(config) / 'gtk-4.0'
    directory.mkdir()
    (directory / 'settings.ini').write_text(
        '[Settings]\ngtk-theme-name=Adwaita\ngtk-icon-theme-name=hicolor\n')
    for version in ('3.0', '4.0'):
        subprocess.run([sys.executable, '-c', """
import sys
import gi
gi.require_version('Gtk', sys.argv[1])
from gi.repository import Gio, Gtk
preferences = Gio.Settings.new('org.gnome.desktop.interface')
assert preferences.get_string('gtk-theme') == 'adw-gtk3-dark'
assert preferences.get_string('icon-theme') == 'Papirus'
assert preferences.get_string('color-scheme') == 'prefer-dark'
if sys.argv[1] == '3.0':
    assert preferences.set_string('gtk-theme', 'Adwaita')
    assert preferences.set_string('icon-theme', 'hicolor')
Gtk.init([]) if sys.argv[1] == '3.0' else Gtk.init()
settings = Gtk.Settings.get_default()
assert settings.get_property('gtk-theme-name') == 'Adwaita', 'Personal GTK theme was overridden'
assert settings.get_property('gtk-icon-theme-name') == 'hicolor', 'Personal icon theme was overridden'
""", version], check=True, env=dict(os.environ, GSETTINGS_BACKEND='memory', XDG_CONFIG_HOME=config))
for version in ('gtk-3.0', 'gtk-4.0'):
    assert (Path('/usr/share/themes/adw-gtk3-dark') / version / 'gtk.css').is_file()
theme = Gtk.IconTheme.get_for_display(Gdk.Display.get_default())
for name in ('uflinux', 'org.uflinux.Settings', 'system-file-manager', 'preferences-system'):
    assert theme.has_icon(name), 'Missing icon: ' + name
    icon = theme.lookup_icon(name, None, 32, 1, Gtk.TextDirection.NONE, Gtk.IconLookupFlags.FORCE_REGULAR)
    assert icon.get_file() and icon.get_file().query_exists(None), 'Unresolvable icon: ' + name
root = Path('/usr/share/backgrounds/uflinux')
images = sorted(root.glob('*.jpg'))
assert len(images) == 6 and (root / 'Dark_Ivy.jpg').is_file()
for path in images:
    info, width, height = GdkPixbuf.Pixbuf.get_file_info(str(path))
    assert info and 0 < width * height <= 16_777_216 and path.stat().st_size <= 32 * 1024 * 1024, path
    preview = GdkPixbuf.Pixbuf.new_from_file_at_scale(str(path), 192, 108, True)
    assert preview.get_width() <= 192 and preview.get_height() <= 108
for path in (Path('/usr/share/icons/hicolor/scalable/apps/uflinux.svg'),
             Path('/usr/share/icons/hicolor/scalable/apps/org.uflinux.Settings.svg')):
    icon = GdkPixbuf.Pixbuf.new_from_file_at_scale(str(path), 32, 32, True)
    assert icon.get_width() == 32 and icon.get_height() == 32
print('adw-gtk3 GTK3/GTK4 defaults and personal overrides, original artwork, Papirus lookup and bounded wallpaper previews passed')
