#!/usr/bin/env python3
"""Exercise ConnMan SNTP and typed clock operations inside the disposable VM.

ConnMan is UFLinux's only clock writer, so this checks its real D-Bus policy,
the timeserver list it would actually query, and the kernel's own sync flag.
The pool servers are unreachable in QEMU, so the test installs a fake NTP
server on loopback and makes it ConnMan's only configured timeserver.
"""
import ctypes
import json
import os
from pathlib import Path
import pwd
import re
import socket
import struct
import subprocess
import threading
import time
from gi.repository import Gio, GLib

assert os.geteuid() == 0 and Path('/run/uflinux-test-vm').exists(), 'Disposable VM only'
assert 'QEMU' in Path('/sys/class/dmi/id/sys_vendor').read_text()
bus = Gio.bus_get_sync(Gio.BusType.SYSTEM)
user = pwd.getpwnam('live')
main_conf = Path('/etc/connman/main.conf')
original_conf = main_conf.read_bytes()


def operation(value):
    result = bus.call_sync('org.uflinux.Settings.System1','/org/uflinux/Settings/System',
        'org.uflinux.Settings.System1','Execute',GLib.Variant('(s)',(json.dumps(value),)),
        GLib.VariantType('(s)'),Gio.DBusCallFlags.NONE,45000,None)
    return json.loads(result.unpack()[0])

def rejected(value):
    try: operation(value)
    except GLib.Error: return
    raise AssertionError('An invalid clock operation was accepted')

def clock():
    return bus.call_sync('net.connman','/','net.connman.Clock','GetProperties',None,None,
        Gio.DBusCallFlags.NONE,5000,None).unpack()[0]

def set_clock(name, value):
    bus.call_sync('net.connman','/','net.connman.Clock','SetProperty',
        GLib.Variant('(sv)',(name,GLib.Variant('s',value))),None,Gio.DBusCallFlags.NONE,5000,None)

def connected():
    services = bus.call_sync('net.connman','/','net.connman.Manager','GetServices',None,None,
        Gio.DBusCallFlags.NONE,5000,None).unpack()[0]
    return [path for path,properties in services if properties.get('State') in ('ready','online')]

def wait_for(condition, message, deadline=90):
    stop = time.monotonic()+deadline
    while time.monotonic()<stop:
        if condition(): return
        time.sleep(1)
    raise AssertionError('Timed out waiting for '+message)

def netdev_denies_clock():
    # A fresh connection is required: the bus authenticates the uid at connect time,
    # so reusing the parent's connection would still report root.
    child = os.fork()
    if child == 0:
        os.initgroups(user.pw_name,user.pw_gid)
        os.setgid(user.pw_gid)
        os.setuid(user.pw_uid)
        try:
            Gio.bus_get_sync(Gio.BusType.SYSTEM).call_sync('net.connman','/','net.connman.Clock',
                'SetProperty',GLib.Variant('(sv)',('TimeUpdates',GLib.Variant('s','manual'))),
                None,Gio.DBusCallFlags.NONE,5000,None)
        except GLib.Error:
            os._exit(0)
        os._exit(1)
    return os.waitpid(child,0)[1] == 0

def only_timeserver(address):
    text = original_conf.decode()
    line = 'FallbackTimeservers='+address
    pattern = re.compile(r'(?m)^FallbackTimeservers=.*$')
    main_conf.write_text(pattern.sub(line,text) if pattern.search(text)
        else text.rstrip('\n')+'\n'+line+'\n')

# ConnMan owns the policy: synchronization automatic, timezone left to Settings.
properties = clock()
assert properties['TimeUpdates'] == 'auto' and properties['TimezoneUpdates'] == 'manual', properties
# Root is the trusted side of the clock interface, so the round trip must work.
set_clock('TimeUpdates','manual')
assert clock()['TimeUpdates'] == 'manual'
set_clock('TimeUpdates','auto')
assert netdev_denies_clock(), 'netdev was allowed to mutate net.connman.Clock'
rejected({'operation':'set-timezone','timezone':'../../etc/passwd'})
rejected({'operation':'set-time-sync','enabled':True,'servers':['pool.ntp.org']})
rejected({'operation':'set-ntp','enabled':True})
rejected({'operation':'set-time','utc_seconds':int(time.time())}) # Synchronization is automatic.
original = operation({'operation':'status'})
assert original['time_sync_enabled'] and original['sntp_running'], original

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
        # ConnMan sets LI=3 (its own clock is unsynced), so the first byte is 0xe3:
        # mask to the version field (bits 3-5) and require client mode (bits 0-2).
        if len(packet)<48 or (packet[0]>>3)&7 not in (3,4) or packet[0]&7 != 3: continue
        reply=bytearray(48);reply[0:4]=bytes([0x24,1,4,236]);reply[12:16]=b'LOCL'
        reply[16:24]=timestamp();reply[24:32]=packet[40:48]
        reply[32:40]=timestamp();reply[40:48]=timestamp()
        server.sendto(reply,address);requests.append(time.monotonic())
thread=threading.Thread(target=serve);thread.start()
try:
    only_timeserver('127.0.0.1')
    subprocess.run(['rc-service','connman','restart'],check=True,timeout=90)
    wait_for(connected,'a connected ConnMan service')
    # Toggling the policy makes ConnMan rebuild its list and query immediately.
    operation({'operation':'set-time-sync','enabled':False})
    disabled=operation({'operation':'status'})
    assert not disabled['time_sync_enabled'] and not disabled['sntp_running'], disabled
    assert disabled['synchronized'] is None, 'A manual clock must not claim to be synchronized'
    # With automatic synchronization off, the user owns the clock again.
    operation({'operation':'set-time','utc_seconds':int(time.time())})
    operation({'operation':'set-time-sync','enabled':True})
    deadline=time.monotonic()+120
    while not requests and time.monotonic()<deadline: time.sleep(0.5)
    assert requests, 'ConnMan SNTP did not contact the configured server'
    status=operation({'operation':'status'})
    assert status['sntp_running'] and status['timeservers'], status
    assert '127.0.0.1' in status['timeservers'], status
    assert not any(server.endswith('.pool.ntp.org') for server in status['timeservers']), status
    wait_for(lambda: operation({'operation':'status'})['synchronized'] is True,
        'the kernel to accept a clock update',60)
    # Read the kernel state independently. An NTP reply is not proof of sync.
    tx=ctypes.create_string_buffer(512)
    kernel_result=ctypes.CDLL(None).adjtimex(ctypes.byref(tx))
    status=operation({'operation':'status'})
    assert status['synchronized'] == (kernel_result>=0 and not (ctypes.c_int.from_buffer(tx,40).value & 0x40))
    names=[p.read_text().strip() for p in Path('/proc').glob('[0-9]*/comm') if p.exists()]
    assert 'connmand' in names and not any(n in names for n in
        ('ntpd','chronyd','openntpd','systemd-timesyn')), names
    print('UFLINUX_CLOCK_VM:'+json.dumps({'connman_sntp_client':True,'server_requests':len(requests),
        'root_clock_mutation_allowed':True,'netdev_clock_mutation_denied':True,
        'effective_timeserver_list':True,'sole_clock_writer':True,'truthful_sync_status':True}),flush=True)
finally:
    stop.set();thread.join(timeout=2);server.close()
    main_conf.write_bytes(original_conf)
    subprocess.run(['rc-service','connman','restart'],check=True,timeout=90)
    if not original['time_sync_enabled']:
        operation({'operation':'set-time-sync','enabled':True})
