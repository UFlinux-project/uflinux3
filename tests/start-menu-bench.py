#!/usr/bin/env python3
"""Resident launcher readiness, independent lifetime, repeated toggles and memory."""
import json
import os
from pathlib import Path
import re
import subprocess
import time

results = Path('/tmp/uflinux-settings-smoke-results')
log_path = results / 'launcher-bench.log'
env = dict(os.environ, UFLINUX_SETTINGS_BENCH='1')
def command(*args):
    return subprocess.run(['uflinux-start-menu', *args], env=env, stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE, text=True, check=True, timeout=10).stdout
def wait(predicate, message):
    for _ in range(100):
        assert primary.poll() is None, 'Resident launcher exited'
        if predicate(): return
        time.sleep(.05)
    raise AssertionError(message)
def pss(pid):
    return int(next(line.split()[1] for line in Path(f'/proc/{pid}/smaps_rollup').read_text().splitlines() if line.startswith('Pss:'))) * 1024
def ticks(pid):
    fields=Path(f'/proc/{pid}/stat').read_text().rsplit(')',1)[1].split()
    return int(fields[11])+int(fields[12])
command('--quit')
with log_path.open('w') as log:
    primary = subprocess.Popen(['uflinux-start-menu', '--daemon'], env=env, stdout=log, stderr=log)
try:
    wait(lambda: command('--status') == 'ready=true visible=false\n', 'Default screen was not prepared while hidden')
    maps = Path(f'/proc/{primary.pid}/maps').read_text()
    assert 'libpulse' not in maps, 'Launcher linked Settings audio dependencies'
    baseline=pss(primary.pid)
    samples=[]
    command('--output', 'HEADLESS-1')
    wait(lambda: 'UFLINUX_START_MENU_FIRST_FRAME_MS=' in log_path.read_text(), 'No first frame')
    command('--hide')
    wait(lambda: 'visible=false' in command('--status'), 'Hide did not unmap the menu')
    for _ in range(10):
        command(); time.sleep(.1); samples.append(pss(primary.pid)); command(); time.sleep(.1)
        assert 'visible=false' in command('--status'), 'Toggle failed to hide menu'
    closed=pss(primary.pid)
    before=ticks(primary.pid); began=time.monotonic();time.sleep(2)
    idle = (ticks(primary.pid)-before)/os.sysconf('SC_CLK_TCK')/(time.monotonic()-began)*100
    frames=[float(v) for v in re.findall(r'UFLINUX_START_MENU_FIRST_FRAME_MS=([\d.]+)',log_path.read_text())]
    metrics={'resident_first_frame_ms':frames[0], 'warm_frame_ms':frames[1:],
             'resident_hidden_pss_bytes':closed, 'launcher_pss_bytes':max(samples),
             'idle_cpu_percent':round(idle,3), 'open_close_cycles':10,
             'scope':'resident GTK launcher; prepared hidden screen; native headless labwc'}
    assert closed <= 80*1024**2,metrics
    assert closed-baseline <= 12*1024**2,metrics
    assert idle < 2,metrics
    assert not re.search(r'panicked at|CRITICAL|assertion.*failed', log_path.read_text()), log_path.read_text()
    (results/'launcher-metrics.json').write_text(json.dumps(metrics,indent=2)+'\n')
finally:
    command('--quit')
    primary.wait(timeout=10)
print('Resident launcher hidden preparation, repeated toggles, memory, idle and explicit exit passed')
