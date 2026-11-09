# GitHub setup

1. Push this repository to a **public** GitHub repository with default branch `main`.
2. Enable Actions and choose **GitHub Actions** as the Pages source.
3. Configure the `github-pages` environment to allow deployment from version tags
   such as `v*`; its default branch restrictions may otherwise reject tag builds.
4. Generate a persistent RSA key outside this repository:

   ```sh
   install -d -m 700 "$HOME/.config/uflinux"
   openssl genrsa -out "$HOME/.config/uflinux/uflinux.rsa" 3072
   chmod 600 "$HOME/.config/uflinux/uflinux.rsa"
   openssl rsa -in "$HOME/.config/uflinux/uflinux.rsa" -pubout \
     -out "$HOME/.config/uflinux/uflinux.rsa.pub"
   ```

5. Add two Actions secrets containing the **PEM file contents**, not paths or
   base64: `APK_SIGNING_PRIVATE_KEY` and `APK_SIGNING_PUBLIC_KEY`.
6. Optionally set repository variable `APK_REPOSITORY_URL` for a custom Pages
   domain. Otherwise the workflow derives
   `https://OWNER.github.io/REPO/apk/v3.24/uflinux` (and handles account-site repos).
7. Run the workflow manually to inspect development artifacts. Then tag a release:

   ```sh
   git tag v0.1.0
   git push origin v0.1.0
   ```

Pull requests and main pushes build x86_64; manual runs and version tags use all
seven architectures.
Only push events for valid version tags can publish. Manual runs never become
release builds by supplying a private key. Protect the default branch and release
tags according to the maintainer access you intend to grant.

Compilation uses five independent package-origin jobs per architecture, with
separate caches and development keys and a maximum of eight concurrent jobs.
The image job checks completeness, build identity, archive hashes and native
reproducibility reports before signing. It downloads compile artifacts directly
to avoid an extra upload/download of the combined package and cache archives. Release input assets retain
origin-specific cache archives. Images and VM tests remain separate matrix jobs.
Compilation is interrupted
before the six-hour job limit so caches can be saved. Emulated cold builds can be
slow: an incomplete architecture fails the release instead of silently disappearing
from the matrix. Rerun failed jobs after examining their logs.

Release keys are passed only to signing/assembly and repository-index steps.
Private key files are removed when those commands finish, including failures.
Builds use isolated development keys without release secrets. Only public keys,
APKs, images, caches of public inputs, and test/build records are uploaded.

Publication is serialized. It requires package/desktop verification for every
architecture and actual ISO installation/reboot reports for every configured
firmware mode. It first prepares a draft release, deploys the complete signed
Pages repository, and publishes the draft after deployment succeeds. If deployment
fails, the release remains a draft and can be retried. A fresh client verifies the
signed Pages layout before deployment, including a defaults-package upgrade from
the oldest retained release when one exists. After deployment, the same checks
run against the public HTTPS repository before the draft becomes public. The
first release records that no earlier version exists. The initial public key
is pinned to the verified image key, rather than trusted from the network alone.
Pages and Releases are
separate services; this is not a cross-service atomic transaction.

Rebuild inputs are attested and uploaded to the unpublished draft by architecture,
so a publication runner does not have to hold all architectures' input caches at
once. The final SHA256SUMS includes those assets. ISOs and the final APK repository
are published only after every input upload and verification job succeeds.

The repository keeps the current generation and two previous generations of
custom APKs. A snapshot in each release supplies the next publication's history.
Upstream packages and ISOs are not hosted on Pages. Publishing refuses a Pages
payload over 900 MiB and an individual release asset at or above 2 GiB.

Input archives and multiarchitecture compiler caches consume significant artifact
storage. Review your account's storage usage and retention settings. Per-run
compiler cache keys allow saving progress after a failed or interrupted build.

To verify downloads:

```sh
sha256sum -c SHA256SUMS
gh attestation verify uflinux-0.1.0-x86_64.iso --repo OWNER/REPO
gh attestation verify uflinux-apk-repository.tar.gz --repo OWNER/REPO
```

Download the assets named in SHA256SUMS before checking it. The repository snapshot
contains `uflinux.rsa.pub`; install it under the same name in `/etc/apk/keys` when
manually configuring a client. Obtain and verify the initial key through the
authenticated release, rather than treating a checksum downloaded beside a file
as independent authentication.
