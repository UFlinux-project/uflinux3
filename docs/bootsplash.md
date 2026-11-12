# Startup animation

UFLinux shows the same composition on every machine: the firmware's own boot
logo keeps the centre of the screen, the UFLinux spinner rotates below it, and
the original UFLinux logotype sits below the spinner. The layout follows what
UEFI machines report through the ACPI BGRT table, so the vendor logo is never
moved, scaled or overpainted.

The theme uses Plymouth's `two-step` module with `UseFirmwareBackground=true`,
which is the only Plymouth plugin that reads
`/sys/firmware/acpi/bgrt/{status,xoffset,yoffset,image}`. On machines without
BGRT — BIOS firmware, most ARM boards, virtual machines — `bgrt-fallback.png`
occupies exactly the same slot, so the original white leaf appears where the
firmware logo would have been and nothing shifts. The firmware logo, the leaf
and the logotype are all derived from pinned upstream artwork; provenance is in
`config/artwork.lock.json` and `artwork-NOTICE`.

Packaging keeps the upstream bootloader splash unaltered as
`uflinux-splash-original.png`. `uflinux-boot-assets.py` (standard library only)
copies its leaf box to `bgrt-fallback.png` and its logotype box to
`watermark.png`, converting the black background into alpha without resampling,
so both images also compose correctly over a vendor logo. The spinner is
rasterized into 24 `throbber-NNNN.png` frames at a 15-degree step, which the
`two-step` module animates itself; `lock.png`, `entry.png` and `bullet.png`
cover the passphrase and filesystem-check dialogs. `rsvg-convert` is build-only.
The theme lives in `/usr/share/plymouth/themes/uflinux`; its packaged
configuration is `/etc/plymouth/plymouthd.conf`.

## Boot lifecycle and fallback

`uflinux-initramfs-init` runs Alpine's own init with three injected hooks: it
prepares `/run` before the kernel options are read, starts the splash as soon as
the boot-driver module probes finish (right after `simpledrm`, so the animation
is up before root probing or the live image's APK unpacking), repeats that start
at both single-user checkpoints for slow device nodes, and calls
`plymouth update-root-fs` at each root handoff so the daemon survives
`switch_root`. If Alpine's init no longer has the expected structure, the
wrapper boots it unmodified and the OpenRC services take over.

`uflinux-splash` starts in OpenRC's sysinit runlevel after udev/hardware
discovery and `uflinux-splash-stop` releases the display before greetd starts.
Both live and installed overlays enable these services, which also cover the
case where the initramfs hooks were skipped. The existing Zstandard boot
artifacts are retained.

The renderer uses the local VT and DRM/framebuffer device. Serial output remains
available. Machines without a local VT or graphics device skip the splash. Add
`splash=no`, `nosplash`, or `plymouth.enable=0` to the kernel command line to disable
it and retain normal boot output. No artificial delay is added to normal boot.

Daemon startup and client calls are bounded. Renderer failure continues boot and
records a short message; details are in `/run/uflinux-splash/daemon.log`. Runtime
state is private to root. Cleanup matches the daemon executable and its private
pidfile. No Plymouth daemon is intended to remain after the display handoff.

## Validation

Fast tests run the production lifecycle with isolated fake devices/commands,
covering opt-out flags, serial-only boot, renderer failure, private runtime state,
idempotent start/stop and cleanup. They never open host DRM devices.

The x86_64 VM test checks automatic boot handoff, temporarily stops greetd inside
the disposable live guest, starts the splash, and captures two QMP screenshots.
It requires moving frames, blue spinner pixels and the original white logo, then
checks daemon exit and restarts greetd. The installed 512 MiB acceptance test also
runs through the normal splash-to-greetd handoff. Other graphics hardware requires
separate validation; architecture support does not guarantee a usable local GPU.

Completed locally on 8 November 2026: 48 Python regression checks, ShellCheck,
actionlint, package availability checks for all seven targets, native UFLinux
package compilation, and fresh-root signature/ELF/library/headless desktop checks
for all 22 custom APKs. Existing unchanged ConnMan, Ironbar and Settings APKs were
reused and their original signatures verified before development signing.

Both x86_64 BIOS and UEFI runs passed animation capture, live and installed
handoff, XFS installation/reboot and unrelated-partition preservation. Settings,
flyout, GTK authorization, TLP and ACPI tests passed at 512 MiB with no swap or OOM.
These runs did not repeat hwsim network tests or validate physical GPU coverage.
The other six architecture images have not been rebuilt/boot-tested locally for
this change. No boot-speed or byte-identical rebuild claim is made.

Development ISO: `.cache/splash-final-out/images/x86_64/uflinux-0.1.0-x86_64.iso`
(987,758,592 bytes, 942 MiB).

SHA-256: `c7bee52757977e453d1610a283d7d07b7d84eaad517fcb269c5ac528aa485b4c`.

Reports, captured PPM frames and consoles are in
`.cache/splash-vm/x86_64/{native,uefi}/`; a PNG preview is
`.cache/splash-vm/x86_64/native/bootsplash.png`. Fresh-root reports are under
`.cache/splash-final-out/checks/x86_64/`. Build logs use `.cache/splash-final-*.log`.
