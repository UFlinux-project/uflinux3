#!/usr/bin/env python3
"""Create the settings source bundle with byte-stable USTAR headers (no compressor)."""
import argparse
import hashlib
import io
import json
from pathlib import Path
import re
import tarfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]


def bundle():
    source = ROOT / "settings"
    version = tomllib.loads((source / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    contents = {"LICENSE": (ROOT / "LICENSE").read_bytes()}
    for path in sorted(source.rglob("*")):
        relative = path.relative_to(source)
        if any(part in {"target", "__pycache__", ".git"} for part in relative.parts):
            continue
        if path.is_symlink():
            raise ValueError("Source bundles cannot contain symlinks")
        if path.is_file():
            contents[str(relative)] = path.read_bytes()
    stream = io.BytesIO()
    with tarfile.open(fileobj=stream, mode="w", format=tarfile.USTAR_FORMAT) as archive:
        for name, data in sorted(contents.items()):
            entry = tarfile.TarInfo(f"uflinux-settings-{version}/{name}")
            entry.size = len(data)
            entry.mode = 0o644
            archive.addfile(entry, io.BytesIO(data))
    return version, stream.getvalue()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--update", action="store_true")
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    version, data = bundle()
    digest = hashlib.sha512(data).hexdigest()
    lock_path = ROOT / "config/sources.lock.json"
    recipe_path = ROOT / "aports/uflinux-settings/APKBUILD"
    lock = json.loads(lock_path.read_text())
    recipe = recipe_path.read_text()
    if args.update:
        lock["settings"] = {"version": version, "source_sha512": digest}
        lock_path.write_text(json.dumps(lock, indent=2) + "\n")
        recipe = re.sub(r"^pkgver=.*$", "pkgver=" + version, recipe, flags=re.M)
        recipe = re.sub(r'^sha512sums="[^"]*"', lambda _: f'sha512sums="\n{digest}  uflinux-settings-{version}.tar\n"', recipe, flags=re.M)
        recipe_path.write_text(recipe)
    elif args.check:
        if lock.get("settings") != {"version": version, "source_sha512": digest} or digest not in recipe:
            raise SystemExit("Settings source changed; run python3 scripts/settings-source.py --update")
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_bytes(data)
    print(f"{digest}  uflinux-settings-{version}.tar")


if __name__ == "__main__":
    main()
