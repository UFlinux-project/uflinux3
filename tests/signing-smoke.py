#!/usr/bin/env python3
"""Exercise the release-key input path with a disposable key, never a production key."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
from builder import ARCHES, LOCK, PROJECT, unsigned_digest, temporary_tree

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--arch", choices=ARCHES, required=True)
parser.add_argument("--version", default=LOCK["distribution"]["version"])
parser.add_argument("--outdir", default="out")
args = parser.parse_args()
if os.geteuid() != 0:
    parser.error("Run on the disposable build host as root")
out = Path(args.outdir).resolve()
source = out / "packages" / args.arch
before = {p.name: unsigned_digest(p) for p in source.glob("*.apk")}
if not before:
    raise RuntimeError("Build packages before running the signing smoke test")
(PROJECT / ".cache").mkdir(exist_ok=True)
with temporary_tree(prefix="signing-smoke-", dir=PROJECT / ".cache") as temporary:
    temporary = Path(temporary)
    private = temporary / "test.rsa"
    subprocess.run(["openssl", "genrsa", "-out", str(private), "3072"], check=True)
    private.chmod(0o600)
    public = subprocess.check_output(["openssl", "pkey", "-in", str(private), "-pubout"], text=True)
    private_text = private.read_text()
    scratch = temporary / "out"
    shutil.copytree(source, scratch / "packages" / args.arch)
    env = dict(os.environ, APK_SIGNING_PRIVATE_KEY=private_text, APK_SIGNING_PUBLIC_KEY=public)
    command = [sys.executable, str(PROJECT / "scripts/builder.py"), "--arch", args.arch,
               "--version", args.version, "--stage", "sign", "--release",
               "--outdir", str(scratch), "--workdir", str(temporary / "roots")]
    stale_repository = temporary / "roots" / args.arch / "root/repo" / args.arch
    # Populate after bootstrap: signing must discard earlier release artifacts.
    bootstrap = [sys.executable, str(PROJECT / "scripts/builder.py"), "--arch", args.arch,
                 "--version", args.version, "--stage", "bootstrap", "--outdir", str(scratch),
                 "--workdir", str(temporary / "roots")]
    with (temporary / "sign.log").open("w") as log:
        subprocess.run(bootstrap, env=os.environ, stdout=log, stderr=subprocess.STDOUT, check=True)
        stale_repository.mkdir(parents=True, exist_ok=True)
        (stale_repository / "uflinux-obsolete-0.0.1-r0.apk").write_bytes(b"obsolete build")
        subprocess.run(command, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
    after = {p.name: unsigned_digest(p) for p in (scratch / "repository" / args.arch).glob("*.apk")}
    if before != after:
        raise RuntimeError("Resigning changed APK control/data streams")
    staged_key = temporary / "roots" / args.arch / "root/var/lib/uflinux/keys/uflinux.rsa"
    if staged_key.exists():
        raise RuntimeError("The successful signing command retained its private key")
    env["APK_SIGNING_PUBLIC_KEY"] = (source / "build.rsa.pub").read_text()
    rejected = subprocess.run(command, env=env, text=True, capture_output=True)
    if not rejected.returncode or "public and private signing keys do not match" not in rejected.stderr:
        raise RuntimeError("A mismatched signing keypair was not explicitly rejected")
    if staged_key.exists():
        raise RuntimeError("The failed signing command retained its private key")
    shard_keys_verified = False
    key_manifest = scratch / "packages" / args.arch / "build-keys.json"
    if key_manifest.exists():
        keys = json.loads(key_manifest.read_text())
        if len(set(keys.values())) < 2:
            raise RuntimeError("Independent package jobs must not share a development key")
        name = sorted(keys)[0]
        keys[name] = next(value for value in keys.values() if value != keys[name])
        key_manifest.write_text(json.dumps(keys))
        env["APK_SIGNING_PUBLIC_KEY"] = public
        wrong_shard = subprocess.run(command, env=env, text=True, capture_output=True)
        if not wrong_shard.returncode or not any(word in (wrong_shard.stdout + wrong_shard.stderr).lower() for word in ("signature", "untrusted")):
            raise RuntimeError("An APK was accepted with another package job's signing key")
        if staged_key.exists():
            raise RuntimeError("Rejected shard signature retained the release signing key")
        shard_keys_verified = True
    if private_text in (temporary / "sign.log").read_text() + rejected.stdout + rejected.stderr:
        raise RuntimeError("The signing log disclosed private key contents")
report = out / "checks" / args.arch / "signing-smoke.json"
report.parent.mkdir(parents=True, exist_ok=True)
report.write_text(json.dumps({"arch": args.arch, "packages": len(before), "disposable_key": True,
                              "payloads_preserved": True, "mismatched_keys_rejected": True,
                              "independent_shard_keys_verified": shard_keys_verified,
                              "stale_packages_removed": True,
                              "private_keys_removed_on_success_and_failure": True}, indent=2) + "\n")
print(f"Release signing input path passed for {len(before)} packages using a disposable key")
