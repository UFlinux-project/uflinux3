#!/usr/bin/env python3
"""Native launcher lifetime and memory benchmark in the private labwc session."""
import json
import os
from pathlib import Path
import re
import subprocess
import time

results = Path('/tmp/uflinux-settings-smoke-results')
log_path = results / 'launcher-bench.log'
env = dict(os.environ, UFLINUX_SETTINGS_BENCH='1')
def toggle():
    subprocess.run(['uflinux-settings', '--start-menu'], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True, timeout=10)
def pss(pid):
    return int(next(line.split()[1] for line in Path(f'/proc/{pid}/smaps_rollup').read_text().splitlines() if line.startswith('Pss:'))) * 1024
def ticks(pid):
    fields=Path(f'/proc/{pid}/stat').read_text().rsplit(')',1)[1].split()
    return int(fields[11])+int(fields[12])
with log_path.open('w') as log:
    primary = subprocess.Popen(['uflinux-settings', '--start-menu'], env=env, stdout=log, stderr=log)
try:
    for _ in range(100):
        assert primary.poll() is None, 'Cold launcher failed'
        if 'UFLINUX_START_MENU_FIRST_FRAME_MS=' in log_path.read_text(): break
        time.sleep(.05)
    else: raise AssertionError('No launcher frame')
    time.sleep(2)
    cold_pss = pss(primary.pid)
    before=ticks(primary.pid); began=time.monotonic();time.sleep(2)
    idle = (ticks(primary.pid)-before)/os.sysconf('SC_CLK_TCK')/(time.monotonic()-began)*100
    # Opening full Settings keeps the process alive while the launcher is rebuilt.
    subprocess.run(['uflinux-settings','--page','network'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,check=True,timeout=10)
    time.sleep(.5)
    baseline=pss(primary.pid);samples=[]
    for _ in range(10):
        toggle();time.sleep(.1);samples.append(pss(primary.pid));toggle();time.sleep(.1)
    closed=pss(primary.pid)
    frames=[float(v) for v in re.findall(r'UFLINUX_START_MENU_FIRST_FRAME_MS=([\d.]+)',log_path.read_text())]
    metrics={'cold_frame_ms':frames[0],'warm_frame_ms':frames[1:], 'launcher_pss_bytes':cold_pss,
             'idle_cpu_percent':round(idle,3),'repeated_peak_pss_bytes':max(samples),
             'full_before_pss_bytes':baseline,'full_after_pss_bytes':closed,'open_close_cycles':10,
             'scope':'native x86_64 private headless labwc, cairo GTK renderer; cached build host'}
    assert cold_pss <= 80*1024**2,metrics
    assert closed-baseline <= 12*1024**2,metrics
    (results/'launcher-metrics.json').write_text(json.dumps(metrics,indent=2)+'\n')
    subprocess.run(['wlrctl','toplevel','close','app_id:org.uflinux.Settings'],check=True,timeout=10)
    primary.wait(timeout=10)
finally:
    if primary.poll() is None: primary.terminate();primary.wait(timeout=10)
print('Launcher startup, memory, repeated-use and process-exit checks passed')
