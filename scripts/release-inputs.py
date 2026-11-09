#!/usr/bin/env python3
"""Stage one architecture's rebuild inputs without collecting all targets on one runner."""
import argparse
from pathlib import Path

from builder import ARCHES, PACKAGE_NAMES, link_or_copy, sha

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--arch", choices=ARCHES, required=True)
parser.add_argument("--version", required=True)
args = parser.parse_args()
out = Path("out")
directory = out / "inputs" / args.arch
inputs = sorted(directory.rglob("*.tar.gz"))
if (directory / "build-apks.tar.gz").exists():
    expected = {"build-apks.tar.gz", "image-apks.tar.gz", "cargo-sources.tar.gz", "sources.tar.gz"}
else:
    expected = {"image-apks.tar.gz"}
    for origin in PACKAGE_NAMES:
        expected.update({origin + "/build-apks.tar.gz", origin + "/sources.tar.gz"})
        if origin in {"ironbar", "uflinux-regreet", "uflinux-settings"}:
            expected.add(origin + "/cargo-sources.tar.gz")
    # Non-Rust jobs may retain an existing Cargo cache during local builds.
    expected.update(str(p.relative_to(directory)) for p in inputs
                    if p.name == "cargo-sources.tar.gz" and p.parent.name in PACKAGE_NAMES)
if {str(p.relative_to(directory)) for p in inputs} != expected:
    raise RuntimeError("The complete rebuild input set is required")
checksums = []
for source in inputs:
    if source.stat().st_size >= 2 * 1024**3:
        raise ValueError("A rebuild input exceeds GitHub Releases' per-file size limit")
    name = "-".join(source.relative_to(directory).parts)
    target = out / f"uflinux-{args.version}-{args.arch}-{name}"
    link_or_copy(source, target)
    checksums.append(sha(target) + "  " + target.name + "\n")
(out / f"SHA256SUMS-{args.arch}-inputs").write_text("".join(checksums))
