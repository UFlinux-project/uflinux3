# UFLinux Settings

Settings is a Rust workspace using plain GTK4, application-local CSS and the
user's GTK theme. It does not depend on libadwaita, GNOME Settings or GNOME
Settings Daemon. Super+I opens the eight-page sidebar interface. Dynamic network
lists use GtkListView; pages are constructed on demand and discarded when left.
Image previews are bounded; arbitrary remote wallpapers are not downloaded.

```sh
uflinux-settings --page network
uflinux-settings --quick-settings --output DP-1
uflinux-start-menu --output DP-1
uflinux-settingsctl status
uflinux-settingsctl volume -5
uflinux-settingsctl brightness 5
uflinux-settingsctl mute
uflinux-settingsctl power-profile balanced
```

Other page names are `displays`, `audio`, `power`, `default-apps`, `datetime`, `appearance`
and `about`. The quick panel provides Wi-Fi selection and credentials, network
and radio power, output/input devices, volume, microphone mute and available
brightness controls. Escape, focus loss or a repeated taskbar click dismisses it.
Ironbar supplies the originating monitor through a patched child environment.
The GUI reuses its process and exits after the last UI closes. There are no
periodically spawned shell commands polling the panel's state.

## Services and authorization

`uflinux-settingsd` is a headless GLib/GIO session service. A sleeping Wayland
worker handles output protocols; libpulse integrates asynchronously with GLib.
ConnMan and Pulse subscriptions update a shared snapshot and Ironbar reactive
variables. Audio reconnects after server restarts. Clock state is rechecked every
15 seconds and published when synchronization or policy changes.

Session D-Bus interface `org.uflinux.Settings1` at `/org/uflinux/Settings` exposes
`GetState`, typed `Execute` requests and `Changed` subscriptions. Version 1 JSON
types are defined in `settings/core/src/model.rs`; unknown fields and requests
larger than 256 KiB are rejected. This is a project interface, not a general
command or D-Bus forwarding API. `uflinux-settingsctl request` reads its request
from standard input. Never pass credentials as command arguments.

The system helper is a separate root D-Bus service, authorized per typed operation
with polkit. Its allowed actions are clocks/timezones, NTP, protected Wi-Fi
profiles, exact additional routes and identity-checked DDC brightness. It accepts
no shell commands or arbitrary paths. It normally exits after 45 idle seconds;
saved routes keep its ConnMan subscription alive. Only the open GTK4 UI registers
a libpolkit-agent-1/PAM authentication agent. Wheel administrators authenticate
with their own password; the root account remains locked. Backlights use elogind.
Authorization without an active local session can be denied; console `doas` is
available for administration.

## Networking and clocks

The small ConnMan 2.0-r1 package retains Alpine’s musl/D-Bus fixes and adds a
checksum-pinned iwd credential-delegation patch. ConnMan 2.0 otherwise rejects
iwd-managed enterprise connections when no duplicate ConnMan EAP configuration
exists. Only the iwd driver enables this internal flag; iwd still enforces its
CA and server-name checks. No immutable provisioning workaround or duplicate
enterprise password store is introduced. Legacy supplicant, VPN, modem and
Bluetooth plugins are disabled in this core-networking build.

ConnMan controls Ethernet/Wi-Fi connections, DHCP, IPv4/IPv6, DNS, proxies and
autoconnect. Its iwd plugin is selected explicitly; wpa_supplicant and
NetworkManager are absent from the desktop dependency set. iwd's independent IP
configuration is disabled. Do not enable networking, dhcpcd or another resolver
manager alongside ConnMan. BusyBox-backed boot services are retained.

Network settings provide scans, radio-block reporting, personal and hidden
networks, saved connections, reconnect/disconnect/forget, addressing, DNS/search
domains and ConnMan proxy controls. Additional routes use protocol 242 and a
root-protected per-connection database. Updates are serialized with restoration,
rolled back on failure and remove only exact UFLinux-owned routes. They never
flush routing tables. Current IPv4 and IPv6 routes can be inspected in the UI.

Enterprise Wi-Fi supports EAP-TLS, PEAP/MSCHAPv2 and TTLS/PAP. Supply a valid CA,
expected server domain and the appropriate identity/password or client
certificate/private key. A protected iwd profile performs authentication while
ConnMan remains the connection controller. Profiles and imported certificates
are machine-wide and root-readable; secrets are excluded from shared snapshots
and preferences. Forgetting a connection removes its owned certificate files.
VPNs, hotspots, tethering, virtual interfaces and Bluetooth are outside this UI.

BusyBox ntpd is the only synchronizer. Before ConnMan starts, its persisted
TimeUpdates and TimezoneUpdates policies are initialized to manual; its clock
mutation API is restricted. NTP runs in client mode. Settings provides timezone
search, server selection and manual time only while NTP is disabled. The RTC
uses UTC. Synchronization comes from the kernel's actual state, not a successful
DNS lookup or a running daemon.

## Power modes and the physical power button

The three modes are **Power saver**, **Balanced** and **Performance**, using TLP's
profile commands introduced in 1.9. Alpine 3.24 supplies TLP 1.10.1; this integration
tracks the 1.10 series and its observed runtime state. Balanced is the default.
The selection persists across reboot and charger changes using
`/etc/tlp.d/90-uflinux-profile.conf`; unsupported hardware tunables are left to TLP.
An active profile is reported only when TLP's runtime record confirms it. The
[TLP profile documentation](https://linrunner.de/tlp/usage/tlp.html) describes the
underlying commands and their hardware-dependent effects.

Select a profile in Settings → Power. Its current-mode label and selection
markers update through the shared backend and IPC. There is no resident tlp-pd, power-profiles-daemon or radio-device
wizard. TLP's OpenRC startup and udev hooks apply the chosen policy without a
new persistent scripting process. Upstream TLP requires Perl and grep for its
short-lived operations; they do not form an additional background service.

**Physical power-button behavior is configurable only in Settings → Power**.
Choose Shut down, Suspend or Do nothing independently for AC and battery power.
Changes require administrator authorization. Suspend is rejected when unavailable.
The default short-press action is Shut down; firmware-controlled long presses
cannot be changed by the app. Ordinary suspend/restart/shutdown actions remain
available in Start and do not edit the physical-button policy.

`uflinux-acpid` explicitly invokes `/bin/busybox acpid`, using the same OpenRC
pattern as Alpine's BusyBox service. Its private mapping accepts KEY_POWER press
events only, with a bounded, debounced root hook. The hook reads the root-owned
typed policy in `/etc/uflinux/power.json`, checks shutdown/sleep inhibitors and asks
elogind to perform the action. Elogind's `HandlePowerKey=ignore` drop-in prevents
duplicate physical-key handling. Other keys and lid policy retain elogind defaults.
External acpid and power-profile daemons conflict with this package's ownership.
The BusyBox listener discovers event devices at startup; restart its service after
changing physical input-device topology if a newly added power key is not observed.

The helper accepts only typed profile/button operations. CLI console automation
can submit a `SystemAction` through `uflinux-settingsctl system-request` on stdin;
polkit authorization still applies. Administrator overrides in `/etc/tlp.conf`
take precedence over drop-in defaults and may override the saved boot selection;
the UI continues to show the actual reported profile.

## Displays and other panels

Display settings use the compositor's output-management protocol for advertised
resolution/refresh modes, fractional scale, transforms, placement and enablement.
The compositor tests a configuration before applying it. The session service
requires confirmation within 15 seconds and restores the previous layout after
timeout, caller loss or output loss. It rejects disabling the final display.
Confirmed layouts are saved by display identity and restored at login/hotplug.
Missing outputs, duplicate identities, unavailable modes and invalid saved layouts
recover to a usable preferred-mode display. A rejected saved combination receives
one bounded fallback attempt. Compositor rejection is reported as an error.

VRR controls require protocol support and still need compatible physical hardware.
HDR is labwc's global “Use HDR where supported” request; it is not a per-monitor
activation report. No mirroring is simulated by overlapping displays. These
limitations follow the packaged compositor's capabilities. See the
[labwc configuration manual](https://labwc.github.io/labwc-config.5).

The session wrapper probes hardware Vulkan, then hardware OpenGL, then pixman.
It skips software Vulkan, respects explicit renderer overrides and limits startup
fallback attempts. A normal logout or a crash after desktop startup does not
restart the session with another renderer. On software rendering, GTK uses Cairo
unless explicitly overridden. DDC uses bounded asynchronous ddcutil VCP 0x10
operations against a connected DRM/EDID identity; unsupported monitors show an
unavailable state. Backlight values and device hotplug refresh through filtered
eudev events; display changes and resume coalesce DDC rescans. DDC hardware-button
changes need a rescan because the protocol supplies no push event. No hardware
response is fabricated.

Audio controls use the PulseAudio client protocol against **pipewire-pulse**;
PipeWire and WirePlumber remain the audio stack, without a PulseAudio server.
Devices, profiles, ports, defaults, volume, balance, mute and application streams
share state with the flyout. Default applications use GIO/XDG associations and
preserve unrelated mappings. Wallpapers use only UFLinux-owned swaybg children,
local files up to 32 MiB/16 megapixels, fit/color and per-output preferences.
About provides copyable information and tolerates unavailable hardware data.
User preferences live under `$XDG_CONFIG_HOME/uflinux` (normally `~/.config/uflinux`)
and personal labwc/GTK/XDG files; APK upgrades preserve user files.

## Resource and validation scope

The reference target is an **installed x86_64 system with 512 MiB, one display and
no swap**. Live images load an APK-based root into RAM and need more memory.
High resolutions, physical GPU drivers and other architectures need separate
measurements. Targets are ≤16 MiB PSS for background Settings components,
≤80 MiB for the full UI and ≤50 MiB for the flyout; cold usable frame ≤1.5 seconds
and warm ≤500 ms are native reference targets, not emulated timing guarantees.

Headless tests exercise real labwc output management, rollback and caller
isolation, PipeWire controls, UI pages and process lifetime. A separate D-Bus
fixture tests ConnMan credential/cancellation and property transitions; it does
not certify wireless association. The disposable x86_64 VM adds mac80211_hwsim,
hostapd personal/hidden/enterprise tests, DHCP/route restoration, BusyBox NTP and
the installed memory acceptance test. Test-only hostapd/dnsmasq packages are not
desktop dependencies. Missing test facilities fail the gate rather than silently
claiming coverage. Physical HDR, VRR, DDC/CI, suspend and GPU support remain
separate hardware validation. See [recorded results](settings-validation.md).

The resident [Start menu](start-menu.md) runs in a separate GTK4 process and
launches Settings when requested. See its [validation record](start-menu-validation.md) for launcher tests
and measurements.
