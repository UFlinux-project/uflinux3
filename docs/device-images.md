# Device images

Besides the generic live ISOs, UFLinux builds **installed** GPT disk images for
specific boards. A device image writes straight to the board's SD card or eMMC,
boots to the login screen, and grows its root partition to the size of the media
on first boot. There is no squashfs modloop, no `live` user and no live session.

Device images are **supplementary**: they are built on tags and attached to the
same release as the ISOs, but a failing or missing image never blocks publication
of the ISOs and the signed APK repository.

## Targets

| Id | Boards | Alpine arch | Kernel | Boot | Root fs | Status |
| --- | --- | --- | --- | --- | --- | --- |
| `qcs6490` | Arrow Dragon Q6A and similar QCS6490 boards | aarch64 | `linux-lts` | board UEFI + `systemd-boot` | f2fs | experimental |
| `rpi` | Raspberry Pi 3, 4 and 5 | aarch64 | `linux-rpi` | VideoCore firmware | f2fs | supported |
| `spacemit-k1` | SpacemiT K1 boards (Orange Pi RV2, Orange Pi R2S, BananaPi-F3) | riscv64 | `linux-lts` | U-Boot distro boot (`extlinux`) | f2fs | experimental |
| `vm` | QEMU, KVM and other hypervisors | x86_64, aarch64 | `linux-virt` | UEFI + `systemd-boot` | XFS | supported |

The table, the per-board package list, the root filesystem, the serial console and
the kernel command line come from `config/devices.json`; the optional Nura
repository is pinned in `config/devices.lock.json`. Both files are validated by
`python3 scripts/check.py`, and every package name is checked against the live
Alpine index by `python3 scripts/check-packages.py`.

`spacemit-k1` is the Orange Pi RV2: its SpacemiT Ky X1 is a RISC-V SoC of the K1
family, so the image is riscv64 and uses Alpine's generic `linux-lts`, which is
built with `CONFIG_ARCH_SPACEMIT=y` and ships `spacemit/k1-orangepi-rv2.dtb`. The
mainline K1 device tree has no display or GPU node, so the desktop starts but the
board is expected to be driven over the serial console (`ttyS0`) or headless. The
image reuses the board's own U-Boot through `extlinux/extlinux.conf` on the ESP.

`qcs6490` is experimental for a different reason: some QCS6490 boards have no
firmware-provided device tree. The image therefore carries the Nura `dtbloader`
EFI driver in `EFI/systemd/drivers`, which installs a board device tree before
`systemd-boot` runs. Nura (postmarketOS) is used **only** as an extra aarch64
package source pinned to a specific branch and index hash, and only for that one
driver. That driver is *fetched and unpacked* rather than installed: Nura ships
its own `mkinitfs` replacement which conflicts with the Alpine one the image
needs, so apk must never solve across both repositories at once (see "Package
sources"). Kernels, bootloaders and drivers always come from Alpine, and no Nura
`device-*` package is ever installed.

## Building one locally

```sh
sudo env APK_REPOSITORY_URL=https://OWNER.github.io/REPO/apk/v3.24/uflinux \
  bash scripts/build.sh --arch aarch64 --version 0.1.0 --device rpi --stage all
```

`--device` is accepted only together with `--stage device` (or `--stage all`) and
only for an architecture the board supports. The stage reuses the ordinary
`bootstrap`, `packages` and `sign` stages, so the packages must already be built
and collected for that architecture (`scripts/merge-packages.py`) exactly as for
an ISO. Each device gets its own work and cache directory below
`.state/device/BOARD` and `.cache/build/device/BOARD`, so kernels and firmware do
not leak into the ISO inputs.

Outputs:

| Path | Contents |
| --- | --- |
| `out/devices/ARCH/uflinux-VERSION-DEVICE-ARCH.img.zst` | Raw GPT disk image, Zstandard level 12 |
| `out/devices/ARCH/…img.zst.sha256` | Checksum sidecar |
| `out/metadata/ARCH/device-DEVICE.json` | Build record: geometry, GUIDs, boot file sizes, installed package list |
| `out/checks/ARCH/device-DEVICE.json` | Verification report for the assembled disk |

## Disk layout and boot

Every image has the same two-partition GPT layout, built without loop devices or
mounts so that it is identical on a runner and on a developer machine:

| Partition | Start (512 B sectors) | Size | Filesystem |
| --- | --- | --- | --- |
| ESP | 2048 | 1 048 576 (512 MiB) | FAT32, label `UFLINUX` |
| ROOT | 1 050 624 | rest of the disk minus one alignment gap | f2fs on boards, XFS on `vm`, label `UFLINUX_ROOT` |

f2fs is the root filesystem on the SBC targets because it is built for the
wear-levelling and small-write behaviour of SD cards and eMMC. The `vm` image uses
XFS, the same filesystem as the installed ISO target, since a virtual disk has
none of those constraints. `mkfs.f2fs` and `mkfs.xfs` cannot write a tree at a
partition offset inside a disk image the way `mke2fs -d` does, so those two roots
are built as a standalone volume file (`mkfs.f2fs` plus `sload.f2fs`, or
`mkfs.xfs -p DIRECTORY`) and placed with a single MiB-aligned `dd`. Those formatters
run on the build host rather than in the build chroot, so a local device build needs
`f2fs-tools` and `xfsprogs` installed there in addition to the tools an ISO build
uses; the `device-images` CI job installs them.

The disk, partition and FAT volume identifiers are derived from the version, board
id and architecture, so a rebuild produces the same GUIDs. The kernel command line
uses `root=PARTUUID=<ROOT partition GUID> rootwait rootfstype=<fs> rootflags=rw`:
Alpine's `nlplug-findfs` matches `PARTUUID` against the **partition** GUID, not the
disk GUID, and the explicit type means the initramfs carries only the one
filesystem module it needs. The ESP carries the kernel as `boot/vmlinuz` and the
initramfs as `boot/initramfs`, plus whatever the boot scheme needs:

- `efi` — `EFI/boot/bootaa64.efi` or `EFI/boot/bootx64.efi` (systemd-boot, copied
  from the package because installing it needs firmware variables) and
  `loader/entries/uflinux.conf`.
- `rpi` — `config.txt`, `cmdline.txt`, `usercfg.txt`, the VideoCore firmware and
  the device-tree overlays, all unpacked from the bootloader APKs.
- `extlinux` — `extlinux/extlinux.conf` plus the board device tree under `dtbs/`.

The initramfs is generated by `profiles/device-rootfs.sh` with an explicit feature
set (`base <rootfs> phy mmc usb scsi nvme ata virtio kms`, where `<rootfs>` is
`f2fs` or `xfs`) rather than through `update-kernel`, because that helper always
passes `--media` and would build the live ISO's squashfs modloop. It is verified to
be a Zstandard frame.

## Package sources

While an image is built, its rootfs reads packages from the signed repository that
the `sign` stage just produced, copied into the rootfs as `/repo`, together with
Alpine and (for `qcs6490` only) the pinned Nura suite. The published Pages URL is
not used at build time, because it only lists an architecture after a release has
been published. That copy is a build artefact and is excluded from the image; the
shipped `/etc/apk/repositories` lists the published UFLinux repository and Alpine,
so `apk update` on the board works as soon as the release is public. apk appends
the architecture to every entry itself, so the Nura line ends at the branch name.

Installed packages come only from Alpine and the signed UFLinux repository. Nura
packages are fetched with `apk fetch --stdout` (which resolves the file from the
pinned index without consulting the solver) and unpacked with `tar` at their
package paths. Installing them instead would let apk see Nura's
`postmarketos-mkinitfs` and Alpine's `mkinitfs` — mutually conflicting providers
of `cmd:mkinitfs` — and refuse to select any package set that pulls in a kernel.
The unpacked files are ordinary rootfs content, so they land in the image without
a Nura entry in the installed database.

## First boot

The `uflinux-device` OpenRC service (from the `uflinux` package) runs early in the
`boot` runlevel. It is inert unless `/etc/uflinux/device` exists, so it does nothing
on an ordinary installed system; a device image writes that file with the board id.
When armed it runs `growpart` on the root partition once and then grows the
filesystem to fill it — `resize.f2fs -f` for f2fs (while the root is still mounted
read-only, which is the only way the kernel allows), `xfs_growfs /` for XFS and
`resize2fs` for ext4 — and records `/var/lib/uflinux/device-resized`, so a later
partition change is not undone and a reboot does not repeat the work. Failures are
warnings, not boot failures.

An f2fs root is grown in place, but the kernel does not re-read the superblock on
remount, so the new space only becomes visible in `df` after the next reboot. The
filesystem is already usable at its full size for writes that need it.

Images ship **no regular user**. `root` has an empty password, so the first login is
`root` with a blank password; log in over the serial console or on the framebuffer
console (`tty1`) and then:

```sh
passwd
adduser -G wheel,audio,video,netdev,input yourname
```

`greetd` with the UFLinux greeter is enabled by default, so the new account can log
in graphically. Serial gettys are configured per board from the board description
(`ttyMSM0`, `ttyAMA0` or `ttyS0` at 115200 8n1) in addition to `tty1`/`tty2`.

## Writing an image

```sh
zstd -d < uflinux-0.1.0-rpi-aarch64.img.zst | sudo dd of=/dev/sdX bs=8M oflag=direct status=progress
sync
```

The uncompressed image is 6144 MiB (5120 MiB for `vm`), so any larger card or eMMC
works and the root partition grows to fill it. Verify the checksum sidecar and the
GitHub attestation first:

```sh
sha256sum -c uflinux-0.1.0-rpi-aarch64.img.zst.sha256
gh attestation verify uflinux-0.1.0-rpi-aarch64.img.zst --repo OWNER/REPO
```

## Continuous integration

`device-images` is a matrix job that runs only on version tags (and on manual runs
where the **Build device images** option is checked). It reuses the compile
artifacts, signs the packages, and assembles one image per board/architecture pair,
with at most four at a time. Experimental entries use `continue-on-error`, so their
logs and artifacts are published without failing the release.

`device-assets` runs after `publish` and uploads the images, their checksums and
their build attestations to the already-published release. Because it runs last and
is marked `continue-on-error`, a device problem can never leave a release half
published.

## Status of the evidence

The device stage has unit tests for the geometry, identifiers, package set, boot
configuration contract and the post-build disk verification, and every package name
is checked against the live Alpine and pinned Nura indexes. **No device image has
been booted on physical hardware yet**, and the images are not covered by the QEMU
ISO boot tests. That is why the two generic-kernel board targets are labelled
experimental and why the release notes describe device images as a preview.
