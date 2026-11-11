#!/usr/bin/env python3
"""Exercise desktop state updates and Settings service restart on a private bus."""
import json
import os
from pathlib import Path
import subprocess
import sys
import time
from gi.repository import Gio, GLib

NAME = 'org.uflinux.Settings1'
PATH = '/org/uflinux/Settings'
XML = '''<node><interface name="org.uflinux.Settings1">
<method name="GetState"><arg type="s" direction="out"/></method>
<method name="Execute"><arg type="s" direction="in"/><arg type="s" direction="out"/></method>
<method name="SetOffline"><arg type="b" direction="in"/></method>
<method name="SetStreams"><arg type="i" direction="in"/></method>
<method name="RequestAuth"/>
<signal name="Changed"><arg type="s"/></signal></interface></node>'''


def serve():
    state = json.loads(os.environ['UFLINUX_FAKE_SNAPSHOT'])
    state['network']['available'] = True
    state['network']['offline'] = True
    bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    info = Gio.DBusNodeInfo.new_for_xml(XML).interfaces[0]
    def method(_bus, _sender, _path, _interface, name, params, invocation):
        if name == 'GetState': invocation.return_value(GLib.Variant('(s)', (json.dumps(state),)))
        elif name == 'Execute':
            action = json.loads(params.unpack()[0])
            if action.get('action') == 'auth-cancel':
                state['network']['authentication'] = None
                bus.emit_signal(None, PATH, NAME, 'Changed', GLib.Variant('(s)', (json.dumps(state),)))
            invocation.return_value(GLib.Variant('(s)', ('{}',)))
        elif name == 'SetOffline':
            state['network']['offline'] = params.unpack()[0]
            invocation.return_value(None)
            bus.emit_signal(None, PATH, NAME, 'Changed', GLib.Variant('(s)', (json.dumps(state),)))
        elif name == 'RequestAuth':
            state['network']['authentication'] = {'id': 123, 'service': '/fixture', 'name': 'Fixture WiFi',
                'fields': [{'name': 'Passphrase', 'secret': True, 'required': True}]}
            invocation.return_value(None)
            bus.emit_signal(None, PATH, NAME, 'Changed', GLib.Variant('(s)', (json.dumps(state),)))
        elif name == 'SetStreams':
            count = params.unpack()[0]
            state['audio']['available'] = True
            state['audio']['streams'] = [{'index': index, 'name': f'Fixture stream {index}',
                                          'input': False, 'volume': 50., 'muted': False}
                                         for index in range(1, count + 1)]
            invocation.return_value(None)
            bus.emit_signal(None, PATH, NAME, 'Changed', GLib.Variant('(s)', (json.dumps(state),)))
    bus.register_object(PATH, info, method, None, None)
    bus.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus',
                  'RequestName', GLib.Variant('(su)', (NAME, 4)), None, Gio.DBusCallFlags.NONE, 5000, None)
    print('ready', flush=True)
    GLib.MainLoop().run()


def run():
    import pyatspi
    env = os.environ.copy()
    env['UFLINUX_FAKE_SNAPSHOT'] = subprocess.check_output([env['UFLINUX_SETTINGS_SNAPSHOT_BIN']], text=True)
    bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    def start():
        child = subprocess.Popen([sys.executable, __file__, '--serve'], env=env, stdout=subprocess.PIPE, text=True)
        assert child.stdout.readline().strip() == 'ready'
        return child
    def wait(label):
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            while GLib.MainContext.default().pending(): GLib.MainContext.default().iteration(False)
            queue = [a for a in pyatspi.Registry.getDesktop(0) if a.name == 'uflinux-desktop']
            while queue:
                node = queue.pop(0)
                if node.name == label and node.getRoleName() in ('button', 'push button'): return
                try: queue.extend([child for child in node if child])
                except Exception: pass
            time.sleep(.05)
        raise AssertionError('Desktop status did not become ' + label)
    def stream(name):
        for app in pyatspi.Registry.getDesktop(0):
            if app.name != 'uflinux-desktop': continue
            queue = [app]
            while queue:
                node = queue.pop(0)
                if node.name == name and node.getRoleName() == 'slider': return node
                try: queue.extend([child for child in node if child])
                except Exception: pass
        return None
    def wait_stream(name):
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            result = stream(name)
            if result: return result
            time.sleep(.05)
        raise AssertionError('Missing audio stream ' + name)
    child = start()
    try:
        wait('Networking disabled')
        bus.call_sync(NAME, PATH, NAME, 'SetOffline', GLib.Variant('(b)', (False,)), None, Gio.DBusCallFlags.NONE, 5000, None)
        wait('Disconnected')
        subprocess.run([env['UFLINUX_DESKTOP_BIN'], '--panel', 'sound', '--output', 'HEADLESS-1'], check=True, timeout=5)
        bus.call_sync(NAME, PATH, NAME, 'SetStreams', GLib.Variant('(i)', (1,)), None, Gio.DBusCallFlags.NONE, 5000, None)
        first = wait_stream('Fixture stream 1')
        bus.call_sync(NAME, PATH, NAME, 'SetStreams', GLib.Variant('(i)', (2,)), None, Gio.DBusCallFlags.NONE, 5000, None)
        wait_stream('Fixture stream 2')
        assert not first.getState().contains(pyatspi.STATE_DEFUNCT), 'Existing stream widget was rebuilt'
        bus.call_sync(NAME, PATH, NAME, 'SetStreams', GLib.Variant('(i)', (1,)), None, Gio.DBusCallFlags.NONE, 5000, None)
        deadline = time.monotonic() + 10
        while stream('Fixture stream 2') and time.monotonic() < deadline: time.sleep(.05)
        assert not stream('Fixture stream 2'), 'Removed stream survived'
        assert not first.getState().contains(pyatspi.STATE_DEFUNCT), 'Existing stream widget was rebuilt on removal'
        subprocess.run([env['UFLINUX_DESKTOP_BIN'], '--start-menu', '--output', 'HEADLESS-1'], check=True, timeout=5)
        bus.call_sync(NAME, PATH, NAME, 'RequestAuth', None, None, Gio.DBusCallFlags.NONE, 5000, None)
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            status = json.loads(subprocess.check_output([env['UFLINUX_DESKTOP_BIN'], '--status'], text=True, timeout=5))
            if status['popup'] == 'network': break
            time.sleep(.05)
        else: raise AssertionError('Authentication did not replace the Start menu')
        subprocess.run([env['UFLINUX_DESKTOP_BIN'], '--hide'], check=True, timeout=5)
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            reply = bus.call_sync(NAME, PATH, NAME, 'GetState', None, None, Gio.DBusCallFlags.NONE, 5000, None)
            if json.loads(reply.unpack()[0])['network']['authentication'] is None: break
            time.sleep(.05)
        else: raise AssertionError('Closing authentication did not cancel the request')
        child.terminate(); child.wait(timeout=5)
        wait('Network unavailable')
        child = start()
        wait('Networking disabled')
    finally:
        if child.poll() is None: child.terminate()
        child.wait(timeout=5)
    print('Desktop Settings state updates, authentication, and service restart passed')


if __name__ == '__main__':
    if len(sys.argv) > 1 and sys.argv[1] == '--serve': serve()
    else: run()
