# Start menu validation — 9 November 2026

This historical record predates the separate resident launcher. It covers the
original plain GTK4 launcher in `.cache/start-out/`, separately
from previous development images. The earlier tested Plymouth image remains in
`.cache/splash-final-out/`. This is a development-key build, not a published release.

## Native x86_64 checks

- 48 Python repository tests, ShellCheck, actionlint, Rust formatting, shell/XML
  syntax and source-checksum checks pass.
- Frozen package builds pass 24 Rust tests, including three new catalog tests.
- Required Alpine package availability passes for all seven architectures.
  The pipeline still has seven architectures, four independent origins per
  architecture and 22 custom APK outputs. Other architectures have not run locally.
- A fresh root installs the signed packages, verifies signatures, rejects a
  damaged package, preserves modified defaults, checks ELF/library resolution,
  validates Ironbar and runs actual GTK controls under headless labwc.
- GIO fixtures cover Unicode/token search, desktop visibility, missing TryExec,
  quoted executable paths and arguments, live desktop-file additions and inline
  D-Bus activation failure. Application metadata is loaded off the GTK thread;
  ListView creates icons only for bound rows.
- Actual controls cover All Applications/back, search autofocus, arrow/Enter
  launching, Escape precedence, cancellation, authorization denial, both Super
  keys, Super+I preservation, real Ironbar clicks, repeated toggling, outside
  clicks, direct taskbar switching to quick settings, explicit output selection,
  output removal, fractional scaling and a 640×480 output.
- A private login1 fixture records and denies power operations. Cancel produces
  no operation; confirmed Restart sends one interactive request and preserves
  the menu and its error on denial. No host power action is issued by these tests.
- Closing the last window exits the GUI. Opening Start while full Settings is
  open reuses its process. Menu models, subscriptions and widgets are released.

## Native launcher measurements

| Measurement | Result |
| --- | ---: |
| First usable frame, new process | 157.6 ms |
| First usable frame, existing process | 10.5–23.8 ms |
| Launcher alone, PSS | 44.89 MiB |
| Peak PSS during ten cycles with Settings open | 48.62 MiB |
| Retained PSS change after ten cycles | 0.27 MiB |
| Focused launcher idle CPU, two-second sample | 3.0% |

Measurements use native x86_64, private headless labwc and GTK's Cairo renderer,
with cached filesystem pages. Launcher timing waits for the catalog and GTK's
after-paint signal. The focused search caret remains animated during the CPU
sample. These measurements do not establish disk-cache-cold hardware latency or
other-architecture performance. Raw data and screenshots are under
`checks/x86_64/settings/` in the build output.

## Installed VM and artifact results

The final development ISO is
`.cache/start-out/images/x86_64/uflinux-0.1.0-x86_64.iso`:
987,758,592 bytes (942 MiB), signed with a development key. SHA-256:

```text
50a87786a52f14ff735b1d680b58cafd5deeaf4dea5d5cf03d58622c4707ad3c
```

At the time of this build, Settings source SHA-512 was pinned in the source manifest and APKBUILD.
The stripped GUI executable is 1,107,968 bytes; Ironbar remains 6,703,128 bytes.
All 22 custom APKs remain present. This local incremental artifact reuses the
previously tested ConnMan and Ironbar packages and rebuilds Settings and desktop
defaults. It does not establish repeat-build payload reproducibility for this
revision; CI retains its independent compiler/rebuild jobs.

Both BIOS (`native`) and UEFI passed ISO boot, installation into existing XFS
root and ext4 boot partitions (plus a FAT32 ESP for UEFI), installed reboot,
partition-table preservation and hashing the unrelated partition. Their reports
record the exact ISO hash above. Both installed sessions used one display,
512 MiB RAM, no swap and the Pixman software renderer, with no OOM.

| Installed measurement | BIOS | UEFI |
| --- | ---: | ---: |

| Background PSS | 5.69 MiB | 5.72 MiB |
| Launcher PSS | 45.33 MiB | 45.41 MiB |
| Flyout PSS | 42.37 MiB | 42.31 MiB |
| Full Settings PSS | 46.36 MiB | 46.21 MiB |
| Peak PSS across ten launcher cycles with Settings open | 50.63 MiB | 50.51 MiB |

The software-emulated new-process launcher frame took 1,491 ms in BIOS and
1,600 ms in UEFI. These timings are separate from the native reference above;
no native-latency claim is made for QEMU. Repeated opening preserved the full
Settings process, stayed within the frontend memory budget and exited when
its final window closed.

Installed tests applied all three actual TLP profiles, verified persistence,
and observed the menu selection, session state and Ironbar variable synchronize.
BusyBox acpid policies, GTK authorization cancellation/grant, Do nothing and
actual ACPI shutdown also passed inside the disposable guests. Real suspend/
resume and physical monitor/hardware behavior require separate hardware testing.

`.cache/start-vm/x86_64/{native,uefi}/verified.json` records success;
`settings-512m.json` and `power.json` retain the measurements. `start-menu.png`
contains the installed desktop screenshot opened through the real Super binding.
Native GTK screenshots also cover small and fractionally scaled outputs.

The previous tested image remains unchanged at
`.cache/splash-final-out/images/x86_64/uflinux-0.1.0-x86_64.iso`, SHA-256
`c7bee52757977e453d1610a283d7d07b7d84eaad517fcb269c5ac528aa485b4c`.
No GitHub jobs or other-architecture builds were run for this change. The other
six architectures remain unverified, and nothing has been published.
