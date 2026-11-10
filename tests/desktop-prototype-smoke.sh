#!/bin/sh
# Exercise the unfinished desktop in an isolated headless labwc session.
set -eu
if [ "${UFLINUX_TEST_BUS:-}" != 1 ]; then
    exec dbus-run-session -- env UFLINUX_TEST_BUS=1 sh "$0"
fi
: "${UFLINUX_DESKTOP_BIN:?Set UFLINUX_DESKTOP_BIN to the desktop executable}"
XDG_RUNTIME_DIR=$(mktemp -d)
export XDG_RUNTIME_DIR
chmod 700 "$XDG_RUNTIME_DIR"
mkdir -p "$XDG_RUNTIME_DIR/config"
: > "$XDG_RUNTIME_DIR/config/autostart"
if [ "${UFLINUX_TEST_CATALOG:-0}" = 1 ]; then
    export XDG_DATA_HOME="$XDG_RUNTIME_DIR/data"
    mkdir -p "$XDG_DATA_HOME/applications"
fi
if [ "${UFLINUX_TEST_OUTPUT_HOTPLUG:-0}" = 1 ]; then
    cat > "$XDG_RUNTIME_DIR/config/rc.xml" <<'EOF'
<?xml version="1.0"?>
<labwc_config><keyboard><default/>
<keybind key="C-A-F10"><action name="VirtualOutputAdd" output_name="UF-TEST"/></keybind>
<keybind key="C-A-F11"><action name="VirtualOutputRemove" output_name="UF-TEST"/></keybind>
</keyboard></labwc_config>
EOF
fi
export WLR_BACKENDS=headless WLR_HEADLESS_OUTPUTS=1 WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1
export WAYLAND_DISPLAY=wayland-0 XDG_SESSION_TYPE=wayland GSK_RENDERER=cairo G_DEBUG=fatal-criticals
if [ "${UFLINUX_TEST_FRAME_PROBE:-0}" = 1 ] || [ "${UFLINUX_TEST_RESIDENT_BENCH:-0}" = 1 ]; then export UFLINUX_SETTINGS_BENCH=1; fi
compositor='' desktop='' watcher=''
cleanup() {
    [ -z "$desktop" ] || kill "$desktop" 2>/dev/null || :
    [ -z "$watcher" ] || kill "$watcher" 2>/dev/null || :
    [ -z "$compositor" ] || kill "$compositor" 2>/dev/null || :
    wait 2>/dev/null || :
    if [ -n "${UFLINUX_DEBUG_DESKTOP_LOG:-}" ] && [ -f "$XDG_RUNTIME_DIR/desktop.log" ]; then
        cp "$XDG_RUNTIME_DIR/desktop.log" "$UFLINUX_DEBUG_DESKTOP_LOG"
    fi
    rm -rf "$XDG_RUNTIME_DIR"
}
trap cleanup EXIT HUP INT TERM
labwc --config-dir "$XDG_RUNTIME_DIR/config" > "$XDG_RUNTIME_DIR/labwc.log" 2>&1 &
compositor=$!
for _ in $(seq 1 100); do
    [ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ] && break
    kill -0 "$compositor" || { cat "$XDG_RUNTIME_DIR/labwc.log"; exit 1; }
    sleep .1
done
[ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ]
if [ "${UFLINUX_TEST_EXTERNAL_TRAY:-0}" = 1 ]; then
    python3 "$(dirname "$0")/desktop-tray-protocol.py" --watcher > "$XDG_RUNTIME_DIR/watcher.log" 2>&1 &
    watcher=$!
    for _ in $(seq 1 100); do
        grep -q '^ready$' "$XDG_RUNTIME_DIR/watcher.log" && break
        kill -0 "$watcher" || { cat "$XDG_RUNTIME_DIR/watcher.log"; exit 1; }
        sleep .1
    done
    grep -q '^ready$' "$XDG_RUNTIME_DIR/watcher.log"
fi
"$UFLINUX_DESKTOP_BIN" --daemon > "$XDG_RUNTIME_DIR/desktop.log" 2>&1 &
desktop=$!
for _ in $(seq 1 100); do
    kill -0 "$desktop" || { cat "$XDG_RUNTIME_DIR/desktop.log"; exit 1; }
    if gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus --method org.freedesktop.DBus.NameHasOwner org.uflinux.Desktop | grep -q true; then break; fi
    sleep .1
done
timeout 5 "$UFLINUX_DESKTOP_BIN" --daemon
"$UFLINUX_DESKTOP_BIN" --start-menu --output HEADLESS-1
"$UFLINUX_DESKTOP_BIN" --status | python3 -c 'import json,sys; s=json.load(sys.stdin); assert s["popup"] == "start" and s["output"] == "HEADLESS-1" and isinstance(s["ready"], bool), s'
"$UFLINUX_DESKTOP_BIN" --hide
"$UFLINUX_DESKTOP_BIN" --status | python3 -c 'import json,sys; s=json.load(sys.stdin); assert s["popup"] is None, s'
for _ in $(seq 1 100); do
    if "$UFLINUX_DESKTOP_BIN" --status | python3 -c 'import json,sys; sys.exit(0 if json.load(sys.stdin)["ready"] else 1)'; then break; fi
    kill -0 "$desktop" || { cat "$XDG_RUNTIME_DIR/desktop.log"; exit 1; }
    sleep .1
done
"$UFLINUX_DESKTOP_BIN" --status | python3 -c 'import json,sys; assert json.load(sys.stdin)["ready"]'
for panel in network sound power; do
    "$UFLINUX_DESKTOP_BIN" --panel "$panel" --output HEADLESS-1
    "$UFLINUX_DESKTOP_BIN" --status | python3 -c 'import json,sys; s=json.load(sys.stdin); assert s["popup"] == sys.argv[1] and s["output"] == "HEADLESS-1", s' "$panel"
    "$UFLINUX_DESKTOP_BIN" --panel "$panel" --output HEADLESS-1
    "$UFLINUX_DESKTOP_BIN" --status | python3 -c 'import json,sys; s=json.load(sys.stdin); assert s["popup"] is None, s'
done
if [ "${UFLINUX_TEST_RESIDENT_BENCH:-0}" = 1 ]; then
    UFLINUX_DESKTOP_PID=$desktop UFLINUX_DESKTOP_LOG=$XDG_RUNTIME_DIR/desktop.log \
        python3 "$(dirname "$0")/start-menu-bench.py"
fi
if [ "${UFLINUX_TEST_SETTINGS:-0}" = 1 ]; then
    python3 "$(dirname "$0")/desktop-settings-protocol.py"
fi
if [ "${UFLINUX_TEST_TRAY:-0}" = 1 ]; then
    python3 "$(dirname "$0")/desktop-tray-protocol.py"
fi
if [ "${UFLINUX_TEST_OUTPUT_HOTPLUG:-0}" = 1 ]; then
    python3 "$(dirname "$0")/desktop-output-protocol.py"
fi
if [ "${UFLINUX_TEST_CATALOG:-0}" = 1 ]; then
    python3 "$(dirname "$0")/desktop-catalog-protocol.py"
fi
if [ "${UFLINUX_TEST_FRAME_PROBE:-0}" = 1 ]; then
    python3 "$(dirname "$0")/desktop-frame-probe.py" --log "$XDG_RUNTIME_DIR/desktop.log" \
        --output "${UFLINUX_FRAME_OUTPUT:-$XDG_RUNTIME_DIR/frames.json}" --rounds "${UFLINUX_FRAME_ROUNDS:-2}" \
        --input "${UFLINUX_FRAME_INPUT:-cli}"
fi
for _ in $(seq 1 10); do
    for panel in network sound power; do
        "$UFLINUX_DESKTOP_BIN" --panel "$panel" --output HEADLESS-1
    done
    "$UFLINUX_DESKTOP_BIN" --start-menu --output HEADLESS-1
    "$UFLINUX_DESKTOP_BIN" --hide
done
kill -0 "$desktop" || { cat "$XDG_RUNTIME_DIR/desktop.log"; exit 1; }
"$UFLINUX_DESKTOP_BIN" --quit
wait "$desktop"
desktop=''
