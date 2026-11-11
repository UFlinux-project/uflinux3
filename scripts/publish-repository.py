#!/usr/bin/env python3
"""Assemble and sign a complete Pages repository; retain two previous releases."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile

from builder import ARCHES, Builder, LOCK, PROJECT, SIGNING_KEY, sha, unsigned_digest, PACKAGE_NAMES
from sbom import apk_metadata

ALLOWED = set().union(*PACKAGE_NAMES.values())


def safe_extract(archive, target):
    with tarfile.open(archive) as tar:
        for member in tar.getmembers():
            if not (member.isfile() or member.isdir()):
                raise ValueError("Repository snapshots must contain only regular files and directories")
        tar.extractall(target, filter="data")


def selection(current, history):
    return [current, *[release for release in history if release["version"] != current["version"]][:2]]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--outdir", default="out")
    parser.add_argument("--version", required=True)
    parser.add_argument("--previous")
    args = parser.parse_args()
    out = Path(args.outdir).resolve()
    args.arch = "x86_64"
    args.workdir = str(PROJECT / ".state/publisher")
    args.cache = str(PROJECT / ".cache/publisher")
    args.repository_url = None
    args.release = True
    builder = Builder(args)
    try:
        builder.bootstrap()
        with builder.mounted(), tempfile.TemporaryDirectory(prefix="uflinux-history-") as temporary:
            builder.tools()
            public = builder.signing_key(release=True)
            pages = out / "pages"
            if pages.exists():
                shutil.rmtree(pages)
            branch = pages / "apk" / LOCK["alpine"]["branch"] / "uflinux"
            branch.mkdir(parents=True)
            history = []
            previous = Path(temporary)
            if args.previous:
                safe_extract(args.previous, previous)
                history = json.loads((previous / "releases.json").read_text())
                if history:
                    order = builder.chroot("apk", "version", "-t", args.version, history[0]["version"], stdout=subprocess.PIPE).stdout.decode().strip()
                    if order == "<":
                        raise ValueError("Refusing to replace a newer repository with an older release")
            current = {"version": args.version, "packages": {}}
            for arch in ARCHES:
                # Publication cannot accidentally succeed on a partial matrix.
                check = json.loads((out / "checks" / arch / "verified.json").read_text())
                if check["version"] != args.version or check["signing"] != "release":
                    raise ValueError("Unverified release package set for " + arch)
                if check.get("verified_packages") != {p.name: sha(p) for p in (out / "repository" / arch).glob("*.apk")}:
                    raise ValueError("Verification does not cover the current package archives for " + arch)
                image_record = json.loads((out / "metadata" / arch / "image.json").read_text())
                iso = out / "images" / arch / f"uflinux-{args.version}-{arch}.iso"
                if image_record.get("iso_sha256") != sha(iso):
                    raise ValueError("Image metadata does not describe the current ISO for " + arch)
                inventory = json.loads((out / "metadata" / arch / f"uflinux-{args.version}-{arch}.spdx.json").read_text())
                included = {(package["name"], package["versionInfo"]): package["checksums"][0]["checksumValue"] for package in inventory["packages"]}
                names = set()
                for package in (out / "repository" / arch).glob("*.apk"):
                    info = apk_metadata(package)
                    names.add(info["pkgname"])
                    if not info["pkgname"].endswith(("-dbg", "-doc", "-dev")) and included.get((info["pkgname"], info["pkgver"])) != sha(package):
                        raise ValueError("The ISO does not contain the current custom package: " + package.name)
                if names != ALLOWED:
                    raise ValueError("Incomplete or unexpected custom package set for " + arch)
                modes = ["bios", "uefi"] if arch in {"x86", "x86_64"} else ["native" if arch in {"ppc64le", "s390x"} else "uefi"]
                for mode in modes:
                    result = json.loads((out / "vm" / arch / mode / "verified.json").read_text())
                    iso = out / "images" / arch / f"uflinux-{args.version}-{arch}.iso"
                    if not result.get("xfs_install_reboot") or result.get("iso_sha256") != sha(iso):
                        raise ValueError("Missing installed-system boot verification for " + arch)
                    if arch == "x86_64":
                        memory = result.get("settings_512m") or {}
                        if memory.get("ram_mib") != 512 or memory.get("swap") is not False or memory.get("oom") is not False:
                            raise ValueError("Missing installed 512 MiB Settings acceptance for " + mode)
                        if mode == "bios" and not result.get("network_clock"):
                            raise ValueError("Missing real ConnMan+iwd and ConnMan SNTP verification")
                        power = result.get("power") or {}
                        if not power.get("gtk_authorization_cancel_and_grant") or not power.get("physical_key_do_nothing") or not power.get("physical_key_poweroff") or set(power.get("profiles", [])) != {"power-saver", "balanced", "performance"}:
                            raise ValueError("Missing TLP profiles and BusyBox power-button verification")
                current["packages"][arch] = sorted(p.name for p in (out / "repository" / arch).glob("*.apk"))
                if not current["packages"][arch]:
                    raise ValueError("Empty release architecture: " + arch)
                if arch == "x86_64":
                    proof = json.loads((out / "metadata" / arch / "reproducibility.json").read_text())
                    payloads = {p.name: unsigned_digest(p) for p in (out / "repository" / arch).glob("*.apk")}
                    if not proof.get("matched") or proof.get("baseline") != payloads or proof.get("rebuilt") != payloads:
                        raise ValueError("The reproducibility record does not cover the current native package set")
            releases = selection(current, history)
            for arch in ARCHES:
                staging = builder.root / "repo" / arch
                if staging.exists():
                    shutil.rmtree(staging)
                staging.mkdir(parents=True, exist_ok=True)
                names = set()
                for release in releases:
                    for name in release["packages"][arch]:
                        if Path(name).name != name or not name.endswith(".apk"):
                            raise ValueError("Invalid APK snapshot filename")
                        if name in names:
                            continue
                        names.add(name)
                        source = (out / "repository" / arch if release is current else previous / arch) / name
                        if apk_metadata(source)["pkgname"] not in ALLOWED:
                            raise ValueError("Unexpected custom package namespace: " + name)
                        shutil.copy2(source, staging / name)
                        builder.chroot("apk", "verify", f"/repo/{arch}/{name}")
                builder.index(staging)
                shutil.copytree(staging, branch / arch)
            shutil.copy2(public, branch / (SIGNING_KEY + ".pub"))
            (branch / "releases.json").write_text(json.dumps(releases, indent=2) + "\n")
            (pages / ".nojekyll").touch()
            (pages / "index.html").write_text('<!doctype html><html lang="en"><meta charset="utf-8"><title>UFLinux APK repository</title><h1>UFLinux</h1><p>Signed Alpine packages. Download installation images from this project’s GitHub Releases.</p></html>\n')
            size = sum(p.stat().st_size for p in pages.rglob("*") if p.is_file())
            if size > 900 * 1024**2:
                raise ValueError("Pages repository exceeds the 900 MiB safety budget")
            snapshot = out / "uflinux-apk-repository.tar.gz"
            # GNU tar + gzip -n preserve deterministic metadata.
            tar = subprocess.Popen(["tar", "--sort=name", "--mtime=@" + builder.epoch, "--owner=0", "--group=0", "--numeric-owner", "-C", str(branch), "-cf", "-", "."], stdout=subprocess.PIPE)
            with snapshot.open("wb") as stream:
                subprocess.run(["gzip", "-9n"], stdin=tar.stdout, stdout=stream, check=True)
            tar.stdout.close()
            if tar.wait():
                raise RuntimeError("Snapshot archive creation failed")
            assets = [snapshot, *out.glob("images/*/*.iso"), *out.glob("metadata/*/*.spdx.json")]
            (out / "SHA256SUMS").write_text("".join(sha(p) + "  " + p.name + "\n" for p in sorted(assets)))
        return 0
    finally:
        (builder.root / "var/lib/uflinux/keys" / SIGNING_KEY).unlink(missing_ok=True)


if __name__ == "__main__":
    sys.exit(main())
