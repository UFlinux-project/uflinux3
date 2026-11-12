#!/usr/bin/env python3
import json
import hashlib
import os
from pathlib import Path
import re
import subprocess

root = Path(__file__).resolve().parents[1]
arches = json.loads((root / "config/architectures.json").read_text())
lock = json.loads((root / "config/sources.lock.json").read_text())
def configuration(event, ref):
    release = event == "push" and ref.startswith("refs/tags/v")
    version = ref.removeprefix("refs/tags/v") if release else lock["distribution"]["version"]
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:_(?:alpha|beta|rc)[0-9]+)?", version):
        raise ValueError("Tag must be v followed by an Alpine-compatible version")
    # Ordinary commits build images and boot them in QEMU on every architecture
    # with a reachable emulator on the x86 runner; the ARM targets stay on
    # releases and manual dispatches.
    selected = list(arches) if release or event == "workflow_dispatch" else ["x86_64", "ppc64le", "riscv64", "s390x"]
    matrix = {"include": [{"arch": arch, "runner": arches[arch]["runner"]} for arch in selected]}
    packages = {"include": [{"arch": arch, "runner": arches[arch]["runner"], "package": package}
                             for arch in selected for package in ("connman", "gpu-screen-recorder", "uflinux-regreet", "uflinux-settings", "uflinux")]}
    vm = {"include": [{"arch": arch, "mode": mode} for arch in selected for mode in (
        ["bios", "uefi"] if arch in {"x86", "x86_64"} else ["native" if arch in {"ppc64le", "s390x"} else "uefi"])]}
    return {"matrix": matrix, "packages": packages, "vm": vm, "version": version, "release": str(release).lower()}

def main():
    epoch = subprocess.check_output(["git", "log", "-1", "--format=%ct"], text=True).strip()
    # Cache dependencies across changes to our deterministic source bundle. Cargo's
    # fingerprints and package-scoped clean in prepare() still rebuild project code.
    dependencies = hashlib.sha256(json.dumps({k: lock[k] for k in ("alpine", "rootfs", "sources")}, sort_keys=True).encode())
    for name in ("settings/Cargo.lock", "settings/Cargo.toml", "aports/connman/APKBUILD", "aports/gpu-screen-recorder/APKBUILD", "aports/uflinux-regreet/APKBUILD", "aports/uflinux-settings/APKBUILD"):
        dependencies.update((root / name).read_bytes().split(b'sha512sums=')[0])
    with open(os.environ["GITHUB_OUTPUT"], "a") as output:
        for key, value in {**configuration(os.environ.get("EVENT", ""),os.environ.get("REF", "")),
                           "epoch": epoch, "cache": dependencies.hexdigest()}.items():
            output.write(f"{key}={json.dumps(value) if isinstance(value,dict) else value}\n")


if __name__ == "__main__":
    main()
