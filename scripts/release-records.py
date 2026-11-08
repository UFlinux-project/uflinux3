#!/usr/bin/env python3
"""Archive audit records and regenerate the checksums for all release assets."""
import argparse
from pathlib import Path
import subprocess
import os

from builder import LOCK, sha

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--version", required=True)
args = parser.parse_args()
out = Path("out")
epoch = os.environ.get("SOURCE_DATE_EPOCH", str(LOCK["source_date_epoch"]))
archive = out / "uflinux-build-records.tar.gz"
tar = subprocess.Popen(["tar", "--sort=name", "--mtime=@" + epoch, "--owner=0", "--group=0", "--numeric-owner",
                        "-C", "out", "-cf", "-", "metadata", "checks", "vm"], stdout=subprocess.PIPE)
with archive.open("wb") as stream:
    subprocess.run(["gzip", "-9n"], stdin=tar.stdout, stdout=stream, check=True)
tar.stdout.close()
if tar.wait():
    raise RuntimeError("Failed to create build record archive")
assets = [*out.glob("images/*/*.iso"), *out.glob("uflinux-*.tar.gz"), *out.glob("metadata/*/*.spdx.json")]
if len({p.name for p in assets}) != len(assets):
    raise ValueError("Release asset basenames must be unique")
if any(p.stat().st_size >= 2 * 1024**3 for p in assets):
    raise ValueError("An asset exceeds GitHub Releases' per-file size limit")
checksums = "".join(sha(p) + "  " + p.name + "\n" for p in sorted(assets))
for manifest in sorted(out.glob("SHA256SUMS-*-inputs")):
    for line in manifest.read_text().splitlines():
        digest, name = line.split("  ", 1)
        if len(digest) != 64 or any(c not in "0123456789abcdef" for c in digest) or Path(name).name != name:
            raise ValueError("Invalid input checksum manifest")
        checksums += line + "\n"
(out / "SHA256SUMS").write_text(checksums)
