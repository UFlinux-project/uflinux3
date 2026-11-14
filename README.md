# UFLinux

A minimal Alpine Linux desktop with labwc and a Windows 10-inspired Ironbar taskbar.
The initial source baseline is Alpine **3.24.4** and Ironbar **0.19.1**.

The repository builds real Alpine packages and ISO images for `x86`, `x86_64`,
`aarch64`, `armv7`, `ppc64le`, and `riscv64`.
`armhf` is excluded because Alpine does not provide a standard ISO profile for it.
On release tags it additionally builds installed disk images for four device
targets: Dragon Q6A-class QCOM boards, Raspberry Pi 3–5, SpacemiT K1 boards such
as the Orange Pi RV2, and virtual machines.
The [startup animation](docs/bootsplash.md) reuses the original UFLinux leaf logo.

```sh
python3 scripts/check.py
sudo env APK_REPOSITORY_URL=https://repo.uflinux.org/apk/v3.24/uflinux \
  bash scripts/build.sh --arch x86_64 --version 0.1.0 --outdir out
```

`APK_REPOSITORY_URL` is optional: builds default to the custom Pages domain
`https://repo.uflinux.org/apk/v3.24/uflinux` for this repository (forks derive
a `github.io` URL from `GITHUB_REPOSITORY`).

Run privileged builds on a disposable Linux host. Other architectures require
QEMU binfmt registration. Local builds use development signing keys unless
`--release` and the release signing secrets are supplied. Add
`--device rpi --stage all` (or `qcs6490`, `spacemit-k1`, `vm`) to build an
installed board image instead of an ISO.

GitHub Actions compiles packages without release secrets, signs and assembles
images in separate jobs, checks the desktop, boots the x86_64 ISO in QEMU in both
BIOS and UEFI mode, installs onto preformatted XFS, and reboots from the installed
disk. Tagged releases publish only after the full package matrix and both x86_64
boot tests pass. Device disk images are built on tags as a
supplementary artifact and cannot block a release. A configured test is not evidence of a
successful run: use the workflow results and attached build records to determine
which architectures have actually passed.

The desktop includes Foot, Nemo, Mako, Transmission, Celluloid, GNOME Disks, File Roller,
and ConnMan with iwd, PipeWire with
pipewire-pulse, WirePlumber, and UFLinux Settings in Rust and plain GTK4. ConnMan's
SNTP client handles time synchronization. The taskbar opens a combined quick-settings
flyout. Print Screen opens a GNOME-style screenshot and recording tool that saves WebP
images to your Pictures folder, copies them to the clipboard and records video through a
patched GPU screen recorder; only the Settings tab for it is always available, the capture
tool itself runs on demand. GTK3 and GTK4 apps use Alpine’s adw-gtk3 dark theme. Firefox and uBlock Origin are included on x86, x86_64, aarch64, armv7, ppc64le,
and riscv64. Firefox uses a streamlined toolbar with horizontal tabs and a hidden
bookmarks bar; users can change these defaults.
Settings offers TLP Power saver, Balanced and Performance modes. Physical
power-button actions are configured in Settings and handled by BusyBox acpid.
The live login is **live / live**, with passwordless local `doas`; root is locked
and SSH is disabled. Installation creates a new password-protected user and does
not copy the live account or its autologin settings.

Installation uses an existing **XFS `/`** and a separate `/boot`: a **FAT32 ESP**
for UEFI, or **ext4** for native BIOS and other non-UEFI boot. The installer does not partition
or format disks. Architecture boot requirements still apply. Board images skip the
installer entirely: they are written to the whole SD card or eMMC and carry their
own bootloader, kernel and root partition.

- [GitHub setup and publishing](docs/github.md)
- [This repository’s signing and Pages setup](docs/github-setup.md)
- [Build stages, outputs, and source verification](docs/building.md)
- [Installation and updates](docs/installing.md)
- [Device disk images for boards and virtual machines](docs/device-images.md)
- [Architecture coverage and validation](docs/architectures.md)
- [Initial local validation results](docs/validation.md)
- [Settings architecture, controls and resource targets](docs/settings.md)
- [Settings validation results](docs/settings-validation.md)
- [Start menu controls](docs/start-menu.md)
- [Start menu validation](docs/start-menu-validation.md)
- [Physical hardware validation and evidence](docs/hardware-validation.md)
- [Original UFLinux artwork and attribution](docs/artwork.md)
- [Maintenance, signing, and key rotation](docs/maintenance.md)

UFLinux scripts, Settings and configuration use GPL-3.0-only, matching this
repository’s license. The original UFLinux leaf logo is reused under GPL-3.0-only.
Wallpapers retain their CC BY-SA 3.0 or CC0 licenses; Papirus is supplied by Alpine.
Vendored files and distributed packages retain their upstream licenses and
attribution. See the artwork guide and the ISO APK inventory.
