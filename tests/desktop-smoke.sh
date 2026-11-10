#!/bin/sh
# Runs inside the target Alpine userspace, including under qemu-user.
set -eu
if [ "${UFLINUX_TEST_BUS:-}" != 1 ]; then
	exec dbus-run-session -- env UFLINUX_TEST_BUS=1 sh -x "$0"
fi
export XDG_RUNTIME_DIR
XDG_RUNTIME_DIR=$(mktemp -d)
chmod 700 "$XDG_RUNTIME_DIR"
export XDG_CONFIG_HOME="$XDG_RUNTIME_DIR/preferences" XDG_DATA_HOME="$XDG_RUNTIME_DIR/data" XDG_CACHE_HOME="$XDG_RUNTIME_DIR/cache"
export WAYLAND_DISPLAY=wayland-0 WLR_BACKENDS=headless WLR_HEADLESS_OUTPUTS=1
export WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 GSK_RENDERER=cairo
export XDG_CURRENT_DESKTOP=labwc XDG_SESSION_TYPE=wayland
results=/tmp/uflinux-smoke-results
mkdir -p "$results" "$XDG_RUNTIME_DIR/config"
python3 - "$XDG_RUNTIME_DIR/config/rc.xml" <<'RC'
import sys,xml.etree.ElementTree as E
root=E.parse('/etc/xdg/labwc/rc.xml')
keyboard=root.getroot().find('keyboard')
for key,action in [('C-A-F10','VirtualOutputAdd'),('C-A-F11','VirtualOutputRemove')]:
    binding=E.SubElement(keyboard,'keybind',key=key)
    E.SubElement(binding,'action',name=action,output_name='UF-TEST')
binding=E.SubElement(keyboard,'keybind',key='C-A-F9')
E.SubElement(binding,'action',name='MoveToOutput',output='HEADLESS-1')
E.SubElement(binding,'action',name='MoveTo',x='0',y='0')
root.write(sys.argv[1])
RC
: > "$XDG_RUNTIME_DIR/config/autostart"
pids=""
cleanup() {
	for pid in $pids; do kill "$pid" 2>/dev/null || :; done
	# Under qemu-user the host process name is qemu-*, not the guest daemon
	# name. Restrict cleanup to this session's unique runtime directory.
	python3 - <<'PY'
import os, signal
from pathlib import Path
runtime = ('XDG_RUNTIME_DIR=' + os.environ['XDG_RUNTIME_DIR']).encode()
for process in Path('/proc').glob('[0-9]*'):
    try:
        pid = int(process.name)
        if pid in (os.getpid(), os.getppid()) or process.stat().st_uid != os.getuid():
            continue
        if runtime in (process / 'environ').read_bytes().split(b'\0'):
            os.kill(pid, signal.SIGTERM)
    except (OSError, ValueError):
        pass
PY
	wait 2>/dev/null || :
	rm -rf "$XDG_RUNTIME_DIR"
}
trap cleanup EXIT HUP INT TERM
labwc --config-dir "$XDG_RUNTIME_DIR/config" > "$results/labwc.log" 2>&1 &
compositor=$!; pids="$pids $compositor"
for _attempt in $(seq 1 60); do
	[ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ] && break
	kill -0 "$compositor"
	sleep 1
done
[ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ]
wayland-info > "$results/wayland.txt"
grep -q zwlr_foreign_toplevel_manager_v1 "$results/wayland.txt"
python3 /project/tests/artwork-smoke.py > "$results/artwork.txt"
swaybg -c '#102c46' -i /usr/share/backgrounds/uflinux/Dark_Ivy.jpg -m fill > "$results/background.log" 2>&1 &
background=$!; pids="$pids $background"
# Record Wayland setup and, when running under qemu-user, guest syscalls. This
# distinguishes a stalled compositor handshake from a D-Bus connection failure.
WAYLAND_DEBUG=1 QEMU_STRACE=1 mako > "$results/notifications.log" 2>&1 &
notifications=$!; pids="$pids $notifications"
# A running process is not necessarily registered on D-Bus yet. Calling
# notify-send too early can trigger a competing D-Bus activation of mako.
for _attempt in $(seq 1 600); do
	if gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus --method org.freedesktop.DBus.NameHasOwner org.freedesktop.Notifications | grep -q true; then break; fi
	if ! kill -0 "$notifications" 2>/dev/null; then cat "$results/notifications.log" >&2; exit 1; fi
	sleep 1
done
if ! gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus --method org.freedesktop.DBus.NameHasOwner org.freedesktop.Notifications | grep -q true; then
	for entry in status wchan syscall; do
		{ echo "--- mako /proc/$notifications/$entry ---"; cat "/proc/$notifications/$entry"; } >> "$results/notifications-state.txt" 2>&1 || :
	done
	cat "$results/notifications-state.txt" >&2
	tail -100 "$results/notifications.log" >&2
	echo 'Notification service did not register on D-Bus' >&2
	exit 1
fi
/usr/libexec/pipewire-launcher > "$results/audio.log" 2>&1 &
for _attempt in $(seq 1 30); do
	pactl info > "$results/pulse.txt" 2>/dev/null && break
	sleep 1
done
pactl load-module module-null-sink sink_name=uflinux_test > "$results/audio-module.txt"
for _attempt in $(seq 1 30); do
	if pactl set-default-sink uflinux_test 2> "$results/audio-default-error.txt" &&
		pactl info > "$results/pulse.txt" && grep -Fxq 'Default Sink: uflinux_test' "$results/pulse.txt"; then break; fi
	sleep 1
done
if ! grep -Fxq 'Default Sink: uflinux_test' "$results/pulse.txt"; then
	cat "$results/audio-default-error.txt" "$results/audio.log" >&2
	exit 1
fi
for _attempt in $(seq 1 10); do
	pactl set-sink-volume uflinux_test 50%
	pactl get-sink-volume uflinux_test > "$results/volume.txt"
	grep -q '50%' "$results/volume.txt" && break
	sleep 1
done
grep -q '50%' "$results/volume.txt"
mkdir -p /tmp/uflinux-settings-smoke-results
/usr/libexec/uflinux-settingsd > /tmp/uflinux-settings-smoke-results/service.log 2>&1 &
settings_daemon=$!; pids="$pids $settings_daemon"
for _attempt in $(seq 1 100); do
    if gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus --method org.freedesktop.DBus.NameHasOwner org.uflinux.Settings1 | grep -q true; then break; fi
    kill -0 "$settings_daemon"
    sleep .1
done
gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus --method org.freedesktop.DBus.NameHasOwner org.uflinux.Settings1 | grep -q true
G_DEBUG=fatal-criticals UFLINUX_SETTINGS_BENCH=1 /usr/bin/uflinux-desktop --daemon > "$results/desktop.log" 2>&1 &
panel=$!; pids="$pids $panel"
foot --app-id=uflinux-smoke --title=UFLinux-Smoke sh -c 'sleep 600' > "$results/foot.log" 2>&1 &
terminal=$!; pids="$pids $terminal"
for _attempt in $(seq 1 60); do
	wlrctl toplevel list > "$results/toplevels.txt"
	grep -q uflinux-smoke "$results/toplevels.txt" && break
	sleep 1
done
grep -q uflinux-smoke "$results/toplevels.txt"
wlrctl toplevel minimize app_id:uflinux-smoke
wlrctl toplevel focus app_id:uflinux-smoke
sleep 3
kill -0 "$panel"
kill -0 "$terminal"
kill -0 "$background"
kill -0 "$notifications"
notify-send --expire-time=3000 UFLinux 'Desktop validation passed'
for menu_name in network sound power; do
    uflinux-desktop --panel "$menu_name" --output HEADLESS-1
    uflinux-desktop --status | python3 -c 'import json,sys; assert json.load(sys.stdin)["popup"] == sys.argv[1]' "$menu_name"
    uflinux-desktop --hide
done
uflinux-desktop --start-menu --output HEADLESS-1
uflinux-desktop --status | python3 -c 'import json,sys; assert json.load(sys.stdin)["popup"] == "start"'
uflinux-desktop --hide
UFLINUX_DESKTOP_PID=$panel UFLINUX_DESKTOP_LOG=$results/desktop.log \
    UFLINUX_SETTINGS_DAEMON_PID=$settings_daemon sh /project/tests/settings-smoke.sh
if grep -Eiq 'panicked at|thread.*panicked|CRITICAL|assertion.*failed' "$results/desktop.log"; then
    cat "$results/desktop.log" >&2
    exit 1
fi
grim "$results/desktop.png"
ps -o pid,rss,args -p "$compositor","$panel","$terminal" > "$results/memory.txt"
printf '%s\n' 'headless compositor, configuration, panel, terminal, toplevel controls, missing battery and audio volume changes passed' > "$results/result.txt"
