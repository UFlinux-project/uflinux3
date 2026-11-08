#!/usr/bin/env python3
"""Exercise real labwc output test/apply/confirmation/rollback, not a display mock."""
import json
import os
import time
import subprocess
from gi.repository import Gio, GLib

bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
name = 'org.uflinux.Settings1'
path = '/org/uflinux/Settings'


def state():
    return json.loads(bus.call_sync(name,path,name,'GetState',None,None,0,5000,None).unpack()[0])


def call(action, connection=bus):
    return json.loads(connection.call_sync(name,path,name,'Execute',GLib.Variant('(s)',(json.dumps(action),)),None,0,20000,None).unpack()[0])


def wait(predicate, timeout=5):
    for _ in range(int(timeout/.05)):
        if predicate(state()):
            return
        time.sleep(.05)
    raise AssertionError('Display state did not converge')


def config(outputs):
    return [{**{k:o[k] for k in ['name','enabled','x','y','scale','transform','adaptive_sync']},'mode':o['current']} for o in outputs]


before=config(state()['displays']['outputs'])
assert before
invalid=[{**o,'enabled':False} for o in before]
try:
    call({'action':'apply-displays','outputs':invalid,'hdr':False})
    raise AssertionError('All outputs were disabled')
except GLib.Error:
    pass
changed=[dict(o) for o in before]
changed[-1]['x']+=80
identifier=call({'action':'apply-displays','outputs':changed,'hdr':False})['id']
wait(lambda s:s['displays']['outputs'][-1]['x']==changed[-1]['x'])
call({'action':'confirm-displays','id':identifier})
assert state()['displays']['pending'] is None
temporary=[dict(o) for o in changed]
temporary[-1]['x']+=80
call({'action':'apply-displays','outputs':temporary,'hdr':False})
wait(lambda s:s['displays']['pending'] is None,20)
wait(lambda s:s['displays']['outputs'][-1]['x']==changed[-1]['x'])
other=Gio.DBusConnection.new_for_address_sync(os.environ['DBUS_SESSION_BUS_ADDRESS'],
    Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT|Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION,None,None)
identifier=call({'action':'apply-displays','outputs':temporary,'hdr':False},other)['id']
try:
    call({'action':'confirm-displays','id':identifier})
    raise AssertionError('A different caller confirmed the configuration')
except GLib.Error:
    pass
other.close_sync(None)
wait(lambda s:s['displays']['pending'] is None)
wait(lambda s:s['displays']['outputs'][-1]['x']==changed[-1]['x'])
# Leave the reference compositor in its original layout.
identifier=call({'action':'apply-displays','outputs':before,'hdr':False})['id']
wait(lambda s:s['displays']['outputs'][-1]['x']==before[-1]['x'])
call({'action':'confirm-displays','id':identifier})
# Exercise fractional scale and rotation against the actual compositor.
scaled=[dict(o) for o in before]
scaled[0].update(scale=1.25,transform=1)
identifier=call({'action':'apply-displays','outputs':scaled,'hdr':False})['id']
wait(lambda s:abs(s['displays']['outputs'][0]['scale']-1.25)<.01 and s['displays']['outputs'][0]['transform']==1)
call({'action':'confirm-displays','id':identifier})
identifier=call({'action':'apply-displays','outputs':before,'hdr':False})['id']
call({'action':'confirm-displays','id':identifier})
# Private labwc test bindings create and remove a real headless output.
subprocess.run(['wtype','-M','ctrl','-M','alt','-k','F10','-m','alt','-m','ctrl'],check=True)
wait(lambda s:any(o['name']=='UF-TEST' for o in s['displays']['outputs']))
time.sleep(.5) # Let the startup restore transaction finish.
with_virtual=config(state()['displays']['outputs'])
virtual_only=[dict(o,enabled=o['name']=='UF-TEST') for o in with_virtual]
identifier=call({'action':'apply-displays','outputs':virtual_only,'hdr':False})['id']
call({'action':'confirm-displays','id':identifier})
subprocess.run(['wtype','-M','ctrl','-M','alt','-k','F11','-m','alt','-m','ctrl'],check=True)
wait(lambda s:len(s['displays']['outputs'])==len(before) and any(o['enabled'] for o in s['displays']['outputs']))
# A disappearing output during an unconfirmed change must also release the transaction.
subprocess.run(['wtype','-M','ctrl','-M','alt','-k','F10','-m','alt','-m','ctrl'],check=True)
wait(lambda s:any(o['name']=='UF-TEST' for o in s['displays']['outputs']))
time.sleep(.5)
temporary=config(state()['displays']['outputs'])
temporary[-1]['x']+=40
call({'action':'apply-displays','outputs':temporary,'hdr':False})
subprocess.run(['wtype','-M','ctrl','-M','alt','-k','F11','-m','alt','-m','ctrl'],check=True)
wait(lambda s:s['displays']['pending'] is None and len(s['displays']['outputs'])==len(before) and any(o['enabled'] for o in s['displays']['outputs']))
time.sleep(.5)
identifier=call({'action':'apply-displays','outputs':before,'hdr':False})['id']
call({'action':'confirm-displays','id':identifier})
print('Display test/apply, timeout, caller loss, scaling, rotation, hotplug and last-output recovery passed')
