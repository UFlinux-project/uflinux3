#!/bin/sh
# Run as an unprivileged user inside an existing headless labwc + PipeWire session.
set -eu
results=/tmp/uflinux-settings-smoke-results
mkdir -p "$results"
daemon='' gui='' pulse=''
cleanup() {
	[ -z "$gui" ] || kill "$gui" 2>/dev/null || :
	[ -z "$daemon" ] || kill "$daemon" 2>/dev/null || :
	[ -z "$pulse" ] || kill "$pulse" 2>/dev/null || :
	wait 2>/dev/null || :
}
trap cleanup EXIT HUP INT TERM
/usr/libexec/uflinux-settingsd > "$results/service.log" 2>&1 &
daemon=$!
printf '%s\n' "$daemon" > "$results/service.pid"
sleep 1
for _attempt in $(seq 1 30); do
	uflinux-settingsctl status > "$results/state.json" 2>/dev/null || :
	if python3 - "$results/state.json" <<'PY'
import json,sys
try:
    state=json.load(open(sys.argv[1]))
    assert state['audio']['available'] and state['audio']['default_output']
    assert state['displays']['available'] and state['displays']['outputs']
except (ValueError,OSError,AssertionError,KeyError):
    sys.exit(1)
PY
	then break; fi
	kill -0 "$daemon"
	sleep 1
done
python3 - "$results/state.json" <<'PY'
import json,sys
s=json.load(open(sys.argv[1]))
assert s['audio']['available'] and s['displays']['available']
assert s['audio']['default_output'] and s['displays']['outputs']
PY
if [ "${UFLINUX_TEST_CONNMAN_FIXTURE:-0}" = 1 ]; then
	python3 "$(dirname "$0")/settings-network-protocol.py"
	# Authentication opens the quick panel through the normal activation path.
	/usr/bin/uflinux-settings --quick-settings
	sleep 1
fi
python3 "$(dirname "$0")/settings-display-protocol.py"
python3 "$(dirname "$0")/settings-preferences-protocol.py"
daemon=$(cat "$results/service.pid")
printf '%s\n' '{"action":"set-volume","input":false,"device":"","percent":42}' | uflinux-settingsctl request
for _attempt in $(seq 1 20); do
	uflinux-settingsctl status > "$results/state.json"
	if python3 - "$results/state.json" <<'PY'
import json,sys
s=json.load(open(sys.argv[1]));d=next(d for d in s['audio']['outputs'] if d['name']==s['audio']['default_output'])
sys.exit(0 if abs(d['volume']-42)<1 else 1)
PY
	then break; fi
	sleep 1
done
! printf '%s\n' '{"action":"set-volume","input":false,"device":"","percent":999}' | uflinux-settingsctl request
! printf '%s\n' '{"action":"system","operation":{"operation":"execute-shell","command":"id"}}' | uflinux-settingsctl request
# A non-administrator in this headless session must not bypass polkit.
! printf '%s\n' '{"action":"system","operation":{"operation":"set-timezone","timezone":"Etc/UTC"}}' | uflinux-settingsctl request
printf '%s\n' '{"action":"refresh"}' | uflinux-settingsctl request
# Restart only the Pulse server in this disposable user's test session.
old_pulse=$(python3 - <<'PY'
import os
from pathlib import Path
matches=[]
runtime=('XDG_RUNTIME_DIR='+os.environ['XDG_RUNTIME_DIR']).encode()
for path in Path('/proc').glob('[0-9]*/comm'):
    try:
        if path.stat().st_uid==os.getuid() and path.read_text().strip()=='pipewire-pulse' and runtime in (path.parent/'environ').read_bytes().split(b'\0'):
            matches.append(path.parent.name)
    except OSError: pass
assert len(matches)==1, matches
print(matches[0])
PY
)
kill "$old_pulse"
/usr/bin/pipewire-pulse > "$results/pulse-restart.log" 2>&1 &
pulse=$!
for _attempt in $(seq 1 20); do
	uflinux-settingsctl status > "$results/state.json"
	if python3 - "$results/state.json" <<'PY'
import json,sys
s=json.load(open(sys.argv[1]));sys.exit(0 if s['audio']['available'] and s['audio']['outputs'] else 1)
PY
	then break; fi
	sleep 1
done
G_DEBUG=fatal-criticals UFLINUX_SETTINGS_BENCH=1 /usr/bin/uflinux-settings --quick-settings --output HEADLESS-1 > "$results/flyout.log" 2>&1 &
gui=$!
sleep 2
python3 - "$gui" "$results/flyout-pss.txt" <<'PY'
from pathlib import Path
import sys
Path(sys.argv[2]).write_text(next(l.split()[1] for l in Path('/proc',sys.argv[1],'smaps_rollup').read_text().splitlines() if l.startswith('Pss:')))
PY
/usr/bin/uflinux-settings --quick-settings --output HEADLESS-1
wait "$gui"
gui=''
G_DEBUG=fatal-criticals UFLINUX_SETTINGS_BENCH=1 /usr/bin/uflinux-settings --page network > "$results/ui.log" 2>&1 &
gui=$!
for _attempt in $(seq 1 30); do
	wlrctl toplevel list > "$results/windows.txt"
	grep -q org.uflinux.Settings "$results/windows.txt" && break
	kill -0 "$gui"
	sleep 1
done
grep -q org.uflinux.Settings "$results/windows.txt"
sh "$(dirname "$0")/build-ui-driver.sh" /tmp/uflinux-ui-driver
python3 "$(dirname "$0")/settings-ui-protocol.py"
for page in network displays audio power default-apps datetime appearance about; do
	/usr/bin/uflinux-settings --page "$page"
	sleep 1
	kill -0 "$gui"
	grim "$results/$page.png"
	if [ "$page" = audio ]; then python3 "$(dirname "$0")/settings-audio-protocol.py"; fi
done
/usr/bin/uflinux-settings --quick-settings --output HEADLESS-1
sleep 2
kill -0 "$gui"
grim "$results/quick-settings.png"
python3 - "$daemon" "$gui" "$results/metrics.json" <<'PY'
import json,sys,os,time,re
from pathlib import Path
def pss(pid):
    return int(next(l.split()[1] for l in Path('/proc',pid,'smaps_rollup').read_text().splitlines() if l.startswith('Pss:')))*1024
records={'service_pss_bytes':pss(sys.argv[1]),'ui_pss_bytes':pss(sys.argv[2]),'scope':'headless native smoke; installed-system RAM is tested separately'}
directory=Path(sys.argv[3]).parent
records['flyout_pss_bytes']=int((directory/'flyout-pss.txt').read_text())*1024
def ticks(pid):
    fields=Path('/proc',pid,'stat').read_text().rsplit(')',1)[1].split()
    return int(fields[11])+int(fields[12])
before=[ticks(pid) for pid in sys.argv[1:3]];start=time.monotonic();time.sleep(2);elapsed=time.monotonic()-start
records['idle_cpu_percent']={name:round((ticks(pid)-initial)/os.sysconf('SC_CLK_TCK')/elapsed*100,3) for name,pid,initial in zip(('service','ui'),sys.argv[1:3],before)}
records['first_usable_frame_ms']={name:[int(n) for n in re.findall(r'UFLINUX_FIRST_FRAME_MS=(\d+)',(directory/filename).read_text())] for name,filename in [('full','ui.log'),('flyout','flyout.log')]}
Path(sys.argv[3]).write_text(json.dumps(records,indent=2)+'\n')
maps=Path('/proc',sys.argv[1],'maps').read_text()
assert 'libgtk-4' not in maps and 'libadwaita' not in maps
PY
/usr/bin/uflinux-settings --quick-settings --output HEADLESS-1
wlrctl toplevel close app_id:org.uflinux.Settings
for _attempt in $(seq 1 10); do
	kill -0 "$gui" 2>/dev/null || break
	sleep 1
done
if kill -0 "$gui" 2>/dev/null; then
	echo 'GUI remained alive after its last window closed' >&2
	exit 1
fi
gui=
if grep -Eiq 'panicked at|thread.*panicked|CRITICAL|assertion.*failed' "$results/ui.log" "$results/flyout.log" "$results/service.log"; then
	cat "$results/ui.log" "$results/service.log" >&2
	exit 1
fi
printf '%s\n' 'settings pages, quick controls, shared audio, input rejection and GUI lifetime passed' > "$results/result.txt"
