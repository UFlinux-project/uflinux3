#!/usr/bin/env python3
"""Compare APK payloads against a second build with a fresh compiler target cache."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

from builder import ARCHES, PROJECT, unsigned_digest, remove_tree

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--arch", choices=ARCHES, required=True)
parser.add_argument("--version", required=True)
parser.add_argument("--outdir", default="out")
args = parser.parse_args()
if os.geteuid() != 0:
    parser.error("Run as root on the disposable build host")
out = Path(args.outdir).resolve()
baseline = {p.name: unsigned_digest(p) for p in (out / "packages" / args.arch).glob("*.apk")}
if not baseline:
    raise RuntimeError("The baseline packages stage must complete first")
cache = PROJECT / ".cache/reproducibility"
if cache.exists():
    remove_tree(cache)
cache.mkdir(parents=True)
original = PROJECT / ".cache/build"
for name in ("downloads", args.arch + "/apk", args.arch + "/cargo"):
    if (original / name).exists():
        shutil.copytree(original / name, cache / name)
work = PROJECT / ".state/reproducibility"
if work.exists():
    remove_tree(work)
repeat = PROJECT / ".cache/reproducibility-output"
if repeat.exists():
    remove_tree(repeat)
try:
    subprocess.run([sys.executable, str(PROJECT / "scripts/builder.py"), "--arch", args.arch,
                    "--version", args.version, "--outdir", str(repeat), "--cache", str(cache),
                    "--workdir", str(work), "--stage", "packages"], check=True)
    rebuilt = {p.name: unsigned_digest(p) for p in (repeat / "packages" / args.arch).glob("*.apk")}
    report = {"arch": args.arch, "version": args.version, "fresh_compiler_target": True,
              "signature_streams_excluded": True, "baseline": baseline, "rebuilt": rebuilt,
              "matched": baseline == rebuilt}
    (out / "metadata" / args.arch / "reproducibility.json").write_text(json.dumps(report, indent=2) + "\n")
    if baseline != rebuilt:
        raise RuntimeError("Clean compiler rebuild changed unsigned APK contents; inspect reproducibility.json")
finally:
    # No child builds remain when builder.py returns, and its mounts are released.
    for path in (work, repeat, cache):
        if path.exists():
            remove_tree(path)
