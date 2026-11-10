#!/usr/bin/env python3
"""Root-only harness inside the disposable, installed 512 MiB reference VM."""
import json
import os
from pathlib import Path
import pwd
import subprocess
import time
import sys

def diagnostics(kind,error,traceback):
    for name in ('uflinux-settings-ui.log', 'uflinux-desktop.log'):
        log=Path('/run') / name
        if log.exists(): print(name+':\n'+log.read_text(),file=sys.stderr)
    if 'frontend' in globals(): print('Primary GUI exit status:',frontend.poll(),file=sys.stderr)
    result=subprocess.run(['dmesg'],text=True,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
    print('Kernel log tail:\n'+result.stdout[-6000:],file=sys.stderr)
    sys.__excepthook__(kind,error,traceback)
sys.excepthook=diagnostics

assert os.geteuid() == 0
user = pwd.getpwnam("alice")
assert len(Path("/proc/swaps").read_text().splitlines()) == 1, "Swap must be disabled"
memory = int(next(l.split()[1] for l in Path("/proc/meminfo").read_text().splitlines() if l.startswith("MemTotal:")))
assert memory <= 512 * 1024, "Run the reference test with 512 MiB RAM"

def processes(name):
    for path in Path("/proc").glob("[0-9]*/comm"):
        try:
            if path.read_text().strip() == name and path.stat().st_uid == user.pw_uid:
                yield int(path.parent.name)
        except (OSError, ProcessLookupError):
            pass

for _ in range(90):
    daemons = list(processes("uflinux-setting"))  # Linux comm is truncated to 15 bytes.
    daemons = [pid for pid in daemons if Path(f"/proc/{pid}/exe").resolve().name == "uflinux-settingsd"]
    if daemons:
        break
    time.sleep(1)
assert daemons, "The installed graphical login did not start Settings service"
service = daemons[0]
environment = dict(item.split("=", 1) for item in Path(f"/proc/{service}/environ").read_bytes().decode().split("\0") if "=" in item)
environment["UFLINUX_SETTINGS_BENCH"] = "1"
assert environment.get("WAYLAND_DISPLAY") and environment.get("DBUS_SESSION_BUS_ADDRESS")

def demote():
    os.initgroups(user.pw_name, user.pw_gid)
    os.setgid(user.pw_gid)
    os.setuid(user.pw_uid)

def run(*args):
    if args[0] == 'uflinux-settings':
        # GApplication forwards stdio descriptors to the primary process. Do not
        # wait for pipe EOF from a GUI that intentionally remains alive.
        with Path('/run/uflinux-settings-ui.log').open('a') as log:
            subprocess.run(args,env=environment,preexec_fn=demote,stdout=log,stderr=log,check=True,timeout=30)
        return ''
    return subprocess.check_output(args, env=environment, preexec_fn=demote, text=True, timeout=30)

def state():
    return json.loads(run("uflinux-settingsctl", "status"))

for _ in range(120):
    snapshot=state()
    if snapshot['audio']['available'] and snapshot['audio']['outputs'] and snapshot['displays']['available'] and snapshot['displays']['outputs']:
        break
    time.sleep(0.5)
else: raise AssertionError('Installed audio/display services did not become ready: '+json.dumps(snapshot))
assert len([o for o in snapshot["displays"]["outputs"] if o["enabled"]]) == 1
assert snapshot["audio"]["available"]

def pss(pid):
    return int(next(l.split()[1] for l in Path(f"/proc/{pid}/smaps_rollup").read_text().splitlines() if l.startswith("Pss:"))) * 1024

def gui_pid():
    for path in Path("/proc").glob("[0-9]*/exe"):
        try:
            if path.resolve().name == "uflinux-settings" and path.parent.stat().st_uid == user.pw_uid:
                return int(path.parent.name)
        except OSError:
            pass
    raise AssertionError("Settings GUI exited unexpectedly")

# The installed desktop is supervised and remains alive while full Settings exits.
def desktop_status():
    return json.loads(run('uflinux-desktop', '--status'))

def desktop_pid():
    for path in Path('/proc').glob('[0-9]*/exe'):
        try:
            if path.resolve().name == 'uflinux-desktop' and path.parent.stat().st_uid == user.pw_uid:
                return int(path.parent.name)
        except OSError:
            pass
    raise AssertionError('Resident desktop was not found')

for _ in range(180):
    try:
        if desktop_status()['ready']:
            break
    except (subprocess.SubprocessError, json.JSONDecodeError):
        pass
    time.sleep(.5)
else:
    raise AssertionError('Desktop did not become ready')
desktop = desktop_pid()
initial_pss = pss(desktop)
output = snapshot['displays']['outputs'][0]['name']
started = time.monotonic()
for menu in ('start', 'network', 'sound', 'power'):
    command = ('--start-menu',) if menu == 'start' else ('--panel', menu)
    for _ in range(10):
        run('uflinux-desktop', *command, '--output', output)
        assert desktop_status()['popup'] == menu
        run('uflinux-desktop', '--hide')
        assert desktop_status()['popup'] is None
assert desktop_pid() == desktop
closed_pss = pss(desktop)
with Path('/run/uflinux-settings-ui.log').open('w') as log:
    frontend = subprocess.Popen(['uflinux-settings', '--page', 'network'], env=environment,
                                preexec_fn=demote, stdout=log, stderr=log)
for _ in range(60):
    assert frontend.poll() is None, 'Settings exited during startup'
    try:
        gui_pid()
        break
    except AssertionError:
        time.sleep(.5)
else:
    raise AssertionError('Settings did not start')
for page in ('network', 'displays', 'audio', 'power', 'default-apps', 'datetime', 'appearance', 'about'):
    run('uflinux-settings', '--page', page)
    time.sleep(.4)
full_pss = pss(gui_pid())
run('uflinux-settingsctl', 'volume', '-5')
run('uflinux-settingsctl', 'mute')
run('uflinux-settingsctl', 'mute')
assert state()['audio']['available']
assert list(processes('labwc')) and desktop_pid() == desktop
messages = subprocess.check_output(['dmesg'], text=True)
assert 'Out of memory' not in messages and 'Killed process' not in messages
metrics = {'ram_mib': 512, 'swap': False, 'displays': 1,
           'service_pss_bytes': pss(service), 'desktop_initial_pss_bytes': initial_pss,
           'desktop_closed_pss_bytes': closed_pss, 'full_ui_pss_bytes': full_pss,
           'exercise_seconds': round(time.monotonic() - started, 2), 'oom': False,
           'renderer': snapshot['displays']['renderer'],
           'scope': 'QEMU software-rendered installed x86_64; native latency measured separately'}
Path('/run/uflinux-settings-installed.json').write_text('UFLINUX_INSTALLED_METRICS:' + json.dumps(metrics) + '\n')
frontend.terminate()
frontend.wait(timeout=20)
assert desktop_pid() == desktop, 'Closing Settings stopped the desktop'
