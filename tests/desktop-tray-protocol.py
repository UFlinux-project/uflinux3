#!/usr/bin/env python3
"""Exercise a real StatusNotifier item against an experimental desktop session."""
import os
from pathlib import Path
import re
import subprocess
import sys
import time

from gi.repository import Gio, GLib

NAME = 'UFLinux Tray Fixture'
CHEVRON = re.compile(r'^Show \d+ hidden icons?$')
PATH = '/StatusNotifierItem'
WATCHER = 'org.kde.StatusNotifierWatcher'
WATCHER_PATH = '/StatusNotifierWatcher'
ITEM = 'org.kde.StatusNotifierItem'
XML = '''<node><interface name="org.kde.StatusNotifierItem">
<method name="Activate"><arg type="i" direction="in"/><arg type="i" direction="in"/></method>
<method name="ContextMenu"><arg type="i" direction="in"/><arg type="i" direction="in"/></method>
<property name="Title" type="s" access="read"/><property name="Status" type="s" access="read"/>
<property name="IconName" type="s" access="read"/><property name="ItemIsMenu" type="b" access="read"/>
<property name="Menu" type="o" access="read"/>
<signal name="NewIcon"/>
</interface></node>'''
MENU_XML = '''<node><interface name="com.canonical.dbusmenu">
<method name="AboutToShow"><arg type="i" direction="in"/><arg type="b" direction="out"/></method>
<method name="GetLayout"><arg type="i" direction="in"/><arg type="i" direction="in"/><arg type="as" direction="in"/><arg type="u" direction="out"/><arg type="(ia{sv}av)" direction="out"/></method>
<method name="Event"><arg type="i" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="in"/><arg type="u" direction="in"/></method>
<method name="TriggerUpdate"/><signal name="LayoutUpdated"><arg type="u"/><arg type="i"/></signal>
</interface></node>'''
WATCHER_XML = '''<node><interface name="org.kde.StatusNotifierWatcher">
<method name="RegisterStatusNotifierItem"><arg type="s" direction="in"/></method>
<method name="RegisterStatusNotifierHost"><arg type="s" direction="in"/></method>
<property name="RegisteredStatusNotifierItems" type="as" access="read"/>
<property name="IsStatusNotifierHostRegistered" type="b" access="read"/>
<property name="ProtocolVersion" type="i" access="read"/>
<signal name="StatusNotifierItemRegistered"><arg type="s"/></signal>
<signal name="StatusNotifierItemUnregistered"><arg type="s"/></signal>
</interface></node>'''


def watcher():
    bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    items = {}
    info = Gio.DBusNodeInfo.new_for_xml(WATCHER_XML).interfaces[0]
    def method(_bus, sender, _path, _interface, name, params, invocation):
        if name == 'RegisterStatusNotifierItem':
            requested = params.unpack()[0]
            key = sender + requested if requested.startswith('/') else requested + PATH
            items[key] = sender
            bus.emit_signal(None, WATCHER_PATH, WATCHER, 'StatusNotifierItemRegistered', GLib.Variant('(s)', (key,)))
        invocation.return_value(None)
    def prop(_bus, _sender, _path, _interface, name):
        values = {'RegisteredStatusNotifierItems': GLib.Variant('as', list(items)),
                  'IsStatusNotifierHostRegistered': GLib.Variant('b', True),
                  'ProtocolVersion': GLib.Variant('i', 0)}
        return values[name]
    def owner_changed(_bus, _sender, _path, _interface, _signal, params):
        name, old, new = params.unpack()
        if old and not new:
            for key, owner in list(items.items()):
                if owner == name:
                    del items[key]
                    bus.emit_signal(None, WATCHER_PATH, WATCHER, 'StatusNotifierItemUnregistered', GLib.Variant('(s)', (key,)))
    bus.register_object(WATCHER_PATH, info, method, prop, None)
    bus.signal_subscribe('org.freedesktop.DBus', 'org.freedesktop.DBus', 'NameOwnerChanged',
                         '/org/freedesktop/DBus', None, Gio.DBusSignalFlags.NONE, owner_changed)
    result = bus.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus',
                           'RequestName', GLib.Variant('(su)', (WATCHER, 4)), None,
                           Gio.DBusCallFlags.NONE, 5000, None)
    assert result.unpack()[0] == 1
    print('ready', flush=True)
    GLib.MainLoop().run()


def item(marker, stalled=False):
    bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    info = Gio.DBusNodeInfo.new_for_xml(XML).interfaces[0]
    def method(_connection, _sender, _path, _iface, name, _parameters, invocation):
        marker.write_text(name)
        invocation.return_value(None)
    def prop(_connection, _sender, _path, _iface, name):
        if stalled:
            time.sleep(10)
        values = {'Title': GLib.Variant('s', NAME), 'Status': GLib.Variant('s', 'Active'),
                  'IconName': GLib.Variant('s', 'folder'), 'ItemIsMenu': GLib.Variant('b', True), 'Menu': GLib.Variant('o', '/Menu')}
        return values[name]
    bus.register_object(PATH, info, method, prop, None)
    menu_info = Gio.DBusNodeInfo.new_for_xml(MENU_XML).interfaces[0]
    label = ['Fixture action']
    def menu_method(_connection, _sender, _path, _iface, name, parameters, invocation):
        if name == 'AboutToShow': invocation.return_value(GLib.Variant('(b)', (False,)))
        elif name == 'GetLayout':
            child = GLib.Variant('(ia{sv}av)', (42, {'label': GLib.Variant('s', label[0])}, []))
            invocation.return_value(GLib.Variant('(u(ia{sv}av))', (1, (0, {}, [child]))))
        elif name == 'TriggerUpdate':
            label[0] = 'Updated action'
            invocation.return_value(None)
            bus.emit_signal(None, '/Menu', 'com.canonical.dbusmenu', 'LayoutUpdated', GLib.Variant('(ui)', (2, 0)))
        elif name == 'Event':
            marker.write_text(str(parameters.unpack()[0]))
            invocation.return_value(None)
    bus.register_object('/Menu', menu_info, menu_method, None, None)
    bus.call_sync(WATCHER, WATCHER_PATH, WATCHER, 'RegisterStatusNotifierItem',
                  GLib.Variant('(s)', (PATH,)), None, Gio.DBusCallFlags.NONE, 5000, None)
    GLib.MainLoop().run()


def registered(bus):
    value = bus.call_sync(WATCHER, WATCHER_PATH, 'org.freedesktop.DBus.Properties', 'Get',
                          GLib.Variant('(ss)', (WATCHER, 'RegisteredStatusNotifierItems')),
                          None, Gio.DBusCallFlags.NONE, 5000, None)
    return value.unpack()[0]


def pump():
    while GLib.MainContext.default().pending():
        GLib.MainContext.default().iteration(False)


def nodes():
    """Every node in the desktop shell's accessibility tree, breadth first."""
    import pyatspi
    queue = [a for a in pyatspi.Registry.getDesktop(0) if a.name == 'uflinux-desktop']
    while queue:
        node = queue.pop(0)
        yield node
        try:
            queue.extend(child for child in node if child)
        except Exception:
            pass


def find(match, roles=('button', 'push button', 'menu item')):
    """First mapped node the shell exposes, so a closed flyout never counts."""
    import pyatspi
    pump()
    for node in nodes():
        if not match(node) or node.getRoleName() not in roles:
            continue
        try:
            if node.getState().contains(pyatspi.STATE_SHOWING):
                return node
        except Exception:
            continue
    return None


def chevron():
    return find(lambda node: CHEVRON.match(node.name))


def panel_popup():
    binary = os.environ.get('UFLINUX_DESKTOP_BIN')
    if not binary:
        return None
    status = subprocess.run([binary, '--status'], capture_output=True, text=True, timeout=10)
    import json
    return json.loads(status.stdout).get('popup')


def run(marker):
    import pyatspi
    bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    # Tray icons live inside the overflow flyout, so with no item registered the
    # chevron itself is gone and there is nothing to open.
    if not registered(bus):
        assert chevron() is None, 'Tray overflow shown without any item'
    stalled = None
    if os.environ.get('UFLINUX_TEST_STALLED_TRAY') == '1':
        stalled = subprocess.Popen([sys.executable, __file__, '--stalled-item', str(marker)])
        deadline = time.monotonic() + 10
        while not registered(bus) and time.monotonic() < deadline:
            assert stalled.poll() is None, 'Stalled tray fixture exited'
            time.sleep(.05)
        assert registered(bus), 'Stalled tray fixture did not register'
    previous = set(registered(bus))
    child = subprocess.Popen([sys.executable, __file__, '--item', str(marker)])
    try:
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            button = chevron()
            if button is not None:
                assert button.queryAction().doAction(0)
                break
            if child.poll() is not None: raise AssertionError('Tray fixture exited')
            time.sleep(.05)
        else: raise AssertionError('Tray overflow did not appear')
        assert CHEVRON.match(button.name), button.name
        assert panel_popup() in (None, 'tray'), 'Overflow did not report itself as the open popup'
        while time.monotonic() < deadline:
            node = find(lambda n: n.name == NAME)
            if node is not None:
                assert node.queryAction().doAction(0)
                break
            if child.poll() is not None: raise AssertionError('Tray fixture exited')
            time.sleep(.05)
        else: raise AssertionError('Tray item did not appear')
        items = registered(bus)
        added = set(items) - previous
        assert len(added) == 1, items
        service = added.pop().split('/')[0]
        bus.call_sync(service, '/Menu', 'com.canonical.dbusmenu', 'TriggerUpdate', None, None, Gio.DBusCallFlags.NONE, 5000, None)
        while time.monotonic() < deadline:
            node = find(lambda n: n.name == 'Updated action')
            if node is not None:
                assert node.queryAction().doAction(0)
                break
            time.sleep(.05)
        else: raise AssertionError('DBusMenu action did not appear')
        while not marker.exists() and time.monotonic() < deadline: time.sleep(.05)
        assert marker.read_text() == '42', 'DBusMenu action was not delivered'
        assert any(value.endswith(PATH) for value in registered(bus)), registered(bus)
    finally:
        child.terminate()
        child.wait(timeout=5)
        if stalled:
            stalled.terminate()
            stalled.wait(timeout=5)
    deadline = time.monotonic() + 5
    while registered(bus) and time.monotonic() < deadline: time.sleep(.05)
    assert not registered(bus), 'Tray item survived its service disappearing'
    assert chevron() is None, 'Tray overflow outlived its last item'
    print('StatusNotifier registration, live DBusMenu update, activation, and disconnect passed')


if __name__ == '__main__':
    marker = Path(sys.argv[2] if len(sys.argv) > 2 else os.environ['XDG_RUNTIME_DIR'] + '/tray-action')
    if len(sys.argv) > 1 and sys.argv[1] == '--watcher': watcher()
    elif len(sys.argv) > 1 and sys.argv[1] == '--item': item(marker)
    elif len(sys.argv) > 1 and sys.argv[1] == '--stalled-item': item(marker, stalled=True)
    else: run(marker)
