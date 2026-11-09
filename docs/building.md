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

GitHub compiles four independent origins per architecture: `connman`, `ironbar`,
`uflinux-settings` and `uflinux`. PR/main builds schedule four compile jobs;
manual/tag runs schedule 28, with at most eight compile jobs running together.
Each origin has its own VM, development signing key, timeout and cache. Heavy Rust
builds and their native clean-rebuild comparisons run independently. Signing and
ISO assembly wait for a complete validated set of 22 APKs.

For an individual package build, use the same entry point:

```sh
sudo --preserve-env=APK_REPOSITORY_URL bash scripts/build.sh --arch x86_64 \
  --stage packages --package ironbar
sudo --preserve-env=APK_REPOSITORY_URL python3 scripts/reproducibility.py \
  --arch x86_64 --version 0.1.0 --package ironbar
```

Repeat independently for the other three origins. `--package` automatically uses
`.state/ORIGIN` and `.cache/build/ORIGIN`, writing below
`out/package-shards/ARCH/ORIGIN`. Explicit work/cache directory overrides must
remain distinct for concurrent builds. Shards support only `packages`/`bootstrap`
and never receive release keys. The ordinary `--stage all` command retains its
existing sequential local-build behavior.

After every origin succeeds, collect their outputs before running `sign`, `image`
and `verify` as usual:

```sh
python3 scripts/merge-packages.py --arch x86_64 --version 0.1.0
```

Collection rejects missing APKs, stale hashes, mixed source commits, different
build settings and incomplete native reproducibility proofs. It retains a public
key per APK; the signing stage verifies each original signature against that key
before replacing its signature stream. Development private keys are not uploaded.
Each native origin must have its clean-rebuild report before collection succeeds.
See [local parallel-build validation](parallel-build-validation.md) for completed
checks and their scope.

Twenty-two custom APKs are built: patched ConnMan (runtime/OpenRC/dev/docs/debug),
Ironbar, Settings components and the distribution packages. ConnMan's pinned
upstream source and Alpine compatibility patches are retained with build inputs.

- `packages`: root installs recipe dependencies with `SUDO_APK=apk abuild -F deps`;
  the unprivileged builder runs plain `abuild`, including dependency validation,
  locked Cargo fetch, frozen release build and tests, and development signing.
  Root removes temporary dependencies afterward, including after build failures.
  This avoids target setuid helpers, which cannot elevate under QEMU user
  emulation. Dependency records and input archives are retained.
- `sign`: verifies the incoming development signatures, replaces only the APK v2
  signature stream, signs indexes, and verifies the resulting packages.
- `image`: uses Alpine mkimage, a custom profile, an APK cache, and a reproducible
  configuration overlay. Initramfs and the signed modloop use Zstandard level 19;
  SquashFS uses 128 KiB blocks without XZ-specific BCJ filters. An image-only copy
  of Alpine’s `update-kernel` selects these formats and requires kernel Zstandard
  support. The installed upstream helper remains intact. Both image formats and
  the initramfs stream are checked after ISO extraction. Compression settings and
  the helper patch checksum are recorded in the source lock. The apk-tools 3 and
  compression patches are in `patches`; see [compression details](compression.md).
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
| `out/package-shards/ARCH/ORIGIN` | Isolated compile outputs before collection |
| `out/packages/ARCH` | Build-signed APKs and their public key |
| `out/repository/ARCH` | Final signed APKs and APKINDEX |
| `out/metadata/ARCH` | Source/build records, hashes, package lists, SPDX inventory |
| `out/checks/ARCH` | Headless desktop logs, screenshot, memory, validation result |
| `out/vm/ARCH/MODE` | Actual ISO/install/reboot console logs and result |
| `out/inputs/ARCH` | Compressed package, source, and Cargo inputs for rebuilding |

Release input archives retain build APK caches, Cargo registry sources/indexes,
verified upstream source archives, and image APK caches. Compiler targets are
cached in Actions rather than distributed. Parallel builds retain build APKs,
Cargo inputs and sources separately under `out/inputs/ARCH/ORIGIN`; release asset
names include that origin (for example `uflinux-0.1.0-x86_64-ironbar-sources.tar.gz`).
Restore each archive into that origin's cache namespace; image APK inputs remain
shared at architecture level. This keeps exact input sets from independent jobs
without overwriting indexes or Cargo metadata. Sequential builds retain the
original four-archive format. To reconstruct an older build, use
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

The native CI jobs also rebuild each origin with a fresh compiler target directory and compare
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
python3 scripts/check.py
```

The source bundle is a sorted, uncompressed USTAR archive with normalized headers;
the build calculates its SHA-512 and inserts it into the staged APKBUILD. Run
`python3 scripts/settings-source.py --update` when changing the Settings version.
Build preparation removes only project-crate compiler artifacts to avoid stale
Cargo fingerprints from normalized source timestamps; dependency caches survive.
CI cache keys track upstream inputs, build settings and Cargo locks independently
of the changing project-source hash. Static checks need Python 3.11; compiling
Settings requires Alpine's GTK4/layer-shell, PulseAudio client, polkit-agent,
elogind, Vulkan/EGL and GLib development packages listed in its APKBUILD.

The Power panel uses Alpine's TLP 1.10 series, with the three-profile interface
introduced in 1.9. Power packages are checked for all seven architectures. Tests
run the actual BusyBox ACPI parser in a private chroot with synthetic input events;
the installed x86_64 VM verifies real TLP transitions, shared Ironbar status and
physical-key Do nothing/Shut down behavior. TLP is never executed on the build
host. Live VM tests allocate 4096 MiB (3072 MiB for armv7, whose QEMU machine
disables highmem and limits RAM to 3 GiB). The diskless root unpacks the desktop and
full Papirus icon theme into tmpfs, whose default capacity is half of RAM. This
is separate from the installed reference target; a 2048 MiB live VM exhausted
its root filesystem during boot after the artwork/theme addition. The runner
rejects boot-time APK database errors and logs root byte/inode capacity.
The installed x86_64 test still uses 512 MiB, one display and no swap.
The 512 MiB VM uses an interpreter installed from signed media solely for
measurement; Python is not an installed-desktop dependency.

To preserve an older development ISO, choose a separate `--outdir out-settings`
and fresh `--workdir .state/settings`. Do not reuse results after changing inputs.

OpenRC starts independent services in parallel for both live and installed
systems. `uflinux-defaults` supplies `rc_parallel="YES"` in
`/etc/rc.conf.d/uflinux.conf`; dependencies still determine service ordering.
Edit this file to set `rc_parallel="NO"` when diagnosing boot issues. APK preserves
local edits during upgrades. This setting takes effect on the next boot.
