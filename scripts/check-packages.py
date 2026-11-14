#!/usr/bin/env python3
"""Check declared desktop/installer dependencies against every target repository."""
import concurrent.futures
import hashlib
import io
import json
from pathlib import Path
import re
import shlex
import sys
import tarfile
import urllib.request

sys.path.insert(0, str(Path(__file__).resolve().parent))
from builder import device_packages

root = Path(__file__).resolve().parents[1]
lock = json.loads((root / "config/sources.lock.json").read_text())
text = (root / "aports/uflinux/APKBUILD").read_text()
required = set()
for block in re.findall(r'\bdepends="([^"]*)"', text):
    for item in shlex.split(block):
        name = re.split(r"[=<>~]", item)[0]
        if not name.startswith(("uflinux", "$")):
            required.add(name)
required.update({"linux-lts", "linux-firmware", "xfsprogs", "alpine-conf", "rust", "cargo", "cargo-auditable",
                 "gtk4.0-dev", "gtk4-layer-shell-dev", "pulseaudio-dev", "openssl-dev", "abuild"})
required.update({"polkit-dev", "eudev-dev", "vulkan-loader-dev", "mesa-dev", "pkgconf",
                 "ddcutil", "tzdata", "vulkan-loader", "mesa-vulkan-ati"})
required.update({"rsvg-convert"})
required.update({"tlp", "tlp-openrc", "tlp-udev"})
required.update({"py3-atspi", "wtype", "gcc", "musl-dev", "wayland-dev", "libxkbcommon-dev", "desktop-file-utils", "wlr-randr"})
required.update({"autoconf", "automake", "dbus-dev", "glib-dev", "libmnl-dev", "libnftnl-dev", "libtool", "readline-dev", "linux-headers"})

# Packages built from this repository's own aports tree are not in the Alpine
# repositories; they are compiled by the build jobs before the image is assembled.
local = set()
for recipe in (root / "aports").glob("*/APKBUILD"):
    name = re.search(r'^pkgname="?([^"\s]+)"?$', recipe.read_text(), re.M)
    if name:
        local.add(name.group(1))
required -= local


def index_entries(url):
    with urllib.request.urlopen(url, timeout=60) as response:
        data = response.read()
    with tarfile.open(fileobj=io.BytesIO(data)) as archive:
        return archive.extractfile("APKINDEX").read().decode()


def alpine_index(arch):
    found = set()
    versions = {}
    for repository in ("main", "community"):
        url = f"{lock['alpine']['mirror']}/{lock['alpine']['branch']}/{repository}/{arch}/APKINDEX.tar.gz"
        index = index_entries(url)
        found.update(re.findall(r"^P:(.+)$", index, re.M))
        for entry in index.split("\n\n"):
            fields=dict(line.split(":",1) for line in entry.splitlines() if ":" in line)
            if "P" in fields and "V" in fields: versions[fields["P"]]=fields["V"]
    return found, versions


def check(arch):
    found, versions = alpine_index(arch)
    boots = {"grub-ieee1275"} if arch == "ppc64le" else {"systemd-boot"}
    if arch in {"x86", "x86_64", "aarch64", "armv7", "riscv64"}:
        boots.add("efibootmgr")
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


devices = json.loads((root / "config/devices.json").read_text())
device_lock = json.loads((root / "config/devices.lock.json").read_text())["nura"]


def check_device(device_id):
    device = devices[device_id]
    messages = []
    for arch in device["arches"]:
        found, _ = alpine_index(arch)
        # The UFLinux subpackages are built from this repository's own aports tree,
        # and the Nura extras are validated against the pinned Nura index below.
        installed = [name for name in device_packages(device, arch)
                     if not name.startswith("uflinux") and name not in device["nura_packages"]]
        missing = set(installed) - found - local
        if missing:
            raise RuntimeError(f"{device_id} ({arch}): unavailable device packages: {', '.join(sorted(missing))}")
        messages.append(f"{device_id} ({arch}): {len(installed)} device packages available")
    if device["nura"]:
        arch = device["arches"][0]
        url = f"{device_lock['mirror']}/{device_lock['branch']}/{arch}/APKINDEX.tar.gz"
        with urllib.request.urlopen(url, timeout=60) as response:
            data = response.read()
        digest = hashlib.sha256(data).hexdigest()
        if digest != device_lock["index_sha256"][arch]:
            raise RuntimeError(f"{device_id}: Nura {arch} index differs from its pin ({digest})")
        found = set(re.findall(r"^P:(.+)$", index_entries(url), re.M))
        missing = set(device["nura_packages"]) - found
        if missing:
            raise RuntimeError(f"{device_id}: unavailable Nura packages: {', '.join(sorted(missing))}")
        messages.append(f"{device_id}: pinned Nura index provides {len(device['nura_packages'])} extra packages")
    return messages


with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
    for result in pool.map(check_device, sorted(devices)):
        for line in result:
            print(line)
