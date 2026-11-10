#!/usr/bin/env python3
"""Bound input-to-accessible, painted popup latency in a prepared desktop session.

The result includes CLI startup or an AT-SPI action, plus observation overhead.
It is a diagnostic upper bound, not a substitute for measuring physical panel
input or for the release latency gate. The resident must be started with
UFLINUX_SETTINGS_BENCH=1 and its stderr written to --log.
"""
import argparse
import json
import math
import os
from pathlib import Path
import re
import subprocess
import time

import pyatspi
from gi.repository import GLib


MENUS = ('start', 'network', 'sound', 'power')
TITLES = {'start': 'Start menu', 'network': 'Network', 'sound': 'Sound', 'power': 'Power'}
BUTTONS = {'start': 'Menu', 'network': 'Network unavailable', 'sound': 'Audio unavailable', 'power': 'External power'}


def panel_button(menu):
    for app in pyatspi.Registry.getDesktop(0):
        if app.name != 'uflinux-desktop':
            continue
        for window in app:
            if window.name != 'UFLinux panel':
                continue
            for panel in window:
                offset = 0
                for node in panel:
                    bounds = node.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
                    if node.name == BUTTONS[menu] and node.getRoleName() in ('button', 'push button'):
                        return node, offset
                    offset += bounds.width
    raise AssertionError(f'Panel button not found for {menu}')


def usable(menu):
    for app in pyatspi.Registry.getDesktop(0):
        if app.name != 'uflinux-desktop':
            continue
        for window in app:
            if window.name != TITLES[menu] or not window.getState().contains(pyatspi.STATE_SHOWING):
                continue
            queue = [window]
            while queue:
                node = queue.pop(0)
                state = node.getState()
                if state.contains(pyatspi.STATE_SHOWING) and state.contains(pyatspi.STATE_SENSITIVE):
                    if menu == 'start' and node.name == 'Search applications':
                        return True
                    if menu != 'start' and node.getRoleName() in ('button', 'push button', 'slider', 'check box', 'switch'):
                        return True
                try:
                    queue.extend(list(node))
                except Exception:
                    pass
    return False


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--log', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--rounds', type=int, default=100)
    parser.add_argument('--input', choices=('cli', 'panel'), default='cli')
    args = parser.parse_args()
    if args.rounds < 1:
        parser.error('rounds must be positive')
    binary = os.environ['UFLINUX_DESKTOP_BIN']

    def command(*arguments):
        return subprocess.run([binary, *arguments], capture_output=True, text=True, check=True, timeout=5).stdout

    def measure(menu):
        offset = args.log.stat().st_size
        arguments = ['--start-menu'] if menu == 'start' else ['--panel', menu]
        button, _ = panel_button(menu) if args.input == 'panel' else (None, None)
        began_us = time.monotonic_ns() // 1000
        if button is not None:
            assert button.queryAction().doAction(0), f'Panel action failed for {menu}'
        else:
            command(*arguments, '--output', 'HEADLESS-1')
        pattern = (r'UFLINUX_START_PAINT_US=(\d+)' if menu == 'start' else
                   r'UFLINUX_QUICK_PAINT_US=(\d+) panel=' + menu + r'\b')
        deadline = time.monotonic() + 10
        painted = None
        while time.monotonic() < deadline:
            while GLib.MainContext.default().pending():
                GLib.MainContext.default().iteration(False)
            with args.log.open() as stream:
                stream.seek(offset)
                candidates = (int(value) for value in re.findall(pattern, stream.read()))
                painted = next((value for value in candidates if value >= began_us), None)
            if painted is not None:
                if usable(menu):
                    break
            time.sleep(.002)
        else:
            status = command('--status')
            with args.log.open() as stream:
                stream.seek(offset)
                tail = stream.read()[-2000:]
            raise AssertionError(f'{menu} did not become accessible after painting; status={status.strip()}, accessible={usable(menu)}, began={began_us}, painted={painted}, input={args.input}, log={tail!r}')
        observed_us = time.monotonic_ns() // 1000
        command('--hide')
        # Let the compositor finish unmapping before the next independent input.
        time.sleep(.1)
        return {'paint_ms': (painted - began_us) / 1000,
                'accessible_upper_bound_ms': (observed_us - began_us) / 1000}

    results = {}
    try:
        for menu in MENUS:
            samples = [measure(menu) for _ in range(args.rounds)]
            values = sorted(sample['accessible_upper_bound_ms'] for sample in samples)
            results[menu] = {'samples': samples, 'upper_bound_p95_ms': values[math.ceil(.95 * len(values)) - 1]}
    finally:
        command('--hide')
    output = {'measurement': args.input + '-to-accessibility-after-paint-upper-bound', 'menus': results,
              'limitations': [('Includes CLI startup and accessibility observation overhead' if args.input == 'cli'
                               else 'Uses an accessibility action and includes observation overhead'),
                              'Not physical pointer input and not a complete latency release gate']}
    args.output.write_text(json.dumps(output, indent=2) + '\n')
    print('Painted and accessible menu probe passed')


if __name__ == '__main__':
    main()
