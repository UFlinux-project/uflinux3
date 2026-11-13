# Unified desktop prototype validation

The unified desktop is packaged separately for isolated testing. The default
session still uses Ironbar until the functionality and release gates in the
replacement plan pass. Do not run the prototype beside the normal panel in a
regular session: both provide panel and tray services.

The isolated smoke test runs a headless labwc session with a private D-Bus bus:

```sh
UFLINUX_DESKTOP_BIN=/path/to/uflinux-desktop sh tests/desktop-prototype-smoke.sh
```

Set `UFLINUX_TEST_TRAY=1` to exercise a StatusNotifier item and DBusMenu. The
fixture icon is behind the taskbar overflow chevron, so the check opens the
flyout through its accessible name first and also asserts the chevron is absent
before the first registration and again after the last item disconnects. Add
`UFLINUX_TEST_EXTERNAL_TRAY=1` to exercise an existing external watcher, or
`UFLINUX_TEST_STALLED_TRAY=1` to include an unresponsive tray client. Set
`UFLINUX_TEST_SETTINGS=1` and `UFLINUX_SETTINGS_SNAPSHOT_BIN` to the built
`settings_snapshot` example to test state changes and service restart. To test
output hotplug, build the drivers with `tests/build-ui-driver.sh BUILD_DIR`,
then set `UFLINUX_TEST_OUTPUT_HOTPLUG=1` and
`UFLINUX_KEYBOARD_DRIVER=BUILD_DIR/keyboard-driver`. Set `UFLINUX_TEST_CATALOG=1`
to exercise desktop-entry additions and removals in an isolated data directory.
These tests need labwc,
GTK accessibility, GIO Python bindings, and a Wayland-capable environment.
For a short painted-frame and accessibility diagnostic, also set
`UFLINUX_TEST_FRAME_PROBE=1` with `UFLINUX_TEST_SETTINGS=1`; optionally set
`UFLINUX_FRAME_ROUNDS` and `UFLINUX_FRAME_OUTPUT`. This probe includes command
startup and accessibility observation overhead, so its times are upper bounds
and do not replace the panel-input latency gate.

`tests/desktop-memory-bench.py` collects a single native private-session memory
and idle-CPU sample. Run it separately for each implementation, with release
binaries and no other benchmark sessions running. For example:

```sh
python3 tests/desktop-memory-bench.py --mode baseline \
  --legacy-dir /path/to/original-release --ironbar /path/to/original-ironbar \
  --desktop /path/to/new/uflinux-desktop --daemon /path/to/original/uflinux-settingsd \
  --output /path/to/results/baseline-1.json
```

Repeat with `--mode replacement` and a distinct output path, using the same
daemon, catalog and fixture environment. The default is 100 cycles through all
four menus followed by 60 seconds of idle accounting. Each run snapshots its
device and tray fixtures, records their hashes and the binary hashes, and
collects separate logs beside its JSON output. The process totals cover
Settings daemon, Ironbar and resident Start for the baseline, and Settings
daemon, desktop supervisor, and its descendants for the replacement. They exclude the test fixtures and
compositor on both sides. The privileged helper is not active in this fixture.
Use `scripts/desktop-memory-summary.py` with three `--baseline` and three
`--replacement` paths to compare matching sessions. It rejects inconsistent
fixtures, binaries, or insufficient cycle and idle durations. Because the
fixture hashes are part of that comparison, editing a device or tray fixture
between sessions of one set makes the set incomparable; re-collect the sessions
that predate the edit rather than mixing them.

The memory work in the unified desktop — releasing an unmapped popup surface,
decoding the panel logo at its displayed size, a built-in supervisor instead of
a shell wrapper and a separate launcher process, deferred quick-settings trees,
a bounded allocator, and trimming on hide — passed the memory and idle-CPU
checks with three sessions per implementation. Median process-set proportional
size fell from 86.8 to 73.5 MiB once settled and from 89.2 to 75.9 MiB after one
pass through all four menus, both a 15% reduction against a 10% requirement. No
replacement session retained more than 0.4 MiB across 100 cycles, no session grew
its descriptor count, and desktop idle CPU peaked at 0.067%. Those figures come
from one headless-pixman environment on one kernel and are not a release
approval.

This collector does not measure interactive frame latency, widget or D-Bus
subscription growth, real audio devices, multi-architecture builds, or an
installed low-memory VM. It waits for popup state during cycles but does not
prove that each cycle painted an interactive frame. Its JSON is therefore a diagnostic artifact, not a
complete input to `scripts/desktop-performance-gate.py` and not evidence that
the default switch is approved. The full release gate still requires three
fresh comparable sessions per implementation, interactive latency measurements,
and the remaining functionality and installed-system checks.
