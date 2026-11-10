#!/usr/bin/env python3
"""Check live desktop-entry changes in the resident Start catalog."""
import os
from pathlib import Path
import subprocess
import time

import pyatspi
from gi.repository import GLib


def find(name):
    queue = [app for app in pyatspi.Registry.getDesktop(0) if app.name == 'uflinux-desktop']
    while queue:
        node = queue.pop(0)
        if node.name == name and node.getState().contains(pyatspi.STATE_SHOWING):
            return node
        try:
            queue.extend(list(node))
        except Exception:
            pass
    return None


def wait(predicate, message):
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        while GLib.MainContext.default().pending():
            GLib.MainContext.default().iteration(False)
        try:
            value = predicate()
            if value:
                return value
        except (RuntimeError, LookupError):
            pass
        time.sleep(.05)
    raise AssertionError(message)


name = 'UFLinux Live Catalog Fixture'
entry = Path(os.environ['XDG_DATA_HOME']) / 'applications/uflinux-catalog-fixture.desktop'
try:
    subprocess.run([os.environ['UFLINUX_DESKTOP_BIN'], '--start-menu', '--output', 'HEADLESS-1'],
                   check=True, timeout=5)
    search = wait(lambda: find('Search applications'), 'Start search was not available')
    assert search.queryEditableText().setTextContents(name)
    entry.write_text('[Desktop Entry]\nType=Application\nName=' + name +
                     '\nExec=/bin/true\nIcon=utilities-terminal\nStartupWMClass=UF-Catalog-Fixture\n')
    wait(lambda: find(name), 'New desktop entry did not appear in Start search')
    entry.unlink()
    wait(lambda: not find(name), 'Removed desktop entry remained in Start search')
    print('Live desktop-entry addition, search and removal passed')
finally:
    entry.unlink(missing_ok=True)
    subprocess.run([os.environ['UFLINUX_DESKTOP_BIN'], '--hide'], check=True, timeout=5)
