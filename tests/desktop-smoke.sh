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
	for daemon in pipewire pipewire-pulse wireplumber; do
		pkill -u "$(id -u)" -x "$daemon" 2>/dev/null || :
	done
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
swaybg -c '#102c46' -i /usr/share/backgrounds/uflinux.svg -m fill > "$results/background.log" 2>&1 &
background=$!; pids="$pids $background"
mako > "$results/notifications.log" 2>&1 &
notifications=$!; pids="$pids $notifications"
fuzzel > "$results/launcher.log" 2>&1 &
launcher=$!
sleep 1
kill -0 "$launcher"
kill "$launcher"
wait "$launcher" 2>/dev/null || :
/usr/libexec/uflinux-start-panel --validate-config > "$results/config.log" 2>&1
if grep -q 'type = "battery"' "$XDG_RUNTIME_DIR/uflinux-ironbar.toml"; then
	echo 'Battery-free default still contains the battery module' >&2
	exit 1
fi
/usr/libexec/pipewire-launcher > "$results/audio.log" 2>&1 &
for _attempt in $(seq 1 30); do
	pactl info > "$results/pulse.txt" 2>/dev/null && break
	sleep 1
done
pactl load-module module-null-sink sink_name=uflinux_test > "$results/audio-module.txt"
pactl set-default-sink uflinux_test
for _attempt in $(seq 1 10); do
	pactl set-sink-volume uflinux_test 50%
	pactl get-sink-volume uflinux_test > "$results/volume.txt"
	grep -q '50%' "$results/volume.txt" && break
	sleep 1
done
grep -q '50%' "$results/volume.txt"
/usr/libexec/uflinux-start-panel > "$results/ironbar.log" 2>&1 &
panel=$!; pids="$pids $panel"
foot --app-id=uflinux-smoke --title=UFLinux-Smoke sh -c 'sleep 120' > "$results/foot.log" 2>&1 &
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
sh /project/tests/settings-smoke.sh
# Detect Ironbar's fallback-to-default behavior as well as fatal parse failures.
if grep -Eiq 'error loading config|failed to load.*config|invalid.*config|panicked at' "$results/ironbar.log"; then
	cat "$results/ironbar.log" >&2
	exit 1
fi
grim "$results/desktop.png"
ps -o pid,rss,args -p "$compositor","$panel","$terminal" > "$results/memory.txt"
printf '%s\n' 'headless compositor, configuration, panel, terminal, toplevel controls, missing battery and audio volume changes passed' > "$results/result.txt"
