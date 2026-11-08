# Parallel package build validation

Local x86_64 checks completed on 8 November 2026. These exercise the package-job
split; they are separate from the earlier development ISO's boot records.

- Four independent package commands ran concurrently, with separate roots, caches
  and development signing keys. They produced all 21 expected APKs.
- Each origin passed a second build with a fresh compiler target cache. Collection
  checked its APK hashes and combined all four matching payload comparisons.
- Signing verified all 21 incoming signatures against their originating keys and
  re-signed the archives successfully. The disposable release-key test rejected
  mismatched keys and an APK paired with another origin's public key. Temporary
  private keys were removed on success and failure.
- Fresh-root verification installed the combined package set and passed the real
  headless labwc, Ironbar and Settings suite, ELF/shared-library checks, damaged
  signature rejection and configuration preservation checks.
- Origin-specific rebuild input archives produced distinct release asset names.
  The local warm-cache run retained 13 archives because the two non-Rust caches
  also contained Cargo inputs; cold CI builds normally retain 11.
- All 38 Python tests, ShellCheck, actionlint and whitespace checks passed. Package
  checks included 31 Ironbar tests, 21 Rust workspace tests and six ConnMan tests.

Logs are in `.cache/parallel-*.log`. Machine-readable package, signature,
reproducibility and desktop reports are in `.cache/parallel-out/`.

These were native local builds with retained dependency caches. They establish
that the four jobs can execute independently and combine correctly; they are
not cold-runner speed benchmarks. The updated GitHub workflow and seven other
architectures still need execution on GitHub. Actual concurrency depends on
available runner slots, with a workflow maximum of eight compile jobs.

## QEMU dependency privilege fix

On 8 November 2026, a local x86_64 host built the aarch64 ConnMan shard through
QEMU user emulation. Dependencies were installed by the root chroot using
`SUDO_APK=apk abuild -F deps`; plain `abuild` ran as uid 1000, without a setuid
privilege transition. All six ConnMan tests passed and all five expected APKs
were produced. The runtime APK signature verified and its executable was
confirmed as AArch64. Root dependency cleanup completed successfully.

The regression suite passed 40 Python tests, including cleanup after compiler
failure. Actionlint, ShellCheck for the build entry point, and whitespace checks
passed. Local logs are in `.cache/emulated-fix.log`, with APKs and input records
under `.cache/emulated-fix-out/`. This validates the shared package execution
path under emulation; the reported armv7 GitHub job and the complete architecture
matrix still require rerunning with this change. No ISO was rebuilt for this fix.
