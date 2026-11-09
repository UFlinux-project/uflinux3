# Startup animation

UFLinux displays the original UFLinux leaf above a small rotating blue ring on
a dark background. The leaf is rasterized from the pinned original SVG during
packaging; the spinner and Plymouth script are UFLinux code under GPL-3.0-only.
The old `UFlinux-project/ufplymouththeme` repository, at commit
`f56aa102eb205c174fe28a64064a134c5e813e52`, contains only a README and license.
It does not supply an animation that can be imported. Provenance is recorded in
`config/artwork.lock.json` and the existing artwork notices.

The implementation uses Alpine's prebuilt `plymouth` and `plymouth-drm` packages,
without X11 rendering or a custom resident daemon. `rsvg-convert` is a build-only
dependency. The theme lives in `/usr/share/plymouth/themes/uflinux`; its packaged
configuration is `/etc/plymouth/plymouthd.conf`.

## Boot lifecycle and fallback

`uflinux-splash` starts in OpenRC's sysinit runlevel after udev/hardware discovery.
`uflinux-splash-stop` releases the display before greetd starts. Both live and
installed overlays enable these services. This covers the OpenRC startup phase;
it does not replace bootloader output, kernel/initramfs diagnostics, or the live
image's initial APK unpacking. The existing Zstandard boot artifacts are retained.

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
