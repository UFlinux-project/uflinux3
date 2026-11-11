#!/usr/bin/env python3
"""D-Bus protocol fixture. It never configures host interfaces or routes."""
import signal
from gi.repository import Gio, GLib

NAME = "net.connman"
SERVICE = "/net/connman/service/wifi_000000000001_55464c696e75782054657374_managed_psk"
TECH = "/net/connman/technology/wifi"
XML = """<node>
<interface name="net.connman.Manager">
 <method name="GetProperties"><arg type="a{sv}" direction="out"/></method>
 <method name="GetTechnologies"><arg type="a(oa{sv})" direction="out"/></method>
 <method name="GetServices"><arg type="a(oa{sv})" direction="out"/></method>
 <method name="RegisterAgent"><arg type="o" direction="in"/></method>
 <method name="SetProperty"><arg type="s" direction="in"/><arg type="v" direction="in"/></method>
 <signal name="ServicesChanged"><arg type="a(oa{sv})"/><arg type="ao"/></signal>
 <signal name="PropertyChanged"><arg type="s"/><arg type="v"/></signal>
</interface>
<interface name="net.connman.Technology">
 <method name="SetProperty"><arg type="s" direction="in"/><arg type="v" direction="in"/></method>
 <method name="Scan"/>
 <signal name="PropertyChanged"><arg type="s"/><arg type="v"/></signal>
</interface>
<interface name="net.connman.Service">
 <method name="Connect"/><method name="Disconnect"/><method name="Remove"/>
 <method name="SetProperty"><arg type="s" direction="in"/><arg type="v" direction="in"/></method>
 <signal name="PropertyChanged"><arg type="s"/><arg type="v"/></signal>
</interface>
<interface name="net.connman.Clock">
 <method name="GetProperties"><arg type="a{sv}" direction="out"/></method>
 <method name="SetProperty"><arg type="s" direction="in"/><arg type="v" direction="in"/></method>
 <signal name="PropertyChanged"><arg type="s"/><arg type="v"/></signal>
</interface></node>"""
bus = Gio.bus_get_sync(Gio.BusType.SYSTEM, None)
bus.call_sync("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "RequestName",
              GLib.Variant("(su)", (NAME, 4)), None, 0, 5000, None)
agent = None
powered = True
offline = False
state = "idle"
favorite = False
autoconnect = False
configuration = {}
# ConnMan persists the clock policy itself; the fixture keeps it in memory.
time_updates = "auto"
timezone_updates = "manual"
timeservers = ["clock-fixture.invalid"]


def clock_properties():
    return {"Time": GLib.Variant("u", 1700000000),
            "TimeUpdates": GLib.Variant("s", time_updates),
            "TimezoneUpdates": GLib.Variant("s", timezone_updates),
            "Timeservers": GLib.Variant("as", timeservers),
            "TimeserverSynced": GLib.Variant("b", time_updates == "auto")}


def properties():
    p = {"Name": GLib.Variant("s", "UFLinux Test"), "Type": GLib.Variant("s", "wifi"),
         "State": GLib.Variant("s", state), "Security": GLib.Variant("as", ["psk"]),
         "Strength": GLib.Variant("y", 80), "Favorite": GLib.Variant("b", favorite),
         "AutoConnect": GLib.Variant("b", autoconnect),
         "Ethernet": GLib.Variant("a{sv}", {"Interface": GLib.Variant("s", "wlan-test")}),
         "IPv4": GLib.Variant("a{sv}", {"Method": GLib.Variant("s", "dhcp"), "Address": GLib.Variant("s", "192.0.2.2")}),
         "IPv6": GLib.Variant("a{sv}", {"Method": GLib.Variant("s", "auto"), "Address": GLib.Variant("s", "2001:db8::2")}),
         "Nameservers": GLib.Variant("as", ["192.0.2.1"]),
         # Announced by DHCP: ConnMan prefers service timeservers over its fallback list.
         "Timeservers": GLib.Variant("as", ["192.0.2.1"])}
    p.update(configuration)
    return p


def changed():
    bus.emit_signal(None, "/", "net.connman.Manager", "ServicesChanged",
                    GLib.Variant("(a(oa{sv})ao)", ([(SERVICE, properties())], [])))


def method(connection, sender, path, interface, name, args, invocation):
    global agent, powered, offline, state, favorite, autoconnect
    global time_updates, timezone_updates, timeservers
    if name == "GetProperties" and interface == "net.connman.Clock":
        invocation.return_value(GLib.Variant("(a{sv})", (clock_properties(),)))
    elif name == "GetProperties":
        invocation.return_value(GLib.Variant("(a{sv})", ({"State": GLib.Variant("s", "ready" if state == "ready" else "idle"), "OfflineMode": GLib.Variant("b", offline)},)))
    elif name == "GetTechnologies":
        invocation.return_value(GLib.Variant("(a(oa{sv}))", ([(TECH, {"Type": GLib.Variant("s", "wifi"), "Powered": GLib.Variant("b", powered), "Connected": GLib.Variant("b", state == "ready")})],)))
    elif name == "GetServices":
        invocation.return_value(GLib.Variant("(a(oa{sv}))", ([(SERVICE, properties())],)))
    elif name == "RegisterAgent":
        agent = (sender, args.unpack()[0])
        invocation.return_value(None)
    elif name == "SetProperty":
        key, value = args.unpack()
        if interface == "net.connman.Clock" and key == "TimeUpdates":
            if value not in {"manual", "auto"}:
                invocation.return_dbus_error("net.connman.Error.InvalidArguments", "Unknown clock policy")
                return
            time_updates = value
        elif interface == "net.connman.Clock" and key == "TimezoneUpdates":
            if value not in {"manual", "auto"}:
                invocation.return_dbus_error("net.connman.Error.InvalidArguments", "Unknown clock policy")
                return
            timezone_updates = value
        elif interface == "net.connman.Clock" and key == "Timeservers":
            timeservers = value
        elif interface == "net.connman.Clock":
            # Real ConnMan refuses a manual clock write while synchronizing
            # automatically, and never allows TimeserverSynced to be written.
            invocation.return_dbus_error("net.connman.Error.InvalidArguments", "Fixture clock property is read-only")
            return
        elif interface == "net.connman.Manager" and key == "OfflineMode":
            offline = value
        elif interface == "net.connman.Technology" and key == "Powered":
            powered = value
        elif interface == "net.connman.Service" and key == "AutoConnect":
            autoconnect = value
        elif interface == "net.connman.Service" and key in {"IPv4.Configuration", "IPv6.Configuration", "Nameservers.Configuration", "Domains.Configuration", "Proxy.Configuration"}:
            configuration[key] = args.get_child_value(1).get_variant()
        else:
            invocation.return_dbus_error("net.connman.Error.InvalidProperty", "Unsupported fixture property")
            return
        invocation.return_value(None)
        changed()
    elif name == "Scan":
        invocation.return_value(None)
        changed()
    elif name in {"Disconnect", "Remove"}:
        state = "idle"
        if name == "Remove":
            favorite = False
        invocation.return_value(None)
        changed()
    elif name == "Connect":
        if agent is None:
            invocation.return_dbus_error("net.connman.Error.NoAgent", "No settings agent registered")
            return
        state = "association"
        changed()
        fields = {"Passphrase": GLib.Variant("a{sv}", {"Type": GLib.Variant("s", "psk"), "Requirement": GLib.Variant("s", "mandatory")})}

        def authenticated(connection, result):
            global state, favorite
            try:
                answer = connection.call_finish(result).unpack()[0]
                if answer.get("Passphrase") != "uflinux-test-secret":
                    raise ValueError("Incorrect fixture credential")
                state = "ready"
                favorite = True
                invocation.return_value(None)
            except (GLib.Error, ValueError):
                state = "idle"
                invocation.return_dbus_error("net.connman.Error.InvalidKey", "Authentication failed or canceled")
            changed()

        bus.call(agent[0], agent[1], "net.connman.Agent", "RequestInput", GLib.Variant("(oa{sv})", (SERVICE, fields)),
                 GLib.VariantType("(a{sv})"), 0, 120000, None, authenticated)
    else:
        invocation.return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod", "Unknown fixture operation")


node = Gio.DBusNodeInfo.new_for_xml(XML)
for path, interface in [("/", "net.connman.Manager"), ("/", "net.connman.Clock"),
                        (TECH, "net.connman.Technology"), (SERVICE, "net.connman.Service")]:
    bus.register_object(path, node.lookup_interface(interface), method, None, None)
loop = GLib.MainLoop()
GLib.unix_signal_add(GLib.PRIORITY_DEFAULT, signal.SIGTERM, lambda: (loop.quit(), False)[1])
loop.run()
