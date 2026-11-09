#!/usr/bin/env python3
"""Check declared desktop/installer dependencies against every target repository."""
import concurrent.futures
import io
import json
from pathlib import Path
import re
import shlex
import tarfile
import urllib.request

root = Path(__file__).resolve().parents[1]
lock = json.loads((root / "config/sources.lock.json").read_text())
text = (root / "aports/uflinux/APKBUILD").read_text()
required = set()
for block in re.findall(r'\bdepends="([^"]*)"', text):
    for item in shlex.split(block):
        name = re.split(r"[=<>~]", item)[0]
        if not name.startswith(("uflinux", "$")) and name != "ironbar":
            required.add(name)
required.update({"linux-lts", "linux-firmware", "xfsprogs", "alpine-conf", "rust", "cargo", "cargo-auditable",
                 "gtk4.0-dev", "gtk4-layer-shell-dev", "pulseaudio-dev", "openssl-dev", "abuild"})
required.update({"polkit-dev", "eudev-dev", "vulkan-loader-dev", "mesa-dev", "pkgconf",
                 "ddcutil", "tzdata", "vulkan-loader", "mesa-vulkan-ati"})
required.update({"rsvg-convert"})
required.update({"tlp", "tlp-openrc", "tlp-udev"})
required.update({"py3-atspi", "wtype", "gcc", "musl-dev", "wayland-dev", "libxkbcommon-dev", "desktop-file-utils", "wlr-randr"})
required.update({"autoconf", "automake", "dbus-dev", "glib-dev", "libmnl-dev", "libnftnl-dev", "libtool", "readline-dev", "linux-headers"})


def check(arch):
    found = set()
    versions = {}
    for repository in ("main", "community"):
        url = f"{lock['alpine']['mirror']}/{lock['alpine']['branch']}/{repository}/{arch}/APKINDEX.tar.gz"
        with urllib.request.urlopen(url, timeout=60) as response:
            data = response.read()
        with tarfile.open(fileobj=io.BytesIO(data)) as archive:
            index = archive.extractfile("APKINDEX").read().decode()
        found.update(re.findall(r"^P:(.+)$", index, re.M))
        for entry in index.split("\n\n"):
            fields=dict(line.split(":",1) for line in entry.splitlines() if ":" in line)
            if "P" in fields and "V" in fields: versions[fields["P"]]=fields["V"]
    boots = {"grub-ieee1275"} if arch == "ppc64le" else {"s390-tools"} if arch == "s390x" else {"systemd-boot"}
    if arch in {"x86", "x86_64"}:
        boots.update({"syslinux", "amd-ucode", "intel-ucode"})
    if arch in {"x86", "x86_64"}:
        boots.update({"mesa-vulkan-intel", "mesa-vulkan-nouveau", "mesa-vulkan-virtio"})
    if arch == "aarch64":
        boots.update({"mesa-vulkan-asahi", "mesa-vulkan-broadcom", "mesa-vulkan-freedreno",
                      "mesa-vulkan-imagination", "mesa-vulkan-intel", "mesa-vulkan-nouveau",
                      "mesa-vulkan-panfrost", "mesa-vulkan-virtio"})
    missing = (required | boots) - found
    if missing:
        raise RuntimeError(f"{arch}: unavailable packages: {', '.join(sorted(missing))}")
    if not versions.get("tlp", "").startswith("1.10."):
        raise RuntimeError(f"{arch}: Settings expects the tested TLP 1.10 series")
    return f"{arch}: {len(required | boots)} required Alpine packages available"


with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
    for result in pool.map(check, lock["rootfs"]):
        print(result)
