#!/usr/bin/env python3
"""Measure the existing resident desktop without replacing its supervised process."""
import json
import os
from pathlib import Path
import re
import subprocess
import time

results = Path('/tmp/uflinux-settings-smoke-results')
results.mkdir(parents=True, exist_ok=True)
desktop = int(os.environ['UFLINUX_DESKTOP_PID'])
log_path = Path(os.environ['UFLINUX_DESKTOP_LOG'])
def command(*args):
    return subprocess.run(['uflinux-desktop', *args], stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE, text=True, check=True, timeout=10).stdout
def status():
    return json.loads(command('--status'))
def wait(predicate, message):
    for _ in range(100):
        os.kill(desktop, 0)
        if predicate(): return
        time.sleep(.05)
    raise AssertionError(message)
def pss(pid):
    return int(next(line.split()[1] for line in Path(f'/proc/{pid}/smaps_rollup').read_text().splitlines() if line.startswith('Pss:'))) * 1024
def ticks(pid):
    fields = Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()
    return int(fields[11]) + int(fields[12])

command('--hide')
wait(lambda: status()['ready'] and status()['popup'] is None, 'Desktop did not prepare its hidden menus')
assert 'libpulse' not in Path(f'/proc/{desktop}/maps').read_text(), 'Desktop links the audio backend'
baseline = pss(desktop)
start_offset = log_path.stat().st_size
command('--start-menu', '--output', 'HEADLESS-1')
wait(lambda: status()['popup'] == 'start', 'Start did not open')
wait(lambda: 'UFLINUX_START_MENU_FIRST_FRAME_MS=' in log_path.read_text()[start_offset:], 'No painted Start frame')
command('--hide')
samples = []
for _ in range(10):
    command('--start-menu'); wait(lambda: status()['popup'] == 'start', 'Start did not open')
    samples.append(pss(desktop))
    command('--start-menu'); wait(lambda: status()['popup'] is None, 'Start did not close')
closed = pss(desktop)
before = ticks(desktop); began = time.monotonic(); time.sleep(2)
idle = (ticks(desktop) - before) / os.sysconf('SC_CLK_TCK') / (time.monotonic() - began) * 100
frames = [float(v) for v in re.findall(r'UFLINUX_START_MENU_FIRST_FRAME_MS=([\d.]+)', log_path.read_text()[start_offset:])]
# This smoke run is diagnostic; native release gates use the dedicated multi-session harness.
metrics = {'start_frame_ms': frames, 'desktop_initial_pss_bytes': baseline,
           'desktop_closed_pss_bytes': closed, 'desktop_peak_pss_bytes': max(samples),
           'idle_cpu_percent': round(idle, 3), 'open_close_cycles': 10,
           'scope': 'resident desktop; headless smoke, not the native latency acceptance test'}
assert closed - baseline <= 12 * 1024**2, metrics
assert not re.search(r'panicked at|CRITICAL|assertion.*failed', log_path.read_text()), log_path.read_text()
(results / 'launcher-metrics.json').write_text(json.dumps(metrics, indent=2) + '\n')
print('Resident desktop Start readiness, painted frame, repeated toggles and memory passed')
