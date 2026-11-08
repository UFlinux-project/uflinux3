#!/usr/bin/env python3
"""Stage one architecture's rebuild inputs without collecting all targets on one runner."""
import argparse
from pathlib import Path

from builder import ARCHES, link_or_copy, sha

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--arch", choices=ARCHES, required=True)
parser.add_argument("--version", required=True)
args = parser.parse_args()
out = Path("out")
inputs = sorted((out / "inputs" / args.arch).glob("*.tar.gz"))
if {p.name for p in inputs} != {"build-apks.tar.gz", "image-apks.tar.gz", "cargo-sources.tar.gz", "sources.tar.gz"}:
    raise RuntimeError("The complete rebuild input set is required")
checksums = []
for source in inputs:
    if source.stat().st_size >= 2 * 1024**3:
        raise ValueError("A rebuild input exceeds GitHub Releases' per-file size limit")
    target = out / f"uflinux-{args.version}-{args.arch}-{source.name}"
    link_or_copy(source, target)
    checksums.append(sha(target) + "  " + target.name + "\n")
(out / f"SHA256SUMS-{args.arch}-inputs").write_text("".join(checksums))
