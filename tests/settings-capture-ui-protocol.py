#!/usr/bin/env python3
"""Drive the Screen Capture page over AT-SPI and confirm the daemon records it."""
import json
import subprocess
import time
import pyatspi
from gi.repository import GLib

SWITCH_ROLES = ('switch', 'toggle button', 'check box')
CHECKED = (pyatspi.STATE_CHECKED, pyatspi.STATE_PRESSED)


def state():
    return json.loads(subprocess.check_output(['uflinux-settingsctl', 'status'],
                                              text=True, timeout=15))


def tree():
    root = pyatspi.Registry.getDesktop(0)
    queue = [a for a in root if a.name == 'uflinux-settings']
    result = []
    while queue:
        a = queue.pop()
        result.append(a)
        try:
            queue.extend([c for c in a if c])
        except Exception:
            pass
        assert len(result) < 2000, 'Unbounded accessible tree'
    return result


def find(name, roles=()):
    return next((a for a in tree()
                 if a.name == name and (not roles or a.getRoleName() in roles)), None)


def click(a):
    action = a.queryAction()
    assert action.nActions, 'Control has no keyboard/accessibility action: ' + a.name
    assert action.doAction(0)


def wait(predicate, description):
    for _ in range(80):
        while GLib.MainContext.default().pending():
            GLib.MainContext.default().iteration(False)
        try:
            value = predicate()
            if value:
                return value
        except (RuntimeError, LookupError, AttributeError, StopIteration):
            pass
        time.sleep(.05)
    raise AssertionError(description)


def active(a):
    states = a.getState()
    return any(states.contains(state) for state in CHECKED)


# The page is already showing, and the whole section is written back on each change.
before = state()['screen_capture']
# Rows are looked up by their visible label. GTK4 exposes a DropDown's accessible
# name differently across versions, but the row label itself is always there, and
# the control is appended with it.
wait(lambda: find('Image format', ('label',)), 'Screenshot controls are missing')
wait(lambda: find('Video quality', ('label',)), 'Recording controls are missing')
wait(lambda: find('GPU screen recorder', ('label',)), 'Backend status is missing')
wait(lambda: find('Copy to clipboard', SWITCH_ROLES), 'Clipboard switch is missing')
click(find('Copy to clipboard', SWITCH_ROLES))
wait(lambda: state()['screen_capture']['copy_to_clipboard'] != before['copy_to_clipboard'],
     'Toggling the clipboard switch did not reach the daemon')
# The control follows the published state, so it has to read back as the new value.
wait(lambda: active(find('Copy to clipboard', SWITCH_ROLES))
     == state()['screen_capture']['copy_to_clipboard'], 'Switch and published state differ')
click(find('Copy to clipboard', SWITCH_ROLES))
wait(lambda: state()['screen_capture'] == before, 'Restoring the switch did not converge')
print('Screen Capture page controls, switch round-trip and state echo passed')
