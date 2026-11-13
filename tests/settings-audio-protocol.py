#!/usr/bin/env python3
"""Real PipeWire/Pulse subscriptions while the Settings UI is already open."""
import json
from pathlib import Path
import subprocess
import tempfile
import time
import wave
import pyatspi
from gi.repository import Gio, GLib

bus=Gio.bus_get_sync(Gio.BusType.SESSION)
def call(method, action=None):
    args=GLib.Variant('(s)',(json.dumps(action),)) if action else None
    # Every method here is idempotent, so retry while the audio server has a
    # transient stall (common under s390x emulation) instead of failing the run.
    for _ in range(3):
        try:
            reply=bus.call_sync('org.uflinux.Settings1','/org/uflinux/Settings',
                'org.uflinux.Settings1',method,args,None,0,30000,None)
            break
        except GLib.Error:
            time.sleep(1)
    else:
        raise AssertionError('Settings daemon stopped answering D-Bus calls')
    return json.loads(reply.unpack()[0])
def execute(action): return call('Execute', action)
def state(): return call('GetState')['audio']
def wait(predicate):
    for _ in range(600):
        snapshot=state();value=predicate(snapshot)
        if value: return value
        time.sleep(0.05)
    raise AssertionError('Audio subscriptions did not converge')

previous=state()['default_output']
module=subprocess.check_output(['pactl','load-module','module-null-sink','sink_name=uflinux_alternate',
    'sink_properties=device.description=UFLinux-Alternate-Output'],text=True).strip()
children=[]
unloaded=False
try:
    wait(lambda s:any(d['name']=='uflinux_alternate' for d in s['outputs']))
    # Exercise the taskbar selector through GTK, including its live selected marker.
    subprocess.run(['uflinux-desktop','--panel','sound','--output','HEADLESS-1'],check=True)
    def accessible_children(item):
        # A child accessible can be torn down between two hops of the walk, in
        # which case pyatspi reports it as None. Skip the gap instead of
        # abandoning the rest of the queue.
        try: return [child for child in item if child is not None]
        except Exception: return []
    def find_control(name):
        queue=[app for app in pyatspi.Registry.getDesktop(0) if app.name=='uflinux-desktop']
        while queue:
            item=queue.pop(0)
            try:
                if (item.name==name and item.getRoleName() in ('button','push button','toggle button')
                        and item.getState().contains(pyatspi.STATE_SHOWING)
                        and item.queryAction().nActions): return item
            except (RuntimeError,LookupError,AttributeError): pass
            queue.extend(accessible_children(item))
        return None
    def wait_control(*names):
        for _ in range(600):
            while GLib.MainContext.default().pending(): GLib.MainContext.default().iteration(False)
            for name in names:
                item=find_control(name)
                if item: return item
            time.sleep(.05)
        raise AssertionError('Missing accessible sound control: '+' or '.join(names))
    def click(item):
        queue=[item]
        while queue:
            item=queue.pop(0)
            try:
                action=item.queryAction()
                if action.nActions:
                    assert action.doAction(0)
                    return
            except (NotImplementedError,RuntimeError,AttributeError): pass
            queue.extend(accessible_children(item))
        raise AssertionError('Control has no accessible action')
    click(wait_control('Choose output device'))
    # WirePlumber may already have made the new sink the default.
    click(wait_control('UFLinux-Alternate-Output', 'UFLinux-Alternate-Output (selected)'))
    wait(lambda s:s['default_output']=='uflinux_alternate')
    # GTK may finish dismissing the popover asynchronously, especially under
    # emulation. Check both labels because the selected marker can update first.
    for _ in range(600):
        while GLib.MainContext.default().pending(): GLib.MainContext.default().iteration(False)
        if not (find_control('UFLinux-Alternate-Output') or
                find_control('UFLinux-Alternate-Output (selected)')):
            break
        time.sleep(.05)
    else:
        raise AssertionError('Output selector did not close after selection')
    click(wait_control('Choose output device'))
    wait_control('UFLinux-Alternate-Output (selected)')
    # Close the popover and panel before continuing the backend checks.
    click(wait_control('Choose output device'))
    subprocess.run(['uflinux-desktop','--hide'],check=True)
    execute({'action':'set-volume','input':False,'device':'','percent':34})
    execute({'action':'set-mute','input':False,'device':'','muted':True})
    wait(lambda s:any(d['name']=='uflinux_alternate' and d['muted'] and abs(d['volume']-34)<1 for d in s['outputs']))
    execute({'action':'set-mute','input':False,'device':'','muted':False})
    with tempfile.TemporaryDirectory() as directory:
        sample=Path(directory)/'silence.wav'
        with wave.open(str(sample),'wb') as output:
            output.setnchannels(1);output.setsampwidth(2);output.setframerate(48000)
            output.writeframes(bytes(48000*2*20))
        children.append(subprocess.Popen(['paplay','--device=uflinux_alternate','--client-name=uflinux-audio-test',str(sample)]))
        children.append(subprocess.Popen(['parec','--device=uflinux_alternate.monitor','--client-name=uflinux-recording-test'],stdout=subprocess.DEVNULL))
        for input_stream in (False,True):
            stream=wait(lambda s:next((v for v in s['streams'] if v['input']==input_stream),None))
            execute({'action':'set-stream','input':input_stream,'index':stream['index'],'percent':23,'muted':True})
            wait(lambda s:any(v['index']==stream['index'] and v['input']==input_stream and v['muted'] and abs(v['volume']-23)<1 for v in s['streams']))
    # WirePlumber removes its automatic dummy sink while a real sink exists.
    subprocess.run(['pactl','unload-module',module],check=True);unloaded=True
    wait(lambda s:any(d['name']==previous for d in s['outputs']))
    execute({'action':'set-default','input':False,'device':previous})
    wait(lambda s:s['default_output']==previous)
    print('PipeWire device/default/volume/mute and playback/recording stream subscriptions passed')
finally:
    for child in children:
        child.terminate();child.wait(timeout=5)
    # A killed stream can linger on an emulated runner, keeping the module
    # busy for a moment after its client is gone.
    if not unloaded:
        for _ in range(60):
            if subprocess.run(['pactl','unload-module',module]).returncode == 0: break
            time.sleep(0.5)
        else:
            raise subprocess.CalledProcessError(1,['pactl','unload-module',module])
