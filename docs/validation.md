# Local validation record

This is the historical pre-Settings development ISO. Current ConnMan+iwd/GTK4
work and its separate artifacts are recorded in [Settings validation](settings-validation.md).

The initial development build was checked on 8 November 2026. These results
describe **x86_64 only**. The other seven architectures have a configured CI
pipeline and dependency availability checks, but have not been compiled or
boot-tested locally. GitHub publication and a real public repository upgrade
have not been exercised. A full successful matrix is required before release.

| Check | Result |
| --- | --- |
| Repository tests | 24 passed |
| Ironbar upstream tests | 30 passed with the selected features |
| ShellCheck and actionlint | Passed |
| APKBUILD | Built with Alpine abuild; runtime, debug, documentation, and distro subpackages installed successfully |
| SPDX 2.3 inventory | Validated against the official JSON schema |
| Fresh runtime root | Signed package installation, ELF architecture/PIE, shared libraries, headless desktop, audio volume, window controls, missing battery behavior passed |
| Configuration preservation | Modified system defaults survived package reinstallation |
| Signing | Resigning preserved payloads; damaged archives and mismatched keys rejected; temporary private keys removed on success and failure |
| Repeat build | All ten custom APK control/data payloads matched after a fresh compiler rebuild; signatures excluded |
| BIOS and UEFI | Actual ISO boot, desktop startup, XFS installation and installed-system reboot passed |
| Installation isolation | Complete unrelated partition hash and partition table unchanged; partition hash checked again after reboot |

The final ISO is `out/images/x86_64/uflinux-0.1.0-x86_64.iso`,
918,552,576 bytes, SHA-256:

```text
7b1e22f21357e560c513aa07937662db60de0c9803f9cedc1f6e6369cd382992
```

With QEMU software emulation, two virtual CPUs, 2 GiB RAM and virtio graphics,
time to the live serial login prompt was 25.47 seconds with BIOS and 35.40 seconds
with UEFI. These are local measurements, not hardware performance guarantees.
The stripped Ironbar executable was 7,253,888 bytes. A separate headless desktop
sample measured labwc at 71,628 KiB RSS, Ironbar at 90,688 KiB, and Foot at
8,764 KiB. These are process RSS measurements, not total system memory use.

Machine-readable results, console logs, desktop screenshots, package inventories,
and source/input archives are retained under `out/`. Build caches remain under
`.cache/build`. Generated outputs and caches are excluded from Git.

This ISO uses a **development signing key** and the example update URL
`https://example.github.io/uflinux/apk/v3.24/uflinux`. Rebuild with your actual
repository URL and persistent signing key before distributing a release. Follow
[GitHub setup](github.md) to configure secrets and Pages. No release has been
published from this workspace. Physical GPU/Wi-Fi/suspend coverage, other CPU
architectures, and byte-identical complete ISO rebuilds remain unverified.
