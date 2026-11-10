#!/usr/bin/env python3
"""Exercise desktop panel creation and popup dismissal on monitor hotplug."""
import json
import os
import select
import subprocess
import time

import pyatspi
from gi.repository import GLib


desktop = os.environ['UFLINUX_DESKTOP_BIN']
driver = os.environ['UFLINUX_KEYBOARD_DRIVER']
keyboard = subprocess.Popen([driver], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                            text=True, bufsize=1)


def reply():
    assert select.select([keyboard.stdout], [], [], 10)[0], 'Virtual keyboard timed out'
    return keyboard.stdout.readline().strip()


def key(name):
    for request in ('mod ctrl 1', 'mod alt 1', 'key ' + name, 'mod alt 0', 'mod ctrl 0'):
        keyboard.stdin.write(request + '\n')
        keyboard.stdin.flush()
        assert reply() == 'ok', request


def wait(predicate, message):
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        while GLib.MainContext.default().pending():
            GLib.MainContext.default().iteration(False)
        try:
            if predicate():
                return
        except (RuntimeError, LookupError):
            pass
        time.sleep(.05)
    raise AssertionError(message)


def panels():
    return sum(1 for app in pyatspi.Registry.getDesktop(0) if app.name == 'uflinux-desktop'
               for child in app if child.name == 'UFLinux panel')


def status():
    return json.loads(subprocess.check_output([desktop, '--status'], text=True, timeout=5))


def open_popup(name):
    arguments = ['--start-menu'] if name == 'start' else ['--panel', name]
    subprocess.run([desktop, *arguments, '--output', 'UF-TEST'], check=True, timeout=5)
    wait(lambda: status()['popup'] == name and status()['output'] == 'UF-TEST',
         f'{name} did not open on the new output')


try:
    assert reply() == 'ready'
    wait(lambda: panels() == 1, 'Expected one initial panel')
    for popup in ('start', 'network'):
        key('F10')
        wait(lambda: panels() == 2, 'Panel was not created for the added output')
        open_popup(popup)
        key('F11')
        wait(lambda: panels() == 1, 'Removed output retained its panel')
        wait(lambda: status()['popup'] is None, f'{popup} survived output removal')
    print('Panel hotplug and Start/Network popup dismissal passed')
finally:
    keyboard.terminate()
    keyboard.wait(timeout=5)
