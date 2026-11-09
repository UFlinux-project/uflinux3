# Start menu

Click the leaf on the bottom taskbar or release either Super key to toggle the
launcher. Super combinations still work; Super+R and Super+Space open Fuzzel.
The Start menu runs in its own plain GTK4 process, `uflinux-start-menu`, with
Papirus icons and the selected GTK theme. Login starts it hidden with its default
widgets, layout and application catalog prepared. It retains these between
openings; GTK and the compositor draw buffers when the window is mapped.

The left column contains Files, Terminal, Settings and the default web browser
when installed. All Applications opens the alphabetical catalog. Search matches
application names, generic names, descriptions and keywords, with exact names
ranked first. Hidden desktop entries and entries unavailable in the current
desktop are excluded. Applications launch through GIO desktop-entry handling;
search text is never interpreted as a command.

The right column contains the account name, Home and configured XDG folders,
Settings and Terminal. Unavailable folders have an explanation. `xdg-user-dirs`
initializes standard folders during login and respects existing configuration.
Favorites are fixed; there is no tracking of recent files or applications.

Log out, Restart and Shut down require confirmation with Cancel selected by default.
Suspend is immediate. Unavailable elogind actions are disabled. Authorization
errors appear inside the menu. Power-button policy remains in Settings.

Search receives focus on opening. Arrow keys navigate applications and Enter
launches the selection. Escape first cancels confirmation, then clears search,
returns from All Applications and finally closes the menu. Outside clicks close
it immediately. Start and quick settings replace each other while a full Settings
window can remain open.

`uflinux-start-menu [--output NAME]` opens or toggles the menu on the
specified monitor. With no monitor, labwc selects the keyboard activation output.
Ironbar supplies its originating monitor in `IRONBAR_OUTPUT`. The transparent
layer captures outside clicks and the Start button while leaving other taskbar
controls usable. Output removal closes the menu. The card adapts to smaller
outputs and compositor scaling.

The native desktop harness exercises real GTK controls and GIO fixtures in a
private labwc session. Tests cover hidden startup, repeated toggles, independent
process lifetime, memory and idle CPU. Its login1 fixture records and denies
power requests; shutdown and reboot must only be exercised inside disposable VMs. Installed VM
tests also measure launcher memory, repeated opening and closing, and process
exit. Power-profile synchronization is tested through Settings and Ironbar. See the [validation report](start-menu-validation.md) for
measured results and architecture coverage.

The launcher does not connect to the Settings state service or register a polkit
agent at startup. Settings shortcuts launch `uflinux-settings`; session actions
use login1 and initialize authorization only when requested. Start and quick
settings dismiss each other through D-Bus actions without starting the other app.
`uflinux-settings --start-menu` remains a compatibility alias for user bindings.

`uflinux-start-menu --daemon` starts or ensures the hidden resident process.
`--hide` dismisses it, `--status` reports readiness and visibility, and `--quit`
ends it explicitly. Closing the menu resets search, selection and confirmations;
it does not destroy the catalog or widgets. Desktop-file changes refresh the
catalog while hidden, and monitor removal dismisses the menu. Resident memory
is part of the desktop's background cost; the old nonresident measurements in
the validation record do not establish this version's memory or latency.
