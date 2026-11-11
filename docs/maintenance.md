# Maintenance

Keep the Alpine branch on a currently supported release with both `main` and
`community` support. Alpine desktop packages largely live in `community`, whose
support ends earlier than `main`. Do not assume the kernel's support lifetime
also covers the desktop stack.

For a source update:

1. Select and review an Alpine stable release and aports commit. Obtain each
   architecture's official minirootfs and verify its published signatures/hashes.
2. Update the corresponding versions, archive URLs, SHA-256/SHA-512 values, aports
   commit, and source epoch in `config/sources.lock.json`. Avoid moving branches
   and tags as build inputs; record immutable commits and actual archive hashes.
3. Update the Ironbar APKBUILD version, commit URL, build directory, checksum, and
   desktop dependency when upgrading Ironbar. Review its Cargo lockfile, minimum
   Rust/GTK requirements, feature names, config schema, and labwc compatibility.
4. If changing Alpine branches, update the branch URLs in the UFLinux APKBUILD and
   overlay generator together with the lockfile and publishing path. Recheck the
   pinned mkimage compatibility patch against the new source.
5. Update `uflinux`'s package version and the distribution default version. Bump
   Ironbar's `pkgrel` for packaging, flags, or patch changes without an upstream
   version change; update the exact desktop dependency at the same time.
6. Run `python3 scripts/update-checksums.py` after changing local package assets,
   inspect its diff, run `scripts/check.py`, then use the full manual build matrix.
7. Review package/boot/desktop reports before creating the version tag.

Replace the reserved `noreply@example.invalid` packager identity with your real
maintainer identity before public releases. It is a syntactically valid packaging
default, not a monitored support address. Set the package project URL to your
actual repository as part of adopting the project.

Dependabot updates pinned GitHub Actions. The pinned actionlint Go source commit
is reviewed separately. Changes to runner images, QEMU, firmware, Alpine's kernel,
or Ironbar require the relevant tests to pass again; do not mark failed targets
as optional simply to produce a release.

## Signing keys

Back up the release private key securely outside both Git and Actions caches.
Never commit it or upload `.state`; that directory contains development keys and
temporarily contains the release key during signing. Published metadata includes
only public keys and key paths, not private PEM contents.

UFLinux signs with an **EC P-256** key. apk v2 has no ECDSA signature: apk-tools
3.0.9 recognises only the stream names `RSA512`, `RSA256`, `RSA` and `DSA`, and
selects a trusted key by its **filename** from a plain public-key PEM, so an
X.509 certificate is not accepted as a key. The EC signature is therefore carried
in the SHA-256 stream, and the stream name `RSA256` describes that digest, not the
algorithm. `abuild-sign` supports this directly: it accepts any OpenSSL-capable
private key under `-t RSA256`.

The signing key uses the stable filename `uflinux.ec.pub`; apk v2 signatures refer
to that filename. Keep the same key across routine releases.
For a planned rotation, distribute the replacement public key to installed systems
through an independently authenticated channel before switching the signing
secret. Replacing an already trusted key is an administrator operation; this
version does not implement unattended key rollover or dual-signature migration.

Retained old-generation APKs must also be verified with the old key and re-signed
with the replacement key before combining them into the new repository. The
publisher deliberately fails if those signatures no longer match; it does not
silently trust an old snapshot's bundled key. Keep the prior key and snapshot
available during that controlled migration. For an unexpected compromise, revoke
the old key through your authenticated release channel and rebuild affected
packages rather than merely re-signing unknown binaries.

## Reproducibility and regression review

Use the same source commit, source epoch, rootfs, source archives, Cargo lockfile,
toolchain APKs, runtime APKs, build paths, and release flags for comparison.
Development signature streams differ because development keys are ephemeral.
`builder.unsigned_digest()` compares APK control and payload archives with the
signature streams excluded. It still includes control metadata and timestamps;
matching only the executable is a weaker check.

Compare repeat-build hashes before declaring reproducible releases. Record the
baseline ISO size, stripped Ironbar size, boot time, and desktop RSS from attached
records. No unmeasured minimum-memory or performance claim is implied by the
word "lightweight".

Settings changes require `cargo fmt` and frozen workspace tests. The build
calculates the local source checksum; update the Settings version with
`scripts/settings-source.py --update` when needed.
Check Ironbar's patch against the pinned upstream revision before upgrading it;
keep its originating-monitor environment test. Do not re-enable the removed
NetworkManager/volume modules while using the combined Settings backend.

ConnMan is also a maintained custom package. Track Alpine's ConnMan security
updates and upstream releases, update its locked source/hash and package revision,
and rebase the small iwd credential-delegation patch before release. Its private
driver flag must remain confined to iwd; do not bypass authentication or CA/server
verification. Keep the package's D-Bus rules: group netdev may control ConnMan but
may not mutate `net.connman.Clock`, and the deny has to stay inside the netdev
policy because D-Bus applies console and context policies in a fixed order.
Re-run the real PEAP, TTLS and TLS tests, including server-domain
mismatch rejection, before publishing a changed network stack. The desktop pins
this tested revision so an unrelated official package cannot silently drop the fix.

The x86_64 native VM job exercises simulated Wi-Fi and ConnMan SNTP before disk
installation. Both x86_64 firmware modes reboot the installed desktop with
512 MiB and no swap. Publication requires these results for the exact ISO hash,
in addition to all seven package/desktop and architecture-specific boot reports.
Review PSS, idle CPU, mapped-frame latency, binary/ISO size and physical hardware
coverage separately. The original development ISO's historical record is not a
substitute for validating a changed desktop stack.
