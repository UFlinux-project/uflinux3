# Installation

Boot a matching ISO using firmware supported for that architecture. The desktop
logs in as `live`; serial consoles use **live / live**. The account can run `doas`
without a password. Use the taskbar quick settings or Settings → Network to
configure ConnMan+iwd networking. `connmanctl` is available on the console.
The live desktop and its core packages boot from the media without a network.
Installation uses the signed media repository by default. Add `--online` to allow
official Alpine mirrors for hardware firmware or other packages absent from it.

Prepare and mount the partitions yourself before running the installer. Formatting
commands destroy data; the installer intentionally does not run them.

Required layout:

- `/mnt`: a writable, empty XFS filesystem, normally at least 8 GiB for use.
- `/mnt/boot`: a separate ext4 filesystem; 1 GiB is a practical default.
- UEFI: FAT32 EFI system partition at `/mnt/boot/efi`; 512 MiB is a practical default.
- BIOS with GPT: an existing unformatted BIOS boot partition with the BIOS boot GUID.
- PowerPC: an existing PReP boot partition in addition to root and `/boot`.
- s390x: storage and boot configuration suitable for zipl.

The installer accepts only an empty root, apart from `boot` and `lost+found`.
Existing boot files may be replaced, and BIOS/PReP boot sectors are written on the
explicitly selected boot disk. This does not promise preservation of another
distribution's bootloader. Other mounted filesystems and unrelated partitions
are not selected for formatting or repartitioning.

```sh
doas uflinux-install --target /mnt --user alice --dry-run
# For BIOS or PowerPC, also supply the whole boot disk:
doas uflinux-install --target /mnt --user alice --boot-disk /dev/sda --dry-run
```

After checking the displayed devices, run the corresponding command without
`--dry-run`. Type the confirmation containing the root filesystem UUID, then
choose and repeat a password of at least eight characters. Optional flags:
`--hostname NAME`, and `--boot-mode uefi|native|auto`.

While installing from media, the installer temporarily selects its package
repositories and restores the live repository file on success or failure. The
installed system receives the full stable update repository list. Concurrent
installations are rejected with a lock; `--dry-run` does not change repositories.

UEFI is detected from the running firmware by default. ARM and RISC-V
installations in this version require UEFI; board-specific U-Boot and Raspberry Pi
installation are outside this ISO installer. An ESP must be FAT32, not just a
filesystem identified as vfat.

Alpine's `setup-disk` receives a clean overlay, writes UUID-based mounts and XFS
initramfs configuration, and installs the appropriate bootloader. The installed
system gets a new regular user, password-authenticated `doas`, locked root, and a
normal greetd login. Live credentials, live autologin, and live-only services are
excluded. If installation fails, do not boot the partial target; inspect the
reported failure and clean/reprepare the target before retrying.

Unmount target filesystems and remove the live media before booting the installed
system. Reconnect through Settings if Wi-Fi credentials are needed; live network secrets are
not copied by the installer.

## Updates and configuration

The installed repository list contains UFLinux's signed custom repository and the
matching Alpine stable `main` and `community` repositories.

```sh
doas apk update
doas apk upgrade
```

Do not mix edge packages or another Alpine release branch into this system. A
branch upgrade requires an explicitly tested UFLinux release for the new branch.
Review `.apk-new` files after upgrades if you have edited system defaults.

Use `~/.config/labwc/`, `~/.config/ironbar/config.toml`, and
`~/.config/ironbar/style.css` for personal overrides. Defaults live in
`/etc/xdg`. Ironbar uses its packaged defaults when user files are absent and
omits the battery widget on machines without a battery. Super+Space opens the
launcher; Super+Enter opens a terminal; Super+E opens files; Super+D shows the
desktop. Super+I opens Settings; the taskbar status button opens quick settings.
Audio and brightness hotkeys use the same settings backend.

To roll back a custom package, inspect the retained versions with `apk policy`,
then explicitly select the matching older package and dependency versions. For
larger rollbacks, use the immutable signed repository snapshot from that UFLinux
release. Installed XFS roots have no automatic filesystem snapshots.
