# Architecture and boot coverage

| Alpine architecture | Package build | VM machine | Required VM boot tests |
| --- | --- | --- | --- |
| x86 | native compatibility mode | pc | BIOS and 32-bit UEFI |
| x86_64 | native | q35 | BIOS and UEFI |
| aarch64 | native ARM64 runner | virt | UEFI |
| armv7 | ARM64 runner, qemu-arm | virt / cortex-a15 | UEFI |
| ppc64le | qemu-ppc64le | pseries / POWER9 | Open Firmware |
| s390x | qemu-s390x | s390-ccw-virtio | native IPL / zipl |
| riscv64 | qemu-riscv64 | virt | OpenSBI and UEFI |

## Device image targets

| Board id | Package build | Boot firmware | Status |
| --- | --- | --- | --- |
| `qcs6490` | native ARM64 runner | board UEFI plus `systemd-boot` | experimental |
| `rpi` | native ARM64 runner | VideoCore firmware with `config.txt` | supported |
| `spacemit-k1` | qemu-riscv64 | U-Boot distro boot with `extlinux` | experimental |
| `vm` | native x86_64 and ARM64 runners | UEFI with `systemd-boot` | supported |

Device images are supplementary: they are built on tags and attached to the
release, but they are not part of the publication gate and none of them has a
configured boot test yet. See [device images](device-images.md).

This table describes the configured ISO build/test matrix, not completed physical
hardware certification. Publication requires green test reports for all rows.
The firmware search paths are explicit in `scripts/vm-test.py`; missing firmware
is a test failure, not a skipped architecture.

UEFI tests use Secure Boot disabled. These images do not provide enrolled UEFI
Secure Boot signatures; APK and modloop signatures are separate mechanisms.

Ubuntu 26.04 no longer packages 32-bit OVMF or ARM UEFI. Those two tests download
the official Ubuntu 24.04 firmware archives pinned in `config/firmware.lock.json`,
verify their SHA-256 hashes, and extract them into a test cache without installing
old packages into the runner or the distribution. Other firmware comes from the
runner's current Ubuntu packages.

An architecture name does not guarantee an ISO can boot every board using that
CPU. Generic ARM ISOs require appropriate firmware and do not replace Alpine's
board-specific Raspberry Pi tarballs. Mainframes commonly lack a local display;
UFLinux provides a serial console, while the desktop can be tested with a headless
Wayland backend. No remote-desktop service is enabled by default.

Every target runs Ironbar's upstream tests during compilation. Verification
checks the actual Ironbar config, a headless labwc compositor, foreign-toplevel
protocol availability, terminal mapping, minimizing/focusing, panel survival,
screenshot creation, and memory reporting. Missing physical battery/network/audio
hardware is not represented as physical-device testing. The VM tests exercise real
OpenRC boot, networking for package access, installer validation, XFS mounting,
user/password setup, preservation of an unrelated partition, and installed boot.

The graphical stack uses packaged Mesa and Alpine kernels/firmware. Physical GPU,
Wi-Fi, suspend, multiple displays, Bluetooth, and proprietary drivers require
separate hardware testing. Bluetooth is not part of the default minimal bundle.

Local checks:

```sh
python3 scripts/check.py
shellcheck -S warning scripts/*.sh profiles/*.sh tests/*.sh
actionlint -config-file .github/actionlint.yaml .github/workflows/build.yml
python3 scripts/vm-test.py --arch x86_64 --mode bios \
  --iso out/images/x86_64/uflinux-0.1.0-x86_64.iso
```

The actionlint config recognizes the documented Ubuntu 26.04 labels because its
built-in runner list can lag GitHub's runner releases. It does not provision
self-hosted runners; the workflow continues to request GitHub-hosted runners.
