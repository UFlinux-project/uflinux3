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
assert set(arches) == {"x86", "x86_64", "aarch64", "armv7", "ppc64le", "s390x", "riscv64"}
for path in [*ROOT.glob("scripts/*.py"), *ROOT.glob("tests/*.py"), ROOT / "aports/uflinux/uflinux-install", ROOT / "aports/uflinux/uflinux-install-gui-backend"]:
    ast.parse(path.read_text(), filename=str(path))
for path in [*ROOT.glob("scripts/*.sh"), *ROOT.glob("tests/*.sh"), *ROOT.glob("profiles/*.sh"), *ROOT.glob("aports/*/APKBUILD"),
             *[ROOT / "aports/uflinux" / n for n in ("uflinux-session", "uflinux-autostart", "uflinux-start-panel", "uflinux-live.initd", "uflinux-compositor", "uflinux-network-init", "uflinux-bootsplash", "uflinux-splash.initd", "uflinux-splash-stop.initd")], *ROOT.glob("settings/data/*.initd"), ROOT / "settings/data/uflinux-power-button"]:
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
assert (ROOT / "LICENSE").read_bytes() == (ROOT / "aports/uflinux/LICENSE").read_bytes()
for name in ("ironbar.toml", "greetd-installed.toml", "greetd-live.toml", "regreet.toml"):
    tomllib.loads((ROOT / "aports/uflinux" / name).read_text())
subprocess.run([sys.executable, str(ROOT / "scripts/update-checksums.py"), "--check"], check=True)
subprocess.run([sys.executable, str(ROOT / "scripts/settings-source.py"), "--check"], check=True)
ironbar = (ROOT / "aports/ironbar/APKBUILD").read_text()
assert lock["sources"]["ironbar"]["sha512"] in ironbar
assert '_features="' + lock["ironbar"]["features"] + '"' in ironbar
assert "target-cpu=native" not in ironbar
connman=(ROOT / "aports/connman/APKBUILD").read_text()
assert lock["sources"]["connman"]["sha512"] in connman
for name,digest in lock["connman"]["patches"].items():
    assert hashlib.sha512((ROOT / "aports/connman" / name).read_bytes()).hexdigest()==digest
for name, digest in lock["ironbar"].get("patches", {}).items():
    assert hashlib.sha512((ROOT / "aports/ironbar" / name).read_bytes()).hexdigest() == digest
regreet = (ROOT / "aports/uflinux-regreet/APKBUILD").read_text()
assert lock["sources"]["uflinux-regreet"]["sha512"] in regreet
for name, digest in lock["uflinux-regreet"]["patches"].items():
    assert hashlib.sha512((ROOT / "aports/uflinux-regreet" / name).read_bytes()).hexdigest() == digest
assert "libadwaita" not in (ROOT / "settings/Cargo.lock").read_text()
for origin in ("connman", "ironbar", "uflinux-regreet", "uflinux-settings"):
    recipe_arches = re.search(r'^arch="([^"]+)"$', (ROOT / "aports" / origin / "APKBUILD").read_text(), re.M)
    assert recipe_arches and set(recipe_arches.group(1).split()) == set(arches), f"{origin} architecture list differs from build matrix"
for name, digest in lock["image"]["patches"].items():
    assert hashlib.sha512((ROOT / "patches" / name).read_bytes()).hexdigest() == digest
profile=tomllib.loads((ROOT / "settings/Cargo.toml").read_text())["profile"]["release"]
recipe=(ROOT / "aports/uflinux-settings/APKBUILD").read_text()
for setting in ("codegen-units", "panic"):
    variable="CARGO_PROFILE_RELEASE_"+setting.upper().replace("-","_")
    assert f"export {variable}={profile[setting]}" in recipe, f"abuild must enforce {variable}"
for setting in ("OPT_LEVEL", "LTO"):
    assert re.search(rf"^export CARGO_PROFILE_RELEASE_{setting}=\S+", recipe, re.M), f"abuild must set {setting}"
assert "debug" not in profile, "Settings release profile must use Cargo's default debug level"
for origin in ("ironbar", "uflinux-settings"):
    recipe = (ROOT / "aports" / origin / "APKBUILD").read_text()
    assert "unset CARGO_PROFILE_RELEASE_DEBUG" in recipe, f"{origin} must clear abuild's debug override"
for path in (ROOT / ".github/workflows").glob("*.yml"):
    for action in re.findall(r"uses:\s*([^\s#]+)", path.read_text()):
        if not action.startswith("./"):
            assert re.fullmatch(r"[\w/-]+@[0-9a-f]{40}", action), f"Unpinned action: {action}"
subprocess.run([sys.executable, "-m", "unittest", "discover", "-s", "tests", "-p", "test_*.py"], cwd=ROOT, check=True)
print("Static checks passed")
