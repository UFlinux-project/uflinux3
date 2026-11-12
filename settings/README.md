# UFLinux Settings

Plain GTK4 and Rust; no libadwaita, webview or GNOME settings daemon. The GUI exits
when its last window closes. A separate GLib/GIO service maintains user-session
state, ConnMan authentication and display rollback. The privileged system helper
uses polkit and normally exits after 45 idle seconds; saved additional routes keep
its ConnMan subscription alive. Authentication uses libpolkit-agent-1 and PAM.

Open `uflinux-settings`, use `--page network|displays|audio|power|default-apps|datetime|appearance|screen-capture|about`,
or `--panel network|sound|power --output DP-1` for the taskbar menus.
`--quick-settings` remains an alias for the Network panel. The separate resident `uflinux-start-menu
[--output DP-1]` toggles the ArcMenu-inspired launcher. It prepares its fixed
favorites and application search while hidden at login. The old Settings
`--start-menu` option forwards to this launcher for compatibility.
Either Super key toggles Start; Super+I opens Settings. CLI hotkeys use
`uflinux-settingsctl volume 5`, `brightness -5`, and `mute`. `status` prints JSON.
`request` reads one typed JSON request from standard input; passwords must never
be put in command arguments. Types and validation live in `core/src/model.rs`.

Session D-Bus: `org.uflinux.Settings1`, object `/org/uflinux/Settings`, `GetState`
and `Execute`, with a `Changed` signal. JSON payloads are constrained by the shared
Rust enums; unknown operations and oversized requests are rejected. System D-Bus
uses `org.uflinux.Settings.System1`, object `/org/uflinux/Settings/System` and an
independently authorized `Execute` method. These are version-1 project interfaces.

ConnMan owns IP addresses, DHCP, DNS, connection policy and time synchronization
through its built-in SNTP client. iwd only handles Wi-Fi
authentication. Settings may switch synchronization on and off through ConnMan's
clock API, which ordinary users may not mutate; only the root helper reaches it,
after polkit. Timezone remains manual in ConnMan, because Settings owns it.
Enterprise profiles require a CA and server domain. Saved network
credentials are machine-wide in root-protected ConnMan/iwd storage.

TLP provides Power saver, Balanced and Performance modes, with a persistent
selection. Alpine's TLP 1.10 package includes the profile commands introduced in
1.9; no tlp-pd or power-profiles-daemon runs alongside this service. Profiles are
available in Settings and the Ironbar Power menu, or through `uflinux-settingsctl power-profile
balanced`. Physical power-button behavior is configured only in Settings, with
separate AC/battery choices. A small OpenRC service explicitly runs BusyBox acpid;
elogind ignores the physical power key and performs the requested power action.
Only KEY_POWER presses are mapped; releases, repeats and other events are ignored.
Shutdown/sleep inhibitors are checked and unsupported suspend is rejected.
`uflinux-settingsctl system-request` accepts a typed SystemAction on stdin for
console administration; the system helper still enforces polkit authorization.

Display changes require confirmation within 15 seconds. HDR is a global request
to labwc, not a claim of per-monitor activation. Mirroring is unavailable on this
compositor. Brightness requires a kernel backlight or a responding DDC/CI display.
Additional routes use protocol 242 and only recorded routes are removed. Wallpapers
are local files with a 16-megapixel/32-MiB limit; previews are downscaled.

`screencapture/` is the Print Screen tool shipped as the `uflinux-screencapture` package.
It starts on demand and exits when the capture is done; nothing runs permanently. It reads
its preferences from the Settings service over session D-Bus, so the Screen Capture page
is the only editor, and it captures through the `gpu-screen-recorder` backend, using the
desktop portal for window capture. `uflinux-screencapture [--screenshot|--record]
[--target screen|window|area] [--delay SECONDS] [--silent]` bypasses the chooser;
screenshots go to the configured folder and the clipboard.

Build with `cargo build --workspace --release --locked`; packaging uses frozen
resolution and cargo auditable. Run `cargo test --workspace --locked`. Generate
the verified distribution source with `python3 scripts/settings-source.py --update`
from the repository root after editing sources. Hardware and memory claims require
the integration/VM tests; a successful compile is not hardware certification.

The application icon reuses the original GPL-3.0 UFLinux leaf logo, unchanged,
from UFlinux-project/uf-calamares-settings commit
53217a0b7edc494658a1dfcfee649365a19d0c79,
etc/calamares/branding/uflinux/uflinuxlogo-spaces.svg. Source attribution and exact
checksums are recorded in config/artwork.lock.json in the distro repository.
