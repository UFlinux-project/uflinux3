#!/usr/bin/env python3
"""Exercise actual settings service + ConnMan agent against the D-Bus fixture."""
import json
import time
from gi.repository import Gio, GLib

bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
path = "/org/uflinux/Settings"
name = "org.uflinux.Settings1"
service = "/net/connman/service/wifi_000000000001_55464c696e75782054657374_managed_psk"


def call(action):
    result = bus.call_sync(name, path, name, "Execute", GLib.Variant("(s)", (json.dumps(action),)), None, 0, 120000, None)
    return json.loads(result.unpack()[0])


def state():
    return json.loads(bus.call_sync(name, path, name, "GetState", None, None, 0, 5000, None).unpack()[0])


def wait(predicate, timeout=5):
    for _ in range(int(timeout/.05)):
        value = state()
        if predicate(value):
            return value
        while GLib.MainContext.default().iteration(False):
            pass
        time.sleep(.05)
    raise AssertionError("Settings did not converge to the expected network state")


wait(lambda s: s['network']['available'])
call({'action': 'networking', 'enabled': False})
wait(lambda s: s['network']['offline'])
call({'action': 'networking', 'enabled': True})
call({'action': 'wifi', 'enabled': False})
wait(lambda s: not s['network']['technologies'][0]['powered'])
call({'action': 'wifi', 'enabled': True})
call({'action': 'scan'})
pending = []


def connected(connection, result):
    try:
        pending.append(('ok', connection.call_finish(result)))
    except GLib.Error:
        pending.append(('error', None))


def connect():
    pending.clear()
    bus.call(name, path, name, 'Execute', GLib.Variant('(s)', (json.dumps({'action': 'connect', 'service': service}),)),
             None, 0, 120000, None, connected)
    return wait(lambda s: s['network']['authentication'] is not None)['network']['authentication']['id']


identifier = connect()
call({'action': 'auth-cancel', 'id': identifier})
wait(lambda s: s['network']['authentication'] is None)
identifier = connect()
call({'action': 'auth-reply', 'id': identifier, 'values': {'Passphrase': 'uflinux-test-secret'}})
wait(lambda s: s['network']['services'][0]['state'] == 'ready')
assert 'uflinux-test-secret' not in json.dumps(state())
call({'action': 'auto-connect', 'service': service, 'enabled': True})
wait(lambda s: s['network']['services'][0]['autoconnect'])
call({'action': 'configure-connection', 'service': service, 'config': {
    'ipv4': {'Method': 'manual', 'Address': '192.0.2.2', 'Netmask': '255.255.255.0', 'Gateway': '192.0.2.1'},
    'ipv6': {'Method': 'auto'}, 'dns': ['192.0.2.1', '2001:db8::1'], 'domains': ['example.test'],
    'proxy': {'method': 'manual', 'url': '', 'servers': ['http://192.0.2.1:3128'], 'excludes': ['localhost']}}})
call({'action': 'disconnect', 'service': service})
wait(lambda s: s['network']['services'][0]['state'] == 'idle')
call({'action': 'forget', 'service': service})
wait(lambda s: not s['network']['services'][0]['favorite'])
print('ConnMan protocol, credentials/cancel, power controls and connection configuration passed')
