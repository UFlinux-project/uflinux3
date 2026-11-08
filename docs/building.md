# Building

Use a disposable Linux VM with Python 3.11+, GNU tar, gzip, coreutils, util-linux,
network access, root privileges, and at least 10 GiB free storage. More space is
useful when retaining several builds. QEMU user emulation and binfmt_misc with the
`F` flag are required for nonnative architectures. x86 uses the x86_64 kernel's
32-bit compatibility support. aarch64 builds use native ARM64 GitHub runners.

On the configured Ubuntu runners, install `qemu-user-binfmt binfmt-support` and run:

```sh
python3 scripts/preflight.py --arch ppc64le
```

`config/sources.lock.json` pins minirootfs hashes, aports and Ironbar commit archive
hashes, feature choices, and the default source epoch. Downloaded files are checked
again on every reuse. Alpine's official package repositories remain on the named
stable branch so security revisions can be incorporated. Their resolved package
versions and hashes are recorded; a pinned minirootfs does not freeze those repos.

```sh
export APK_REPOSITORY_URL=https://OWNER.github.io/REPO/apk/v3.24/uflinux
sudo --preserve-env=APK_REPOSITORY_URL bash scripts/build.sh --arch x86_64
```

`--stage` accepts `bootstrap`, `packages`, `sign`, `image`, `verify`, or `all`.
Stages share `--workdir` (default `.state`) and `--outdir` (default `out`). Download,
APK, Cargo, and compiler caches use `--cache` (default `.cache/build`). A lockfile
change requires a fresh work directory, but verified downloads can be reused.

Twenty-one custom APKs are built: patched ConnMan (runtime/OpenRC/dev/docs/debug),
Ironbar, Settings components and the distribution packages. ConnMan's pinned
upstream source and Alpine compatibility patches are retained with build inputs.

- `packages`: unprivileged `abuild -r`, locked Cargo fetch, frozen release build
  and tests, development signatures, dependency records, and input archives.
- `sign`: verifies the incoming development signatures, replaces only the APK v2
  signature stream, signs indexes, and verifies the resulting packages.
- `image`: uses Alpine mkimage, a custom profile, an APK cache, and a reproducible
  configuration overlay. The small apk-tools 3 compatibility patch is in `patches`.
- `verify`: installs the custom packages through apk, validates Ironbar's actual
  configuration, tests a headless desktop and toplevel controls, and exercises the
  installer safeguards. Firmware boot and disk installation are separate VM jobs.

The chroot exposes proc and selected device nodes, but not host sysfs. It is a
build environment, not a security boundary for hostile source code. GitHub uses
fresh disposable VMs and never supplies release private keys to compilation.

Outputs:

| Directory | Contents |
| --- | --- |
| `out/images/ARCH` | ISO and SHA-256/SHA-512 sidecars |
| `out/packages/ARCH` | Build-signed APKs and their public key |
| `out/repository/ARCH` | Final signed APKs and APKINDEX |
| `out/metadata/ARCH` | Source/build records, hashes, package lists, SPDX inventory |
| `out/checks/ARCH` | Headless desktop logs, screenshot, memory, validation result |
| `out/vm/ARCH/MODE` | Actual ISO/install/reboot console logs and result |
| `out/inputs/ARCH` | Compressed package, source, and Cargo inputs for rebuilding |

Release input archives retain build APK caches, Cargo registry sources/indexes,
verified upstream source archives, and image APK caches. Compiler targets are
cached in Actions rather than distributed. To reconstruct an older build, use
its source commit and recorded epoch, unpack the input caches, and resolve the
recorded APK revisions from those saved packages. If mirrors have removed those
revisions, create a local signed repository from the retained APKs and explicitly
use it in the build environment. The default command still resolves current
packages from the stable branch; it is not an automatic offline replay mode.

The SPDX file inventories APKs physically bundled in the ISO, including packages
available for installation but not automatically installed at live boot. It is
not a claim that every cached package is running. Build/verify environment package
lists also contain build or test tools and are labeled by stage.

Original APK license annotations are retained in SPDX `licenseComments`.
`licenseDeclared` and `licenseConcluded` use `NOASSERTION`: Alpine annotations can
contain non-SPDX strings or omit boolean operators, and this inventory does not
guess how to reinterpret them.

The native CI job also rebuilds with a fresh compiler target directory and compares
every custom APK's control and data streams, excluding signature streams. Publication
requires that report to match the exact native package set being published.

The build sets `SOURCE_DATE_EPOCH`, UTC, stable paths, normalized tar metadata,
and gzip without timestamps. GitHub uses the source commit time. These controls
help reproducibility; byte-for-byte reproducibility is not claimed until two
clean builds with identical resolved packages have been compared.

Ironbar uses the locked feature list, `opt-level=s`, ThinLTO, eight codegen units,
abort-on-panic, and dynamically linked Alpine libraries. Debug symbols are split
from the runtime APK. There is no `target-cpu=native` or host instruction-set tuning.

Settings uses the same release optimization and hardening practices. Its four
Rust components share `settings/Cargo.lock`. Only the UI links GTK4; the session
service and privileged helper do not. `cargo auditable` records dependency data.
The six Settings APKs split GUI, daemon/CLI, system helper, OpenRC, documentation
and debug symbols. Ironbar omits its NetworkManager and volume modules and carries
the checksum-pinned `IRONBAR_OUTPUT` child-environment patch.

After editing Settings source or data, run:

```sh
cargo fmt --manifest-path settings/Cargo.toml --all
python3 scripts/settings-source.py --update
python3 scripts/check.py
```

The source bundle is a sorted, uncompressed USTAR archive with normalized headers;
its SHA-512 is locked in both the manifest and APKBUILD. `--check` never updates it.
Build preparation removes only project-crate compiler artifacts to avoid stale
Cargo fingerprints from normalized source timestamps; dependency caches survive.
CI cache keys track upstream inputs, build settings and Cargo locks independently
of the changing project-source hash. Static checks need Python 3.11; compiling
Settings requires Alpine's GTK4/layer-shell, PulseAudio client, polkit-agent,
elogind, Vulkan/EGL and GLib development packages listed in its APKBUILD.

The Power panel uses Alpine's TLP 1.10 series, with the three-profile interface
introduced in 1.9. Power packages are checked for all eight architectures. Tests
run the actual BusyBox ACPI parser in a private chroot with synthetic input events;
the installed x86_64 VM verifies real TLP transitions, shared Ironbar status and
physical-key Do nothing/Shut down behavior. TLP is never executed on the build
host. The 512 MiB VM uses an interpreter installed from signed media solely for
measurement; Python is not an installed-desktop dependency.

To preserve an older development ISO, choose a separate `--outdir out-settings`
and fresh `--workdir .state/settings`. Do not reuse results after changing inputs.
