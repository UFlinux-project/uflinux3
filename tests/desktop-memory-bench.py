#!/usr/bin/env python3
"""Measure desktop PSS and idle CPU in one disposable native labwc session.

Run separately for baseline and replacement, at least three times each. This
collector intentionally does not assert the release gate: latency, interactive
frames, widget/subscription growth, architecture and VM checks are separate.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import time


ROOT = Path(__file__).resolve().parents[1]
MENUS = ('start', 'network', 'sound', 'power')


def pss(pid):
    for line in Path(f'/proc/{pid}/smaps_rollup').read_text().splitlines():
        if line.startswith('Pss:'):
            return int(line.split()[1]) * 1024
    raise RuntimeError(f'PSS missing for process {pid}')


def ticks(pid):
    fields = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
    # Include exited children as well as the process itself.
    return sum(int(fields[index]) for index in (11, 12, 13, 14))


def process_tree(pid):
    result, pending = [], [pid]
    while pending:
        current = pending.pop()
        path = Path(f'/proc/{current}/task/{current}/children')
        try:
            children = path.read_text().split()
        except FileNotFoundError:
            continue
        result.append(current)
        pending.extend(int(child) for child in children)
    return result


def tree_measure(pid, measure):
    total = 0
    for current in process_tree(pid):
        try:
            total += measure(current)
        except FileNotFoundError:
            pass  # A short-lived supervisor child exited between enumeration and reading.
    return total


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def catalog_digest(environment):
    digestor = hashlib.sha256()
    roots = [environment['XDG_DATA_HOME'], *environment.get('XDG_DATA_DIRS', '/usr/local/share:/usr/share').split(':')]
    for root in roots:
        directory = Path(root) / 'applications'
        if directory.is_dir():
            for path in sorted(directory.rglob('*.desktop')):
                if path.is_file():
                    digestor.update(str(path.relative_to(directory)).encode())
                    digestor.update(path.read_bytes())
    return digestor.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--mode', required=True, choices=('baseline', 'replacement'))
    parser.add_argument('--legacy-dir', type=Path, required=True, help='Original settings release binaries')
    parser.add_argument('--ironbar', type=Path, required=True, help='Original Ironbar executable')
    parser.add_argument('--desktop', type=Path, required=True, help='Replacement executable')
    parser.add_argument('--daemon', type=Path, required=True, help='Identical settings daemon for both sessions')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--cycles', type=int, default=100)
    parser.add_argument('--idle-seconds', type=float, default=60)
    args = parser.parse_args()
    if args.cycles < 1 or args.idle_seconds <= 0:
        parser.error('cycles and idle duration must be positive')
    if 'UFLINUX_BENCH_BUS' not in os.environ:
        env = dict(os.environ, UFLINUX_BENCH_BUS='1')
        os.execvpe('dbus-run-session', ['dbus-run-session', '--', sys.executable, *sys.argv], env)

    args.output.parent.mkdir(parents=True, exist_ok=True)
    processes = []
    logs = []
    with tempfile.TemporaryDirectory(prefix='uflinux-memory-bench-') as temporary:
        runtime = Path(temporary)
        runtime.chmod(0o700)
        (runtime / 'config').mkdir()
        (runtime / 'config/autostart').touch()
        # Snapshot fixtures so edits during a long run cannot alter its inputs.
        fixtures = {}
        for name in ('mock-connman.py', 'mock-login1.py', 'desktop-tray-protocol.py'):
            fixtures[name] = runtime / name
            shutil.copyfile(ROOT / 'tests' / name, fixtures[name])
        env = dict(os.environ, XDG_RUNTIME_DIR=str(runtime), XDG_CONFIG_HOME=str(runtime / 'preferences'),
                   XDG_DATA_HOME=str(runtime / 'data'), XDG_CACHE_HOME=str(runtime / 'cache'),
                   WLR_BACKENDS='headless', WLR_HEADLESS_OUTPUTS='1', WLR_RENDERER='pixman',
                   WLR_LIBINPUT_NO_DEVICES='1', WAYLAND_DISPLAY='wayland-0', GSK_RENDERER='cairo',
                   XDG_SESSION_TYPE='wayland', XDG_CURRENT_DESKTOP='labwc',
                   DBUS_SYSTEM_BUS_ADDRESS=os.environ['DBUS_SESSION_BUS_ADDRESS'])
        env['PATH'] = str(args.legacy_dir) + os.pathsep + env['PATH']

        def spawn(name, command):
            stream = (args.output.parent / f'{args.output.stem}-{name}.log').open('w')
            logs.append(stream)
            process = subprocess.Popen([str(part) for part in command], env=env, stdout=stream, stderr=stream)
            processes.append(process)
            return process

        def wait_for(predicate, message, timeout=15):
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                if predicate():
                    return
                if any(process.poll() is not None for process in processes):
                    raise RuntimeError(f'Process exited while waiting for {message}; see session logs')
                time.sleep(.05)
            raise RuntimeError(f'Timed out waiting for {message}')

        def run(command):
            return subprocess.run([str(part) for part in command], env=env, capture_output=True,
                                  text=True, check=True, timeout=15).stdout

        def dbus_owned(name):
            result = subprocess.run(['gdbus', 'call', '--session', '--dest', 'org.freedesktop.DBus',
                                     '--object-path', '/org/freedesktop/DBus', '--method',
                                     'org.freedesktop.DBus.NameHasOwner', name], env=env,
                                    capture_output=True, text=True, timeout=5)
            return result.returncode == 0 and 'true' in result.stdout.lower()

        try:
            compositor = spawn('labwc', ['labwc', '--config-dir', runtime / 'config'])
            spawn('connman', [sys.executable, fixtures['mock-connman.py']])
            spawn('login1', [sys.executable, fixtures['mock-login1.py']])
            wait_for(lambda: (runtime / 'wayland-0').is_socket(), 'Wayland socket')
            wait_for(lambda: dbus_owned('net.connman') and dbus_owned('org.freedesktop.login1'), 'device fixtures')
            daemon = spawn('settingsd', [args.daemon])
            wait_for(lambda: dbus_owned('org.uflinux.Settings1'), 'Settings service')
            if args.mode == 'baseline':
                panel = spawn('ironbar', [args.ironbar, '-c', ROOT / 'tests/fixtures/ironbar-baseline/ironbar.toml',
                                          '-t', ROOT / 'tests/fixtures/ironbar-baseline/ironbar.css'])
                primary = spawn('start', [args.legacy_dir / 'uflinux-start-menu', '--daemon'])
                accounting = [daemon, panel, primary]
                wait_for(lambda: 'ready=true' in run([args.legacy_dir / 'uflinux-start-menu', '--status']), 'Start readiness')
            else:
                bindir = runtime / 'bin'
                bindir.mkdir()
                (bindir / 'uflinux-desktop').symlink_to(args.desktop.resolve())
                env['PATH'] = str(bindir) + os.pathsep + env['PATH']
                env['LABWC_PID'] = str(compositor.pid)
                supervisor = spawn('desktop-supervisor', ['sh', ROOT / 'settings/desktop/data/uflinux-desktop-supervisor'])
                accounting = [daemon, supervisor]
                wait_for(lambda: json.loads(run([args.desktop, '--status']))['ready'], 'desktop readiness')
                reply = run(['gdbus', 'call', '--session', '--dest', 'org.freedesktop.DBus',
                             '--object-path', '/org/freedesktop/DBus', '--method',
                             'org.freedesktop.DBus.GetConnectionUnixProcessID', 'org.uflinux.Desktop'])
                match = re.search(r'uint32\s+(\d+)', reply)
                if not match:
                    raise RuntimeError(f'Could not identify the desktop process: {reply}')
                primary_pid = int(match.group(1))
            wait_for(lambda: dbus_owned('org.kde.StatusNotifierWatcher'), 'tray watcher')
            spawn('tray-fixture', [sys.executable, fixtures['desktop-tray-protocol.py'], '--item', runtime / 'tray-marker'])
            def tray_registered():
                value = run(['gdbus', 'call', '--session', '--dest', 'org.kde.StatusNotifierWatcher',
                             '--object-path', '/StatusNotifierWatcher', '--method',
                             'org.freedesktop.DBus.Properties.Get', 'org.kde.StatusNotifierWatcher',
                             'RegisteredStatusNotifierItems'])
                return '/StatusNotifierItem' in value
            wait_for(tray_registered, 'tray fixture registration')
            time.sleep(.5)

            def total():
                return sum(tree_measure(process.pid, pss) for process in accounting)

            def cycle(menu):
                if args.mode == 'replacement':
                    command = [args.desktop, '--start-menu'] if menu == 'start' else [args.desktop, '--panel', menu]
                    run([*command, '--output', 'HEADLESS-1'])
                    wait_for(lambda: json.loads(run([args.desktop, '--status']))['popup'] == menu, f'{menu} open')
                    run([args.desktop, '--hide'])
                    wait_for(lambda: json.loads(run([args.desktop, '--status']))['popup'] is None, f'{menu} close')
                elif menu == 'start':
                    run([args.legacy_dir / 'uflinux-start-menu', '--output', 'HEADLESS-1'])
                    wait_for(lambda: 'visible=true' in run([args.legacy_dir / 'uflinux-start-menu', '--status']), 'Start open')
                    run([args.legacy_dir / 'uflinux-start-menu', '--hide'])
                else:
                    command = [args.legacy_dir / 'uflinux-settings', '--panel', menu, '--output', 'HEADLESS-1']
                    temporary_log = (args.output.parent / f'{args.output.stem}-quick.log').open('a')
                    quick = None
                    try:
                        quick = subprocess.Popen([str(part) for part in command], env=env,
                                                 stdout=temporary_log, stderr=temporary_log)
                        wait_for(lambda: dbus_owned('org.uflinux.Settings'), f'{menu} service')
                        # Allow the quick control to paint and bind the service snapshot.
                        time.sleep(.2)
                        run(command)
                        quick.wait(timeout=10)
                    finally:
                        if quick is not None and quick.poll() is None:
                            quick.terminate()
                            quick.wait(timeout=5)
                        temporary_log.close()

            settled = total()
            for menu in MENUS:
                cycle(menu)
            time.sleep(.5)
            exercised = total()
            # Warm each retained control before checking additional retention.
            for menu in MENUS:
                cycle(menu)
            time.sleep(.5)
            warmed = total()
            measured_pid = primary.pid if args.mode == 'baseline' else primary_pid
            before_fds = len(list(Path(f'/proc/{measured_pid}/fd').iterdir()))
            for cycle_index in range(args.cycles):
                for menu in MENUS:
                    cycle(menu)
                if (cycle_index + 1) % 10 == 0:
                    print(f'{args.mode}: completed {cycle_index + 1}/{args.cycles} menu cycles', file=sys.stderr, flush=True)
            time.sleep(.5)
            cycled = total()
            after_fds = len(list(Path(f'/proc/{measured_pid}/fd').iterdir()))
            before = {p.pid: tree_measure(p.pid, ticks) for p in accounting}
            began = time.monotonic()
            print(f'{args.mode}: measuring idle CPU for {args.idle_seconds:g} seconds', file=sys.stderr, flush=True)
            time.sleep(args.idle_seconds)
            elapsed = time.monotonic() - began
            cpu = {p.pid: max(0, tree_measure(p.pid, ticks) - before[p.pid]) / os.sysconf('SC_CLK_TCK') / elapsed * 100
                   for p in accounting}
            result = {'mode': args.mode, 'cycles': args.cycles, 'accounting_version': 2,
                      'environment': {'compositor': 'labwc', 'renderer': 'cairo', 'backend': 'headless-pixman',
                                      'outputs': 1, 'device_fixture': 'mock-connman-and-login1',
                                      'tray_fixture': 'desktop-tray-protocol', 'machine': os.uname().machine,
                                      'kernel': os.uname().release, 'catalog_sha256': catalog_digest(env),
                                      'device_fixture_sha256': [digest(fixtures['mock-connman.py']),
                                                                 digest(fixtures['mock-login1.py'])],
                                      'tray_fixture_sha256': digest(fixtures['desktop-tray-protocol.py'])},
                      'binary_sha256': {'settingsd': digest(args.daemon),
                                        'desktop': digest(args.desktop if args.mode == 'replacement' else args.legacy_dir / 'uflinux-start-menu'),
                                        **({'ironbar': digest(args.ironbar), 'settings': digest(args.legacy_dir / 'uflinux-settings')}
                                           if args.mode == 'baseline' else {})},
                      'accounted_processes': ['settingsd', 'ironbar', 'start'] if args.mode == 'baseline' else ['settingsd', 'supervisor and descendants'],
                      'memory': {'settled_pss_bytes': settled, 'exercised_pss_bytes': exercised,
                                 'warmed_pss_bytes': warmed, 'cycled_pss_bytes': cycled},
                      'file_descriptors': {'warmed': before_fds, 'cycled': after_fds},
                      'idle': {'seconds': elapsed, 'desktop_percent': cpu[primary.pid if args.mode == 'baseline' else supervisor.pid],
                               'combined_percent': sum(cpu.values())},
                      'limitations': ['Menu cycles check popup state, not a painted interactive frame',
                                      'Memory/CPU diagnostic only; no latency or widget/subscription evidence',
                                      'Native private headless session, not an installed VM']}
            args.output.write_text(json.dumps(result, indent=2) + '\n')
            print(json.dumps(result, indent=2))
        finally:
            for process in reversed(processes):
                if process.poll() is None:
                    process.terminate()
            for process in reversed(processes):
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
            for stream in logs:
                stream.close()
    return 0


if __name__ == '__main__':
    sys.exit(main())
