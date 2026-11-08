#!/usr/bin/env python3
"""Persist wallpaper state, restart its owner and preserve unrelated swaybg clients."""
import json
import os
from pathlib import Path
import signal
import subprocess
import time
from gi.repository import Gio,GLib
bus=Gio.bus_get_sync(Gio.BusType.SESSION)
def call(method,action=None):
    value=GLib.Variant('(s)',(json.dumps(action),)) if action else None
    return json.loads(bus.call_sync('org.uflinux.Settings1','/org/uflinux/Settings','org.uflinux.Settings1',method,value,None,Gio.DBusCallFlags.NO_AUTO_START,15000,None).unpack()[0])
def wait(predicate):
    for _ in range(80):
        try:
            if predicate():return
        except GLib.Error:pass
        time.sleep(.1)
    raise AssertionError('Preferences or owned wallpaper did not converge')
def wallpaper_children():
    daemon=int(Path('/tmp/uflinux-settings-smoke-results/service.pid').read_text())
    found=[]
    for p in Path('/proc').glob('[0-9]*/comm'):
        try:
            stat=(p.parent/'stat').read_text().rsplit(')',1)[1].split()
            if p.read_text().strip()=='swaybg' and int(stat[1])==daemon:found.append(int(p.parent.name))
        except OSError:pass
    return found
previous=call('GetState')['appearance']
wallpaper={'wallpapers':[{'output':'*','file':'/usr/share/backgrounds/uflinux/Dark_Ivy.jpg','fit':'fit'}],'color':'#153658'}
foreign=subprocess.Popen(['swaybg','-c','#202020'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
try:
    assert foreign.poll() is None
    call('Execute',{'action':'set-appearance','appearance':wallpaper})
    wait(lambda:call('GetState')['appearance']==wallpaper and len(wallpaper_children())==1)
    stored=Path(os.environ['XDG_CONFIG_HOME'])/'uflinux/appearance.json'
    assert json.loads(stored.read_text())==wallpaper and stored.stat().st_mode&0o777==0o600
    initial=wallpaper_children()[0]
    # SIGHUP is not a daemon restart API: restart only this test's owned process.
    daemon=int(Path('/tmp/uflinux-settings-smoke-results/service.pid').read_text())
    os.kill(daemon,signal.SIGTERM)
    wait(lambda:not Path('/proc/'+str(daemon)).exists())
    with Path('/tmp/uflinux-settings-smoke-results/service.log').open('a') as log:
        restarted=subprocess.Popen(['/usr/libexec/uflinux-settingsd'],stdout=log,stderr=log)
    Path('/tmp/uflinux-settings-smoke-results/service.pid').write_text(str(restarted.pid))
    wait(lambda:restarted.poll() is None and call('GetState')['appearance']==wallpaper and len(wallpaper_children())==1)
    assert initial not in wallpaper_children() and foreign.poll() is None
    assert not Path('/proc/'+str(initial)).exists(), 'Old owned wallpaper leaked'
    call('Execute',{'action':'set-appearance','appearance':previous})
    print('Wallpaper persistence, session-service restart, process ownership and restoration passed')
finally:
    foreign.terminate();foreign.wait(timeout=5)
