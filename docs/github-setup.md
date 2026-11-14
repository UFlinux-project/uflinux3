# UFLinux repository setup

Repository: https://github.com/UFlinux-project/uflinux3

The build infers this APK URL from `GITHUB_REPOSITORY`:
`https://uflinux-project.github.io/uflinux3/apk/v3.24/uflinux`.
An `APK_REPOSITORY_URL` Actions variable can override it.

A persistent EC P-256 signing keypair is prepared locally in
`$HOME/.config/uflinux3-signing/uflinux.ec` and
`$HOME/.config/uflinux3-signing/uflinux.ec.pub`, following the commands in
[github.md](github.md). apk v2 has no ECDSA signature type, so the key is carried
in the SHA-256 signature stream; see [maintenance.md](maintenance.md).
The directory and private key are restricted to their owner and kept outside the build workspace and excluded from Git.
Back up the private key securely before publishing a release; do not commit it or
paste it into issues, logs or chat.

In the repository's Settings → Secrets and variables → Actions, add:

- `APK_SIGNING_PRIVATE_KEY`: complete contents of `$HOME/.config/uflinux3-signing/uflinux.ec`.
- `APK_SIGNING_PUBLIC_KEY`: complete contents of `$HOME/.config/uflinux3-signing/uflinux.ec.pub`.

In Settings → Pages, choose **GitHub Actions** as the build/deployment source.
Both secrets have to hold the EC key above; a previously uploaded RSA pair is
rejected by the build rather than silently accepted. The maintainer confirmed the
signing secrets and Pages source were configured.
The connected GitHub integration can inspect this repository, but its source-write
request returned HTTP 403 (Resource not accessible by integration). Publishing
source currently requires a maintainer-authenticated Git push. The integration
also cannot configure secrets, configure Pages or create release tags.

Pushes to main and manual dispatch build development images without release
secrets. A version tag builds the release package set and exposes the key only to
trusted signing/assembly/publication jobs. Publication is gated on all six
architecture checks and native integration/reproducibility records. Pages keeps
three release generations and checks the complete bundle's size before deployment.
See [the full publishing guide](github.md) for key rotation, retention and rollback.
