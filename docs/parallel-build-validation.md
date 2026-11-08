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
