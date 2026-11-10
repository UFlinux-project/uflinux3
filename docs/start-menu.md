# Start menu

Click the leaf on the bottom taskbar or release either Super key to toggle the
launcher. Super combinations still work; Super+R and Super+Space also toggle the Start menu.
The Start menu runs in the resident GTK4 `uflinux-desktop` process, with
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
it immediately. Start and the Network, Sound and Power menus replace each other while a full Settings
window can remain open.

`uflinux-desktop --start-menu [--output NAME]` opens or toggles the menu on the
specified monitor. With no monitor, labwc selects the keyboard activation output.
Panel buttons supply their originating monitor. The transparent
layer captures outside clicks and the Start button while leaving other taskbar
controls usable. Output removal closes the menu. The card adapts to smaller
outputs and compositor scaling.

The desktop connects to the Settings state service and provides the session
polkit agent. Settings shortcuts open `uflinux-settings` in a separate process.
The native desktop harness exercises GTK controls, state and tray fixtures in a
private labwc session. See the [validation report](desktop-prototype-validation.md)
for measurements and remaining release checks.

`uflinux-desktop --daemon` starts or ensures the hidden resident process.
`--hide` dismisses it, `--status` reports readiness and visibility, and `--quit`
ends it explicitly. Closing the menu resets search, selection and confirmations;
it does not destroy the catalog or widgets. Desktop-file changes refresh the
catalog while hidden, and monitor removal dismisses the menu. Resident memory
is part of the desktop's background cost; the old nonresident measurements in
the validation record do not establish this version's memory or latency.

The taskbar has separate Network, Sound and Power buttons with Papirus symbolic
icons. They track connection, output volume and battery state. Sound includes
input/output and application controls; Power includes battery information,
brightness and power profiles. The corresponding Settings page provides advanced
controls. Open a panel directly with `uflinux-desktop --panel network|sound|power
--output NAME`. The settings service reads system batteries and updates the panel
after device changes. Desktop systems use a bolt icon.
