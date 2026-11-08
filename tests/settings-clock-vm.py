#!/usr/bin/env python3
"""Exercise BusyBox NTP and typed clock operations inside the disposable VM."""
import ctypes
import json
import os
from pathlib import Path
import socket
import struct
import threading
import time
from gi.repository import Gio, GLib

assert os.geteuid() == 0 and Path('/run/uflinux-test-vm').exists(), 'Disposable VM only'
assert 'QEMU' in Path('/sys/class/dmi/id/sys_vendor').read_text()
bus = Gio.bus_get_sync(Gio.BusType.SYSTEM)
def operation(value):
    result = bus.call_sync('org.uflinux.Settings.System1','/org/uflinux/Settings/System',
        'org.uflinux.Settings.System1','Execute',GLib.Variant('(s)',(json.dumps(value),)),
        GLib.VariantType('(s)'),Gio.DBusCallFlags.NONE,45000,None)
    return json.loads(result.unpack()[0])

def rejected(value):
    try: operation(value)
    except GLib.Error: return
    raise AssertionError('An invalid clock operation was accepted')

clock=bus.call_sync('net.connman','/','net.connman.Clock','GetProperties',None,None,Gio.DBusCallFlags.NONE,5000,None).unpack()[0]
assert clock['TimeUpdates'] == 'manual' and clock['TimezoneUpdates'] == 'manual', clock
try:
    bus.call_sync('net.connman','/','net.connman.Clock','SetProperty',GLib.Variant('(sv)',('TimeUpdates',GLib.Variant('s','auto'))),None,Gio.DBusCallFlags.NONE,5000,None)
except GLib.Error: pass
else: raise AssertionError('ConnMan clock mutation escaped the mandatory D-Bus policy')
rejected({'operation':'set-timezone','timezone':'../../etc/passwd'})
rejected({'operation':'set-ntp','enabled':True,'servers':['pool.ntp.org;id']})
rejected({'operation':'set-time','utc_seconds':int(time.time())}) # NTP is enabled.
original=operation({'operation':'status'})
operation({'operation':'set-ntp','enabled':False,'servers':original['servers']})
assert not operation({'operation':'status'})['ntp_running']
operation({'operation':'set-time','utc_seconds':int(time.time())})

stop=threading.Event()
requests=[]
server=socket.socket(socket.AF_INET,socket.SOCK_DGRAM)
server.bind(('127.0.0.1',123));server.settimeout(0.5)
def timestamp():
    now=time.time()+2208988800
    return struct.pack('!II',int(now),int((now-int(now))*2**32))
def serve():
    while not stop.is_set():
        try: packet,address=server.recvfrom(512)
        except socket.timeout: continue
        if len(packet)<48: continue
        reply=bytearray(48);reply[0:4]=bytes([0x24,1,4,236]);reply[12:16]=b'LOCL'
        reply[16:24]=timestamp();reply[24:32]=packet[40:48]
        reply[32:40]=timestamp();reply[40:48]=timestamp()
        server.sendto(reply,address);requests.append(time.monotonic())
thread=threading.Thread(target=serve);thread.start()
try:
    operation({'operation':'set-ntp','enabled':True,'servers':['127.0.0.1']})
    deadline=time.monotonic()+90
    while not requests and time.monotonic()<deadline: time.sleep(0.5)
    assert requests, 'BusyBox ntpd did not contact the selected server'
    status=operation({'operation':'status'})
    assert status['ntp_running'] and status['servers']==['127.0.0.1']
    # Read the kernel state independently. An NTP reply is not proof of sync.
    tx=ctypes.create_string_buffer(512)
    kernel_result=ctypes.CDLL(None).adjtimex(ctypes.byref(tx))
    assert status['synchronized'] == (kernel_result>=0 and not (ctypes.c_int.from_buffer(tx,40).value & 0x40))
    names=[p.read_text().strip() for p in Path('/proc').glob('[0-9]*/comm') if p.exists()]
    assert names.count('ntpd')==1 and not any(n in names for n in ('chronyd','openntpd','systemd-timesyn'))
    command=next(p.read_bytes().replace(b'\0',b' ') for p in Path('/proc').glob('[0-9]*/cmdline') if p.exists() and p.read_bytes().split(b'\0')[0].endswith(b'/ntpd'))
    assert b' -l ' not in command, 'NTP server mode must remain disabled'
    print('UFLINUX_CLOCK_VM:'+json.dumps({'busybox_client':True,'server_requests':len(requests),
        'connman_manual_clock':True,'sole_clock_writer':True,'truthful_sync_status':True}),flush=True)
finally:
    stop.set();thread.join(timeout=2);server.close()
    operation({'operation':'set-ntp','enabled':True,'servers':original['servers']})
