# Settings and power validation

These results describe local x86_64 development builds on 8 November 2026.
The earlier ISO in `out/` is unchanged; its results are in
[the original validation record](validation.md). New images, signed APKs,
inventories, screenshots and reports are written to `out-settings/`.

The eight-architecture CI matrix is configured. Only x86_64 has been compiled
and exercised locally. Dependency availability checks passed for all eight
architectures; this does not establish their boot or graphical hardware support.
No GitHub Release or public Pages repository has been published from this workspace.

## Completed checks

- 33 repository tests, ShellCheck and actionlint pass. A QMP regression test
  ensures successive power-button control connections release their sockets.
- 21 Rust tests pass with frozen Cargo resolution and no libadwaita dependency.
- 31 selected-feature Ironbar tests and six ConnMan C tests pass.
- Fresh signed-package installation checks ELF architecture, PIE, shared-library
  resolution, rejection of damaged signatures and preservation of edited defaults.
- Real headless labwc output-management checks exercise test/apply, exact modes,
  last-display rejection, the 15-second deadline and caller-loss rollback.
- PipeWire subscriptions exercise outputs, defaults, volume, mute, recording and
  playback streams, and reconnect after a pipewire-pulse restart.
- A controlled ConnMan D-Bus fixture exercises credentials, cancellation, network
  power, addressing, DNS, proxies and forgetting without exposing secrets.
- The SPDX 2.3 package inventory validates against the official schema.
- A fresh client passed signed-index rejection without the key, installation and
  reinstallation through the staged Pages layout, damaged-package rejection and
  system/user configuration preservation. A separate locally built 0.0.9 defaults
  fixture upgraded to 0.1.0 through that layout without replacing local edits.
  This is a simulated prior generation, not a previously published UFLinux release.
  The release workflow now repeats client checks against the deployed HTTPS URL.
- Real mac80211_hwsim/hostapd tests pass WPA2 personal, WPA3 SAE, hidden networks,
  PEAP/MSCHAPv2, TTLS/PAP and EAP-TLS, including DHCP, reconnect and forgetting.
  Manual addressing, DNS/search domains, proxy configuration, route restoration,
  unrelated-route preservation and server-domain mismatch rejection pass.
- BusyBox ntpd contacts the selected controlled server, remains the sole clock
  synchronizer and reports the actual kernel synchronization state. ConnMan's
  persisted clock policies remain manual.
- A private chroot exercises the actual BusyBox acpid input parser: only power-key
  presses invoke the hook; release, repeat and unrelated input events are ignored.

## Development artifact

The updated ISO is `out-settings/images/x86_64/uflinux-0.1.0-x86_64.iso`,
942,669,824 bytes. SHA-256:

```text
e9723e6b08bc4d9e13926bf0673359d53bdc57cf19b4bdb755ef83905a058d4d
```

All 21 custom APK control/data payloads matched a fresh compiler rebuild.
The signing-input test covered all 21 packages, including mismatched-key rejection
and removal of temporary private keys. This image uses the development key.

## Measurement and acceptance

| Component | Native headless PSS |
| --- | ---: |
| Session service | 5.72 MiB |
| Settings UI | 36.77 MiB |
| Flyout alone | 31.24 MiB |

The two-second idle sample recorded 0% CPU for the service and GUI. First usable
frames were 36 ms for the full UI and 36 ms for a new flyout process;
a warm flyout activation in the existing GUI took 6 ms. These are local process
launch measurements with build caches retained, not a disk-cache-cold hardware benchmark.

Stripped executables: uflinux-settings 989,176 bytes, uflinux-settingsctl 735,328 bytes, uflinux-settingsd 1,370,184 bytes, uflinux-settings-system 775,912 bytes.
Ironbar is 6,703,128 bytes.

`checks/x86_64/settings/metrics.json` records native headless PSS, idle CPU and
first usable frame measurements. A usable frame is recorded after backend state
has arrived and GTK has painted, rather than at process entry. Native headless
results are separate from emulated installed-system measurements.

`vm/x86_64/{native,uefi}/verified.json` is written only after ISO boot,
installation into existing XFS/ext4/ESP or BIOS partitions, installed boot and
unrelated-partition preservation pass. The x86_64 test requires a real graphical
login with 512 MiB RAM, one display and no swap, then exercises all Settings pages
and the flyout. Its power checks select and persist all three real TLP profiles,
observe Settings/Ironbar synchronization, and press the virtual ACPI power button
with Do nothing and Shut down policies. Suspend policy validation is distinct
from physical suspend/resume certification.

The native firmware test additionally uses two mac80211_hwsim radios with
hostapd and DHCP for personal, hidden and enterprise Wi-Fi, plus a controlled NTP
server. Those packages and the installed Python test interpreter are harness
dependencies, not additional default desktop services.

Reports include the exact ISO SHA-256. Tests boot a snapshot of that ISO so a
parallel image rebuild cannot silently invalidate the recorded image identity.
Failed attempts leave console logs but no passing `verified.json`.

## Installed BIOS and UEFI results

Both firmware paths passed ISO boot, XFS installation, installed-system reboot,
partition-table preservation and a complete hash check of the unrelated partition.
Root remained locked and the installed account used its new password.

| Firmware | Live serial login | Background components PSS | Flyout PSS | Full UI PSS |
| --- | ---: | ---: | ---: | ---: |
| BIOS | 44.13 s | 6.29 MiB | 31.22 MiB | 34.77 MiB |
| UEFI | 54.29 s | 6.05 MiB | 31.34 MiB | 34.78 MiB |

These are QEMU software-rendering measurements with 512 MiB RAM, one display
and no swap; neither run reported OOM. Background figures include the session
service and the temporarily active privileged helper, not the entire desktop.
The live boot timing uses the separate 2 GiB live-image test, not the installed
512 MiB test.

Both runs used TLP 1.10.1, applied all three profiles, checked their persisted
configuration and observed matching Settings and Ironbar state. The actual
BusyBox acpid listener received virtual ACPI button events: Do nothing kept
the system running and Shut down completed a normal OpenRC shutdown.
Suspend policy was accepted; physical suspend/resume remains a hardware test.

The BIOS run additionally passed all six real simulated Wi-Fi modes and BusyBox
NTP checks listed above. Both verified records identify the ISO checksum shown
in this document.

## Completed follow-up work

- A filtered libudev monitor refreshes external backlight changes and device
  hotplug. DRM and resume events queue coalesced DDC discovery and display-layout
  restoration; scans and writes run outside the UI thread.
- Display dragging updates numeric coordinates. Mode, scale and rotation edits
  redraw the layout immediately. Saved layouts recover safely when a monitor or
  advertised mode disappears, including when the remaining output was disabled.
- Real headless labwc tests cover fractional scaling, rotation, hotplug,
  saved-layout fallback and rollback after output loss. AT-SPI and virtual input
  exercise dragging and default-browser selection through the actual GTK UI.
- Wallpaper restoration survives session-service restarts and cleans up only its
  owned processes. Tests preserve an unrelated wallpaper process and MIME entry.
- Installed-system tests cancel and grant authorization through the actual GTK
  polkit authentication agent. UI tests make GTK critical errors fatal, including
  repeated flyout activation and focus dismissal.
- UFLinux-owned code and packages use GPL-3.0-only. Third-party packages and
  patches retain their respective licenses.

## Remaining coverage

Physical Wi-Fi devices, GPU acceleration, HDR, VRR, DDC/CI, laptop backlights,
battery transitions and suspend/resume need separate hardware tests. Generic
architecture boot support does not imply those features work on every board.
Other CPU architectures and public-repository upgrades remain unverified locally.
Clean-build comparisons cover unsigned APK control/data payloads, not byte-identical
whole ISOs or signing streams.

This development image uses an ephemeral development key and the configured URL
`https://uflinux-project.github.io/uflinux3/apk/v3.24/uflinux`. It must not be
used as a persistent-key release or to validate public updates. The repository's
release workflow uses the configured persistent signing secrets after a version
tag passes every architecture gate. See [GitHub setup](github-setup.md) and
[physical hardware validation](hardware-validation.md).

## Live VM root-space correction, 8 November 2026

The GitHub BIOS and UEFI tests failed after live APK installation exhausted the
2 GiB VM's tmpfs root. The console recorded `failed to write database: No space
left on device`; the later package lookup therefore failed against an incomplete
APK database. The current Papirus/artwork/theme package set uses about 993 MiB in
the live root. A successful 4 GiB live guest reported a 1.9 GiB tmpfs, 967 MiB
available and 129,923 used inodes out of 501,739.

Live tests now allocate 4 GiB, with 3 GiB for armv7's QEMU highmem restriction.
The installed x86_64 acceptance budget remains 512 MiB, one display and no swap.
The runner rejects boot-time APK database/kernel memory failures explicitly,
logs byte/inode capacity, and separates package checks from Ironbar validation.

A new local development ISO including the original artwork and adw-gtk3 theme
was built and tested through both BIOS and UEFI XFS installation/reboot. Both
runs preserved the unrelated partition and passed installed Settings/flyout,
GTK authorization, TLP profiles and ACPI power-button tests without OOM. This
image is 990,904,320 bytes, SHA-256
`b076b82340114a3c902a0b4411d2846a232be87db1a0597a959332eaaa80ce9f`. It is separate from the historical ISO results above.

Machine-readable results and complete consoles are retained under
`.cache/live-memory-vm/x86_64/{native,uefi}/`; the development image is
`.cache/adw-out/images/x86_64/uflinux-0.1.0-x86_64.iso`. All 43 Python checks,
actionlint and whitespace checks passed. Other architectures and execution of
the updated GitHub workflow remain unverified for this correction. The real
hwsim network/clock fixture was not repeated in these two VM runs.
