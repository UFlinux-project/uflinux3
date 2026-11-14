#!/usr/bin/env python3
"""Fast checks that run without root or an Alpine build environment."""
import ast
import hashlib
import json
from pathlib import Path
import re
import shlex
import subprocess
import sys
import tomllib
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1]
arches = json.loads((ROOT / "config/architectures.json").read_text())
lock = json.loads((ROOT / "config/sources.lock.json").read_text())
assert set(arches) == set(lock["rootfs"])
assert set(arches) == {"x86", "x86_64", "aarch64", "armv7", "ppc64le", "riscv64"}
devices = json.loads((ROOT / "config/devices.json").read_text())
device_lock = json.loads((ROOT / "config/devices.lock.json").read_text())
assert device_lock["schema"] == 1
for device_id, device in devices.items():
    assert re.fullmatch(r"[a-z0-9][a-z0-9-]*", device_id), f"Device id must be a lowercase slug: {device_id}"
    assert device["label"], f"{device_id} needs a label"
    assert device["arches"] and set(device["arches"]) <= set(arches), f"{device_id} lists an unknown architecture"
    # Alpine flavours are installed; a "postmarketos-*" flavour comes from the
    # pinned Nura suite and is fetched and unpacked by the device profile.
    assert device["kernel"] in {"lts", "rpi", "virt"} or device["kernel"].startswith("postmarketos-"), \
        f"{device_id} must use a known kernel flavour"
    assert device["boot"] in {"efi", "rpi", "extlinux"}, f"{device_id} must use a known boot mode"
    if device["boot"] == "extlinux":
        assert re.fullmatch(r"[a-z0-9][a-z0-9._/-]*\.dtb", device.get("dtb", "")), \
            f"{device_id} boots with extlinux and needs the dtb path it loads"
    if device.get("dtb"):
        assert re.fullmatch(r"[a-z0-9][a-z0-9._/-]*\.dtb", device["dtb"]), \
            f"{device_id} needs a device tree path relative to the boot partition's dtbs directory"
    assert device["status"] in {"supported", "experimental"}, f"{device_id} must declare a release status"
    assert device["rootfs"] in {"f2fs", "xfs"}, f"{device_id} must use a root filesystem the device stage can build"
    assert device["packages"] and all(re.fullmatch(r"[a-z0-9][a-z0-9+.-]*", p) for p in device["packages"]), \
        f"{device_id} needs a list of Alpine package names"
    assert isinstance(device["img_size_mib"], int) and device["img_size_mib"] > 512, f"{device_id} needs a usable image size"
    assert re.fullmatch(r"tty[A-Z]*[0-9]+", device["serial"]), f"{device_id} needs a serial console device"
    assert isinstance(device["nura"], bool), f"{device_id} must state whether the Nura repository is used"
    assert device["nura_packages"] == [] or device["nura"], f"{device_id} lists Nura packages without enabling the repository"
nura = device_lock["nura"]
assert re.fullmatch(r"v[0-9]{2}\.[0-9]{2}|main", nura["branch"]), "Nura must be pinned to a release branch"
assert nura["mirror"].startswith("https://") and not nura["mirror"].endswith("/")
assert re.fullmatch(r"[0-9a-f]{64}", nura["repository_key_sha256"])
assert set(nura["arches"]) <= set(arches) and set(nura["index_sha256"]) == set(nura["arches"])
for digest in nura["index_sha256"].values():
    assert re.fullmatch(r"[0-9a-f]{64}", digest), "Nura repository indexes must be pinned by sha256"
for device_id, device in devices.items():
    if device["nura"]:
        assert set(device["arches"]) <= set(nura["arches"]), f"{device_id} uses Nura on an unpinned architecture"
for path in [*ROOT.glob("scripts/*.py"), *ROOT.glob("tests/*.py"), ROOT / "aports/uflinux/uflinux-install", ROOT / "aports/uflinux/uflinux-install-gui-backend", ROOT / "aports/uflinux/uflinux-boot-assets.py"]:
    ast.parse(path.read_text(), filename=str(path))
for path in [*ROOT.glob("scripts/*.sh"), *ROOT.glob("tests/*.sh"), *ROOT.glob("profiles/*.sh"), *ROOT.glob("aports/*/APKBUILD"),
             *[ROOT / "aports/uflinux" / n for n in ("uflinux-session", "uflinux-autostart", "uflinux-live.initd", "uflinux-compositor", "uflinux-network-init", "uflinux-bootsplash", "uflinux-splash.initd", "uflinux-splash-stop.initd", "uflinux-device.initd")], *ROOT.glob("settings/data/*.initd"), ROOT / "settings/data/uflinux-power-button"]:
    subprocess.run(["sh", "-n", str(path)], check=True)
for path in (ROOT / "aports/uflinux").glob("*.xml"):
    ET.parse(path)
for path in (ROOT / "aports/uflinux").glob("*.svg"):
    ET.parse(path)
for path in (ROOT / "settings/data").iterdir():
    if path.suffix in {"svg", "policy", "conf"} and path.read_text().lstrip().startswith("<"):
        ET.parse(path)
artwork = json.loads((ROOT / "config/artwork.lock.json").read_text())
for name in artwork["logo"]["copies"]:
    assert hashlib.sha512((ROOT / name).read_bytes()).hexdigest() == artwork["logo"]["sha512"], "Imported logo differs from its pinned original"
for name in artwork["logotype"]["copies"]:
    assert hashlib.sha512((ROOT / name).read_bytes()).hexdigest() == artwork["logotype"]["sha512"], "Imported boot logotype differs from its pinned original"
assert (ROOT / "LICENSE").read_bytes() == (ROOT / "aports/uflinux/LICENSE").read_bytes()
for name in ("greetd-installed.toml", "greetd-live.toml", "regreet.toml"):
    tomllib.loads((ROOT / "aports/uflinux" / name).read_text())
subprocess.run([sys.executable, str(ROOT / "scripts/update-checksums.py"), "--check"], check=True)
subprocess.run([sys.executable, str(ROOT / "scripts/settings-source.py"), "--check"], check=True)
connman=(ROOT / "aports/connman/APKBUILD").read_text()
assert lock["sources"]["connman"]["sha512"] in connman
for name,digest in lock["connman"]["patches"].items():
    assert hashlib.sha512((ROOT / "aports/connman" / name).read_bytes()).hexdigest()==digest
regreet = (ROOT / "aports/uflinux-regreet/APKBUILD").read_text()
assert lock["sources"]["uflinux-regreet"]["sha512"] in regreet
for name, digest in lock["uflinux-regreet"]["patches"].items():
    assert hashlib.sha512((ROOT / "aports/uflinux-regreet" / name).read_bytes()).hexdigest() == digest
recorder = (ROOT / "aports/gpu-screen-recorder/APKBUILD").read_text()
assert lock["sources"]["gpu-screen-recorder"]["sha512"] in recorder
for name, digest in lock["gpu-screen-recorder"]["patches"].items():
    assert hashlib.sha512((ROOT / "aports/gpu-screen-recorder" / name).read_bytes()).hexdigest() == digest
assert "libadwaita" not in (ROOT / "settings/Cargo.lock").read_text()
for origin in ("connman", "gpu-screen-recorder", "uflinux-regreet", "uflinux-settings"):
    recipe_arches = re.search(r'^arch="([^"]+)"$', (ROOT / "aports" / origin / "APKBUILD").read_text(), re.M)
    assert recipe_arches and set(recipe_arches.group(1).split()) == set(arches), f"{origin} architecture list differs from build matrix"
for name, digest in lock["image"]["patches"].items():
    assert hashlib.sha512((ROOT / "patches" / name).read_bytes()).hexdigest() == digest
profile=tomllib.loads((ROOT / "settings/Cargo.toml").read_text())["profile"]["release"]
recipe=(ROOT / "aports/uflinux-settings/APKBUILD").read_text()
for setting in ("codegen-units", "panic"):
    variable="CARGO_PROFILE_RELEASE_"+setting.upper().replace("-","_")
    assert f"export {variable}={profile[setting]}" in recipe, f"abuild must enforce {variable}"
assert "export CARGO_PROFILE_RELEASE_OPT_LEVEL=s" in recipe, "Settings must build with -Os"
assert re.search(r"^export CARGO_PROFILE_RELEASE_LTO=\S+", recipe, re.M), "abuild must set LTO"
for origin in ("uflinux-regreet",):
    assert "export CARGO_PROFILE_RELEASE_OPT_LEVEL=s" in (ROOT / "aports" / origin / "APKBUILD").read_text(), \
        f"{origin} must build with -Os"
for origin in ("connman", "gpu-screen-recorder"):
    assert re.search(r'^\texport (C|CXX)FLAGS="\$(C|CXX)FLAGS .*-Os', (ROOT / "aports" / origin / "APKBUILD").read_text(), re.M), \
        f"{origin} must append -Os to abuild's flags"
assert "debug" not in profile, "Settings release profile must use Cargo's default debug level"
for origin in ("uflinux-settings",):
    recipe = (ROOT / "aports" / origin / "APKBUILD").read_text()
    assert "unset CARGO_PROFILE_RELEASE_DEBUG" in recipe, f"{origin} must clear abuild's debug override"
for path in (ROOT / ".github/workflows").glob("*.yml"):
    for action in re.findall(r"uses:\s*([^\s#]+)", path.read_text()):
        if not action.startswith("./"):
            assert re.fullmatch(r"[\w/-]+@[0-9a-f]{40}", action), f"Unpinned action: {action}"
subprocess.run([sys.executable, "-m", "unittest", "discover", "-s", "tests", "-p", "test_*.py"], cwd=ROOT, check=True)
print("Static checks passed")
