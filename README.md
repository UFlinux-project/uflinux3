# UFLinux

A minimal Alpine Linux desktop with labwc and a Windows 10-inspired Ironbar taskbar.
The initial source baseline is Alpine **3.24.4** and Ironbar **0.19.1**.

The repository builds real Alpine packages and ISO images for `x86`, `x86_64`,
`aarch64`, `armv7`, `ppc64le`, `s390x`, and `riscv64`.
`armhf` is excluded because Alpine does not provide a standard ISO profile for it.
The [startup animation](docs/bootsplash.md) reuses the original UFLinux leaf logo.

```sh
python3 scripts/check.py
sudo env APK_REPOSITORY_URL=https://OWNER.github.io/REPO/apk/v3.24/uflinux \
  bash scripts/build.sh --arch x86_64 --version 0.1.0 --outdir out
```

Run privileged builds on a disposable Linux host. Other architectures require
QEMU binfmt registration. Local builds use development signing keys unless
`--release` and the release signing secrets are supplied.

GitHub Actions compiles packages without release secrets, signs and assembles
images in separate jobs, checks the desktop, boots the ISO in QEMU, installs onto
preformatted XFS, and reboots from the installed disk. Tagged releases publish
only after the full matrix passes. A configured test is not evidence of a
successful run: use the workflow results and attached build records to determine
which architectures have actually passed.

The desktop includes Foot, Thunar, Fuzzel, Mako, ConnMan with iwd, PipeWire with
pipewire-pulse, WirePlumber, and UFLinux Settings in Rust and plain GTK4. BusyBox
ntpd handles time synchronization. The taskbar opens a combined quick-settings
flyout. GTK3 and GTK4 apps use Alpine’s adw-gtk3 dark theme. It has no browser or office suite by default.
Settings and Start offer TLP Power saver, Balanced and Performance modes. Physical
power-button actions are configured in Settings and handled by BusyBox acpid.
The live login is **live / live**, with passwordless local `doas`; root is locked
and SSH is disabled. Installation creates a new password-protected user and does
not copy the live account or its autologin settings.

Installation uses existing partitions: **XFS `/`**, separate **ext4 `/boot`**, and
a **FAT32 ESP** mounted at `/boot/efi` for UEFI. The installer does not partition
or format disks. Architecture boot requirements still apply.

- [GitHub setup and publishing](docs/github.md)
- [This repository’s signing and Pages setup](docs/github-setup.md)
- [Build stages, outputs, and source verification](docs/building.md)
- [Installation and updates](docs/installing.md)
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
