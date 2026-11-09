#!/usr/bin/env python3
"""Private test-bus power API: records and denies requests, never touches host power."""
import json
from pathlib import Path
import signal
from gi.repository import Gio, GLib

NAME = 'org.freedesktop.login1'
PATH = '/org/freedesktop/login1'
log = Path('/tmp/uflinux-login1-actions.log')
log.write_text('')
xml = '<node><interface name="org.freedesktop.login1.Manager">' + ''.join(
    '<method name="Can'+action+'"><arg type="s" direction="out"/></method>'
    '<method name="'+action+'"><arg type="b" direction="in"/></method>'
    for action in ('Suspend','Reboot','PowerOff')) + '</interface></node>'
bus = Gio.bus_get_sync(Gio.BusType.SYSTEM, None)
bus.call_sync('org.freedesktop.DBus','/org/freedesktop/DBus','org.freedesktop.DBus','RequestName',
              GLib.Variant('(su)',(NAME,4)), None, 0, 5000, None)
def method(connection, sender, path, interface, name, parameters, invocation):
    if name.startswith('Can'):
        invocation.return_value(GLib.Variant('(s)',('yes',)))
    else:
        with log.open('a') as stream:
            stream.write(json.dumps({'method':name,'interactive':parameters.unpack()[0]})+'\n')
        invocation.return_dbus_error('org.freedesktop.DBus.Error.AccessDenied','Fixture denied power operation')
info = Gio.DBusNodeInfo.new_for_xml(xml)
bus.register_object(PATH, info.interfaces[0], method, None, None)
loop = GLib.MainLoop()
for sig in (signal.SIGTERM, signal.SIGINT): signal.signal(sig, lambda *_: loop.quit())
loop.run()
