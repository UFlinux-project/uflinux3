#!/usr/bin/env python3
"""Alpine chroot build stages. Root is needed only on the disposable build host."""
import argparse
import contextlib
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import re
import shlex
import shutil
import subprocess
import struct
import sys
import tarfile
import tempfile
import time
import urllib.parse
import urllib.request
import zlib

PROJECT = Path(__file__).resolve().parents[1]
LOCK = json.loads((PROJECT / "config/sources.lock.json").read_text())
ARCHES = json.loads((PROJECT / "config/architectures.json").read_text())
PACKAGE_NAMES = {
    "connman": {"connman", "connman-dbg", "connman-dev", "connman-doc", "connman-openrc"},
    "ironbar": {"ironbar", "ironbar-dbg", "ironbar-doc"},
    "uflinux-settings": {"uflinux-settings", "uflinux-settings-daemon", "uflinux-settings-dbg",
                         "uflinux-settings-doc", "uflinux-settings-system", "uflinux-settings-system-openrc"},
    "uflinux": {"uflinux", "uflinux-defaults", "uflinux-desktop", "uflinux-installer",
                "uflinux-live", "uflinux-live-openrc", "uflinux-release", "uflinux-bootsplash-openrc",
                "uflinux-initramfs"},
}


def image_compression(initramfs, modloop):
    with Path(initramfs).open("rb") as stream:
        if stream.read(4) != b"\x28\xb5\x2f\xfd":
            raise ValueError("Initramfs is not a Zstandard frame")
    with Path(modloop).open("rb") as stream:
        header = stream.read(32)
    if len(header) < 32 or header[:4] != b"hsqs":
        raise ValueError("Modloop is not a SquashFS filesystem")
    if struct.unpack_from("<H", header, 20)[0] != 6:
        raise ValueError("Modloop does not use Zstandard compression")
    if struct.unpack_from("<I", header, 12)[0] != LOCK["image"]["modloop_block_bytes"]:
        raise ValueError("Modloop block size differs from the image lock")
    return {"initramfs_compression": "zstd", "modloop_compression": "zstd",
            "modloop_block_bytes": LOCK["image"]["modloop_block_bytes"],
            "initramfs_bytes": Path(initramfs).stat().st_size,
            "modloop_bytes": Path(modloop).stat().st_size}


def package_output(out, arch, package=None):
    out = Path(out).resolve()
    return out / "package-shards" / arch / package if package else out



def run(args, **kwargs):
    print("+", shlex.join(str(a) for a in args), flush=True)
    return subprocess.run([str(a) for a in args], check=True, **kwargs)


def sha(path, algorithm="sha256"):
    digest = hashlib.new(algorithm)
    with Path(path).open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def link_or_copy(source, destination):
    destination = Path(destination)
    destination.unlink(missing_ok=True)
    try:
        os.link(source, destination)
    except OSError:
        shutil.copy2(source, destination)


def mounted_paths():
    result = []
    for line in Path("/proc/self/mountinfo").read_text().splitlines():
        field = line.split()[4]
        result.append(Path(re.sub(r"\\([0-7]{3})", lambda match: chr(int(match[1], 8)), field)))
    return result


def remove_tree(path):
    """Never traverse a build directory that still contains bind mounts."""
    path = Path(path)
    if path.is_symlink():
        path.unlink()
        return
    if not path.exists():
        return
    resolved = path.resolve()
    if any(mount == resolved or mount.is_relative_to(resolved) for mount in mounted_paths()):
        raise RuntimeError("Refusing to remove a directory with active mounts: " + str(path))
    shutil.rmtree(path)


@contextlib.contextmanager
def temporary_tree(**kwargs):
    path = Path(tempfile.mkdtemp(**kwargs))
    try:
        yield path
    finally:
        remove_tree(path)


def archive_tree(source, destination, epoch, names=None):
    destination.parent.mkdir(parents=True, exist_ok=True)
    command = ["tar", "--sort=name", "--mtime=@" + str(epoch), "--owner=0", "--group=0", "--numeric-owner",
               "-C", str(source), "-cf", "-", "--", *(names or ["."])]
    tar = subprocess.Popen(command, stdout=subprocess.PIPE)
    try:
        with destination.open("wb") as stream:
            subprocess.run(["gzip", "-9n"], stdin=tar.stdout, stdout=stream, check=True)
    finally:
        tar.stdout.close()
    if tar.wait():
        raise RuntimeError("Failed to archive " + str(source))


def download(spec, cache):
    algorithm = "sha512" if "sha512" in spec else "sha256"
    expected = spec[algorithm]
    path = cache / (expected + ".tar.gz")
    cache.mkdir(parents=True, exist_ok=True)
    if not path.exists():
        temporary = path.with_suffix(".partial")
        for attempt in range(3):
            try:
                with urllib.request.urlopen(spec["url"], timeout=120) as response, temporary.open("wb") as stream:
                    shutil.copyfileobj(response, stream)
                if sha(temporary, algorithm) != expected:
                    raise RuntimeError("Checksum mismatch: " + spec["url"])
                temporary.replace(path)
                break
            except OSError:
                temporary.unlink(missing_ok=True)
                if attempt == 2:
                    raise
                time.sleep(2)
    if sha(path, algorithm) != expected:
        raise RuntimeError("Corrupt cached source: " + str(path))
    return path


def gzip_members(data):
    """APK v2 signatures, control, and data are distinct gzip streams."""
    while data:
        decoder = zlib.decompressobj(31)
        raw = decoder.decompress(data)
        if not decoder.eof:
            raise ValueError("Truncated APK gzip member")
        consumed = len(data) - len(decoder.unused_data)
        yield data[:consumed], raw
        data = decoder.unused_data


def gzip_file_members(path):
    """Read APK control streams without loading or expanding the data payload."""
    with Path(path).open("rb") as stream:
        pending = stream.read(65536)
        while pending:
            decoder = zlib.decompressobj(31)
            compressed, raw = bytearray(), bytearray()
            while not decoder.eof:
                chunk = pending or stream.read(65536)
                if not chunk:
                    raise ValueError("Truncated APK gzip member")
                raw.extend(decoder.decompress(chunk))
                used = len(chunk) - len(decoder.unused_data)
                compressed.extend(chunk[:used])
                pending = decoder.unused_data
            yield bytes(compressed), bytes(raw)
            if not pending:
                pending = stream.read(65536)


def promote_apk(path, cache):
    """apk v2 cache names use the first eight hex digits of the control SHA-1."""
    for compressed, raw in gzip_file_members(path):
        with tarfile.open(fileobj=io.BytesIO(raw)) as archive:
            if ".PKGINFO" not in archive.getnames():
                continue
        suffix = hashlib.sha1(compressed).hexdigest()[:8]
        name = re.sub(r"\.[0-9a-f]{8}$", "", path.stem) + "." + suffix + ".apk"
        target = cache / name
        if not target.exists():
            link_or_copy(path, target)
        return
    raise ValueError("No APK control stream: " + str(path))


def split_apk(path):
    members = list(gzip_members(Path(path).read_bytes()))
    control = None
    for index, (_, raw) in enumerate(members):
        with tarfile.open(fileobj=io.BytesIO(raw)) as archive:
            names = archive.getnames()
        if ".PKGINFO" in names:
            control = index
            break
        if not names or any(not name.startswith(".SIGN.") for name in names):
            raise ValueError("Unexpected APK member before control archive")
    if control is None or control + 2 != len(members):
        raise ValueError("Expected APK v2 signature/control/data streams")
    return members[control][0], members[control + 1][0]


def unsigned_digest(path):
    control, data = split_apk(path)
    return hashlib.sha256(gzip.decompress(control) + gzip.decompress(data)).hexdigest()


def repository_url(value):
    if not value:
        repo = os.environ.get("GITHUB_REPOSITORY", "")
        if repo:
            owner, name = repo.split("/", 1)
            suffix = "" if name.lower() == owner.lower() + ".github.io" else "/" + name
            value = f"https://{owner.lower()}.github.io{suffix}/apk/{LOCK['alpine']['branch']}/uflinux"
        else:
            raise ValueError("Set APK_REPOSITORY_URL to the final HTTPS APK repository URL")
    parsed = urllib.parse.urlsplit(value)
    if parsed.scheme != "https" or not parsed.netloc or parsed.username is not None or parsed.password is not None or parsed.query or parsed.fragment:
        raise ValueError("APK_REPOSITORY_URL must be an HTTPS directory URL")
    if any(c.isspace() for c in value) or any(c in value for c in "'\"\\`$"):
        raise ValueError("Unsafe repository URL")
    return value.rstrip("/")


class Builder:
    def __init__(self, args):
        self.args = args
        self.root = Path(args.workdir).resolve() / args.arch / "root"
        self.cache = Path(args.cache).resolve()
        self.out = package_output(args.outdir, args.arch, getattr(args, "package", None))
        self.packages_dir = self.out / "packages" / args.arch
        self.epoch = str(os.environ.get("SOURCE_DATE_EPOCH", LOCK["source_date_epoch"]))
        self.url = repository_url(args.repository_url)
        self.mounts = []
        self.env = {"PATH": "/usr/sbin:/usr/bin:/sbin:/bin", "HOME": "/root", "TERM": "dumb",
                    "SOURCE_DATE_EPOCH": self.epoch, "TZ": "UTC", "LC_ALL": "C.UTF-8",
                    "UFLINUX_REPOSITORY_URL": self.url, "UFLINUX_APK_CACHE": "/cache/image-apk",
                    "sigtype": "RSA256", "ARCH": args.arch}
        github_repository = os.environ.get("GITHUB_REPOSITORY", "")
        if re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", github_repository):
            self.env["UFLINUX_PROJECT_URL"] = "https://github.com/" + github_repository

    def chroot(self, *command, **kwargs):
        env = dict(self.env, **kwargs.pop("extra_env", {}))
        return run(["chroot", self.root, "/usr/bin/env", "-i", *[f"{k}={v}" for k, v in env.items()], *command], **kwargs)

    def put(self, name, content, mode=0o644):
        path = self.root / name.lstrip("/")
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)
        path.chmod(mode)
        return path

    def bind(self, source, destination):
        path = self.root / destination.lstrip("/")
        if Path(source).is_dir():
            path.mkdir(parents=True, exist_ok=True)
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.touch(exist_ok=True)
        run(["mount", "--bind", source, path])
        self.mounts.append(path)

    def bootstrap(self):
        fingerprint = hashlib.sha256(json.dumps(LOCK, sort_keys=True).encode()).hexdigest()
        marker = self.root / ".uflinux-bootstrap"
        if marker.exists() and marker.read_text() != fingerprint:
            raise RuntimeError("Build inputs changed: use a new --workdir")
        if not marker.exists():
            if self.root.exists() and any(self.root.iterdir()):
                raise RuntimeError("Incomplete rootfs: remove only this build's workdir and retry")
            archive = download(LOCK["rootfs"][self.args.arch], self.cache / "downloads")
            self.root.mkdir(parents=True, exist_ok=True)
            run(["tar", "--numeric-owner", "-xpf", archive, "-C", self.root])
            marker.write_text(fingerprint)
        resolv = self.root / "etc/resolv.conf"
        resolv.unlink(missing_ok=True)
        resolv.write_bytes(Path("/etc/resolv.conf").read_bytes())
        self.put("etc/apk/repositories", "\n".join(f"{LOCK['alpine']['mirror']}/{LOCK['alpine']['branch']}/{repo}" for repo in ("main", "community")) + "\n")
        # Executing apk is also a check of the kernel's binfmt registration.
        self.chroot("apk", "--print-arch")
        for name in ("aports", "profiles", "config", "scripts", "tests", "patches", "settings"):
            destination = self.root / "project" / name
            if destination.exists():
                shutil.rmtree(destination)
            shutil.copytree(PROJECT / name, destination, ignore=shutil.ignore_patterns("__pycache__", "*.pyc"))
        (self.root / "project/LICENSE").write_bytes((PROJECT / "LICENSE").read_bytes())
        self.put("project/repository-url", self.url + "\n")

    @contextlib.contextmanager
    def mounted(self):
        try:
            self.bind("/dev", "dev")
            # Container runtimes may bind these nodes individually over placeholders.
            for device in ("null", "zero", "full", "random", "urandom", "tty"):
                self.bind("/dev/" + device, "dev/" + device)
            self.bind("/dev/pts", "dev/pts")
            self.bind("/dev/shm", "dev/shm")
            # Builds do not need access to the host's sysfs/device topology.
            (self.root / "proc").mkdir(exist_ok=True)
            run(["mount", "-t", "proc", "proc", self.root / "proc"])
            self.mounts.append(self.root / "proc")
            for name in ("apk", "cargo", "target", "image-apk"):
                host = self.cache / self.args.arch / name
                host.mkdir(parents=True, exist_ok=True)
                self.bind(host, "cache/" + name)
            cache_link = self.root / "etc/apk/cache"
            if not cache_link.exists() and not cache_link.is_symlink():
                cache_link.symlink_to("/cache/apk")
            yield
        finally:
            for path in reversed(self.mounts):
                result = subprocess.run(["umount", str(path)], check=False)
                if result.returncode:
                    subprocess.run(["umount", "--lazy", str(path)], check=False)
            self.mounts.clear()

    def tools(self, *packages):
        self.chroot("apk", "add", "--no-progress", "abuild", "alpine-conf", "alpine-keys", "openssl", "tar", "gzip", "python3", *packages)

    def signing_key(self, release=False, owner=0):
        directory = self.root / "var/lib/uflinux/keys"
        directory.mkdir(parents=True, exist_ok=True, mode=0o700)
        private = directory / "uflinux.rsa"
        public = private.with_suffix(".rsa.pub")
        if release:
            raw = os.environ.get("APK_SIGNING_PRIVATE_KEY", "")
            expected = os.environ.get("APK_SIGNING_PUBLIC_KEY", "").replace("\r\n", "\n").strip()
            if not raw or not expected:
                raise RuntimeError("Release signing requires APK_SIGNING_PRIVATE_KEY and APK_SIGNING_PUBLIC_KEY PEM secrets")
            # Never overwrite a compilation key's inode: an unprivileged child
            # could still hold an open descriptor to that old development key.
            os.chown(directory, 0, 0)
            directory.chmod(0o700)
            private.unlink(missing_ok=True)
            fd = os.open(private, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(fd, "w") as stream:
                stream.write(raw.strip() + "\n")
            actual = self.chroot("openssl", "pkey", "-in", "/var/lib/uflinux/keys/uflinux.rsa", "-pubout", stdout=subprocess.PIPE).stdout.decode().strip()
            if actual != expected:
                raise RuntimeError("Release public and private signing keys do not match")
            public.write_text(expected + "\n")
        elif not private.exists():
            self.chroot("openssl", "genrsa", "-out", "/var/lib/uflinux/keys/uflinux.rsa", "3072")
            result = self.chroot("openssl", "rsa", "-in", "/var/lib/uflinux/keys/uflinux.rsa", "-pubout", stdout=subprocess.PIPE)
            public.write_bytes(result.stdout)
        for path in (directory, private, public):
            os.chown(path, owner, owner)
        private.chmod(0o600)
        public.chmod(0o644)
        shutil.copy2(public, self.root / "etc/apk/keys/uflinux.rsa.pub")
        self.env.update(PACKAGER_PRIVKEY="/var/lib/uflinux/keys/uflinux.rsa", PACKAGER_PUBKEY="/var/lib/uflinux/keys/uflinux.rsa.pub")
        return public

    def build_origin(self, origin, env):
        # QEMU user emulation cannot elevate through a target setuid binary.
        # Resolve dependencies as root, but run every compilation/check/package
        # hook unprivileged. Plain abuild still checks that dependencies exist.
        directory = f"cd /project/aports/{origin} && "
        dependency_env = dict(env, SUDO_APK="apk", REPODEST="/home/builder/packages")
        self.chroot("sh", "-ec", directory + "abuild -F deps", extra_env=dependency_env)
        try:
            self.chroot("su", "-s", "/bin/sh", "builder", "-c",
                        directory + "abuild", extra_env=env)
        finally:
            self.chroot("sh", "-ec", directory + "abuild -F undeps", extra_env=dependency_env)

    def package(self):
        self.tools("git", "shadow", "sudo", "alpine-sdk")
        self.chroot("sh", "-ec", "id builder >/dev/null 2>&1 || adduser -D -u 1000 builder; addgroup builder abuild")
        self.signing_key(owner=1000)
        source = self.root / "project/aports"
        for path in (source, *source.rglob("*")):
            os.chown(path, 1000, 1000)
        # Restored Actions caches belong to the host runner, not uid 1000 here.
        self.chroot("chown", "-R", "builder:builder", "/cache/cargo", "/cache/target")
        self.chroot("install", "-d", "-o", "builder", "-g", "builder", "/home/builder/packages", "/home/builder/.abuild")
        self.put("home/builder/.abuild/abuild.conf", 'PACKAGER="UFLinux contributors <noreply@example.invalid>"\nPACKAGER_PRIVKEY="/var/lib/uflinux/keys/uflinux.rsa"\nPACKAGER_PUBKEY="/var/lib/uflinux/keys/uflinux.rsa.pub"\n')
        apkbuild = source / "uflinux/APKBUILD"
        apkbuild.write_text(re.sub(r"^pkgver=.*$", "pkgver=" + self.args.version, apkbuild.read_text(), flags=re.M))
        # Only fetch the selected origin's sources in a package shard.
        selected = (self.args.package,) if getattr(self.args, "package", None) else tuple(PACKAGE_NAMES)
        for origin in ("ironbar", "connman"):
            if origin in selected:
                archive = download(LOCK["sources"][origin], self.cache / "downloads")
                suffix = "tar.gz" if origin == "ironbar" else "tar.xz"
                shutil.copy2(archive, self.root / "var/cache/distfiles" / f"{origin}-{LOCK[origin]['version']}.{suffix}")
        if "uflinux" in selected:
            artwork = download(LOCK["sources"]["gnome-backgrounds"], self.cache / "downloads")
            shutil.copy2(artwork, self.root / "var/cache/distfiles/gnome-backgrounds-3.38.0.tar.xz")
        if "uflinux-settings" in selected:
            import importlib.util
            spec = importlib.util.spec_from_file_location("settings_source", PROJECT / "scripts/settings-source.py")
            source_builder = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(source_builder)
            settings_version, settings_source = source_builder.bundle()
            if settings_version != LOCK["settings"]["version"]:
                raise RuntimeError("Settings version differs from the package version")
            settings_digest = hashlib.sha512(settings_source).hexdigest()
            recipe = source / "uflinux-settings/APKBUILD"
            recipe.write_text(re.sub(r'^sha512sums="[^"]*"',
                                     f'sha512sums="\n{settings_digest}  uflinux-settings-{settings_version}.tar\n"',
                                     recipe.read_text(), flags=re.M))
            (self.root / "var/cache/distfiles" / f"uflinux-settings-{settings_version}.tar").write_bytes(settings_source)
            (source / "uflinux-settings" / f"uflinux-settings-{settings_version}.tar").write_bytes(settings_source)
            (self.cache / "downloads" / (settings_digest + ".tar")).write_bytes(settings_source)
        env = {"CARGO_HOME": "/cache/cargo", "CARGO_TARGET_DIR": "/cache/target",
               "CARGO_BUILD_JOBS": os.environ.get("CARGO_BUILD_JOBS", "1"),
               "VERGEN_GIT_SHA": LOCK["ironbar"]["commit"], "VERGEN_GIT_DESCRIBE": "v" + LOCK["ironbar"]["version"]}
        for package in selected:
            self.build_origin(package, env)
        from sbom import apk_metadata
        versions = {}
        for origin, version in (("connman",LOCK["connman"]["version"]),("ironbar", LOCK["ironbar"]["version"]), ("uflinux-settings", LOCK["settings"]["version"]), ("uflinux", self.args.version)):
            if origin not in selected:
                continue
            recipe = (source / origin / "APKBUILD").read_text()
            revision = re.search(r"^pkgrel=([0-9]+)$", recipe, re.M).group(1)
            versions[origin] = version + "-r" + revision
        if self.packages_dir.exists():
            shutil.rmtree(self.packages_dir)
        self.packages_dir.mkdir(parents=True, exist_ok=True)
        for package in (self.root / "home/builder/packages").rglob("*.apk"):
            info = apk_metadata(package)
            if versions.get(info.get("origin")) == info["pkgver"]:
                shutil.copy2(package, self.packages_dir / package.name)
        (self.out / "metadata" / self.args.arch / "reproducibility.json").unlink(missing_ok=True)
        shutil.copy2(self.root / "etc/apk/keys/uflinux.rsa.pub", self.packages_dir / "build.rsa.pub")
        self.manifest("compile")
        dependency_record = self.cache / self.args.arch / "target/uflinux-build-packages.txt"
        if dependency_record.exists():
            shutil.copy2(dependency_record, self.out / "metadata" / self.args.arch / "ironbar-build-packages.txt")
        inputs = self.out / "inputs" / self.args.arch
        inputs.mkdir(parents=True, exist_ok=True)
        archive_tree(self.cache / self.args.arch / "apk", inputs / "build-apks.tar.gz", self.epoch)
        archive_tree(self.cache / "downloads", inputs / "sources.tar.gz", self.epoch)
        cargo = self.cache / self.args.arch / "cargo"
        names = [str(p.relative_to(cargo)) for p in (cargo / "registry/cache", cargo / "registry/index", cargo / "git/db") if p.exists()]
        if names:
            archive_tree(cargo, inputs / "cargo-sources.tar.gz", self.epoch, names)

    def sign(self):
        if not list(self.packages_dir.glob("*.apk")):
            raise RuntimeError("No package artifacts; run the packages stage first")
        self.tools()
        incoming = self.root / "incoming"
        incoming.mkdir(exist_ok=True)
        key_manifest = self.packages_dir / "build-keys.json"
        keys = json.loads(key_manifest.read_text()) if key_manifest.exists() else None
        if keys is not None and set(keys) != {p.name for p in self.packages_dir.glob("*.apk")}:
            raise ValueError("Build keys do not cover the complete package set")
        shutil.copy2(self.packages_dir / "build.rsa.pub", incoming / "build.rsa.pub")
        # The original signature names the key uflinux.rsa.pub.
        (incoming / "uflinux.rsa.pub").write_bytes((incoming / "build.rsa.pub").read_bytes())
        public = self.signing_key(release=self.args.release)
        repo = self.root / "repo" / self.args.arch
        if repo.exists():
            shutil.rmtree(repo)
        repo.mkdir(parents=True, exist_ok=True)
        for package in sorted(self.packages_dir.glob("*.apk")):
            if keys is not None:
                (incoming / "uflinux.rsa.pub").write_text(keys[package.name])
            copied = self.root / "incoming/package.apk"
            shutil.copy2(package, copied)
            self.chroot("apk", "--keys-dir", "/incoming", "verify", "/incoming/package.apk")
            control, data = split_apk(copied)
            control_path = self.root / "incoming/control.tar.gz"
            control_path.write_bytes(control)
            self.chroot("abuild-sign", "-t", "RSA256", "/incoming/control.tar.gz")
            (repo / package.name).write_bytes(control_path.read_bytes() + data)
            self.chroot("apk", "verify", "/repo/" + self.args.arch + "/" + package.name)
        self.index(repo)
        final = self.out / "repository" / self.args.arch
        if final.exists():
            shutil.rmtree(final)
        shutil.copytree(repo, final)
        shutil.copy2(public, self.out / "repository/uflinux.rsa.pub")
        self.manifest("sign")

    def index(self, repo):
        relative = "/" + str(repo.relative_to(self.root))
        self.chroot("apk", "index", "--description", "UFLinux " + self.args.version,
                    "--rewrite-arch", repo.name, "--no-warnings", "--output", relative + "/APKINDEX.tar.gz",
                    *[relative + "/" + p.name for p in sorted(repo.glob("*.apk"))])
        self.chroot("abuild-sign", "-t", "RSA256", relative + "/APKINDEX.tar.gz")

    def image(self):
        self.tools("xorriso", "mtools", "squashfs-tools", "zstd", "util-linux", "syslinux" if self.args.arch in {"x86", "x86_64"} else "util-linux")
        if self.args.arch == "s390x":
            self.chroot("apk", "add", "s390-tools")
        elif self.args.arch == "ppc64le":
            self.chroot("apk", "add", "grub", "grub-ieee1275")
        for name, digest in LOCK["image"]["patches"].items():
            if hashlib.sha512((PROJECT / "patches" / name).read_bytes()).hexdigest() != digest:
                raise RuntimeError("Image compression patch differs from the source lock: " + name)
        # Keep Alpine's installed helper intact; use a patched image-only copy.
        image_tools = self.root / "opt/uflinux/bin"
        image_tools.mkdir(parents=True, exist_ok=True)
        shutil.copy2(self.root / "usr/sbin/update-kernel", image_tools / "update-kernel")
        self.chroot("patch", "--batch", "--fuzz=0", "-d", "/opt/uflinux/bin", "-p1",
                    "-i", "/project/patches/update-kernel-zstd.patch")
        # mkimage builds its kernel in a separate root. Select our init wrapper
        # without changing Alpine's installed mkinitfs or its configuration.
        wrapper = image_tools / "mkinitfs"
        wrapper.write_text('#!/bin/sh\nexec /sbin/mkinitfs -i /project/aports/uflinux/uflinux-initramfs-init "$@"\n')
        wrapper.chmod(0o755)
        self.signing_key(release=self.args.release)
        final = self.out / "repository" / self.args.arch
        if not final.is_dir():
            raise RuntimeError("Missing signed repository; run the sign stage first")
        repo = self.root / "repo" / self.args.arch
        if repo.exists():
            shutil.rmtree(repo)
        shutil.copytree(final, repo)
        public = self.out / "repository/uflinux.rsa.pub"
        if public.read_bytes() != (self.root / "etc/apk/keys/uflinux.rsa.pub").read_bytes():
            raise RuntimeError("Signing key changed between signing and ISO assembly")
        archive = download(LOCK["sources"]["aports"], self.cache / "downloads")
        upstream = self.root / "opt/aports"
        upstream.mkdir(parents=True, exist_ok=True)
        with tarfile.open(archive) as source:
            for member in source.getmembers():
                parts = Path(member.name).parts
                if len(parts) > 1 and parts[1] == "scripts":
                    member.name = str(Path(*parts[1:]))
                    source.extract(member, upstream, filter="data")
        self.chroot("patch", "--batch", "--forward", "-d", "/opt/aports", "-p1", "-i", "/project/patches/mkimage-apk3-root.patch")
        plugins = self.root / "root/.mkimage"
        plugins.mkdir(parents=True, exist_ok=True)
        shutil.copy2(PROJECT / "profiles/mkimg.uflinux.sh", plugins / "mkimg.uflinux.sh")
        (self.root / "image-out").mkdir(exist_ok=True)
        # mkimage treats an existing APKROOT as initialized, even after a failed run.
        old_cache = self.root / "image-work" / ("apkroot-" + self.args.arch) / "etc/apk/cache"
        if old_cache.is_dir() and not old_cache.is_symlink():
            for path in old_cache.iterdir():
                if path.is_file():
                    link_or_copy(path, self.cache / self.args.arch / "image-apk" / path.name)
        for path in (self.root / "image-work").rglob("*.apk"):
            if path.is_file() and not path.is_symlink():
                promote_apk(path, self.cache / self.args.arch / "image-apk")
        # A development key may change while APK control hashes stay the same.
        # Always fetch custom packages from the freshly signed local repository.
        for path in (self.cache / self.args.arch / "image-apk").glob("*.apk"):
            if path.name.startswith(("uflinux-", "ironbar-", "connman-")):
                path.unlink()
        if (self.root / "image-work").exists():
            shutil.rmtree(self.root / "image-work")
        command = ["sh", "/opt/aports/scripts/mkimage.sh", "--arch", self.args.arch,
                   "--profile", "uflinux", "--tag", self.args.version,
                   "--outdir", "/image-out", "--workdir", "/image-work", "--checksum",
                   "--repository", "/repo", "--repository", f"{LOCK['alpine']['mirror']}/{LOCK['alpine']['branch']}/main",
                   "--repository", f"{LOCK['alpine']['mirror']}/{LOCK['alpine']['branch']}/community"]
        self.chroot(*command, extra_env={"PATH": "/opt/uflinux/bin:" + self.env["PATH"],
                                         "complevel_zstd": "-" + str(LOCK["image"]["initramfs_level"]),
                                         "features_dir": "/project/aports/uflinux /etc/mkinitfs/features.d"})
        target = self.out / "images" / self.args.arch
        target.mkdir(parents=True, exist_ok=True)
        for path in (self.root / "image-out").glob("*"):
            link_or_copy(path, target / path.name)
        caches = self.out / "apk-cache" / self.args.arch
        if caches.exists():
            shutil.rmtree(caches)
        caches.mkdir(parents=True, exist_ok=True)
        contents = {}
        for path in [*(self.cache / self.args.arch / "image-apk").glob("*.apk"), *(self.root / "image-work").rglob("*.apk")]:
            if path.is_symlink() or not path.is_file():
                continue
            if not (caches / path.name).exists():
                digest = sha(path)
                destination = caches / path.name
                link_or_copy(contents.get(digest, path), destination)
                contents[digest] = destination
        self.manifest("image")
        for path in caches.glob("*.apk"):
            promote_apk(path, self.cache / self.args.arch / "image-apk")
        archive_tree(caches, self.out / "inputs" / self.args.arch / "image-apks.tar.gz", self.epoch)
        self.verify_image_compression()
        extracted = self.root / "iso-apks"
        if extracted.exists():
            shutil.rmtree(extracted)
        self.chroot("xorriso", "-osirrox", "on", "-indev", "/image-out/uflinux-" + self.args.version + "-" + self.args.arch + ".iso",
                    "-extract", "/apks/" + self.args.arch, "/iso-apks")
        self.chroot("apk", "verify", *["/iso-apks/" + path.name for path in sorted(extracted.glob("*.apk"))])
        from sbom import generate
        generate(extracted, self.out / "metadata" / self.args.arch / f"uflinux-{self.args.version}-{self.args.arch}.spdx.json", self.args.version, self.args.arch, self.epoch)

    def verify_image_compression(self):
        compression_dir = self.root / "compression-check"
        compression_dir.mkdir(exist_ok=True)
        for name in ("initramfs-lts", "modloop-lts"):
            (compression_dir / name).unlink(missing_ok=True)
            self.chroot("xorriso", "-osirrox", "on", "-indev",
                        "/image-out/uflinux-" + self.args.version + "-" + self.args.arch + ".iso",
                        "-extract", "/boot/" + name, "/compression-check/" + name)
        compression = image_compression(compression_dir / "initramfs-lts", compression_dir / "modloop-lts")
        self.chroot("zstd", "--test", "/compression-check/initramfs-lts")
        report = self.out / "checks" / self.args.arch / "compression.json"
        report.parent.mkdir(parents=True, exist_ok=True)
        report.write_text(json.dumps(compression, indent=2) + "\n")

    def verify(self):
        (self.out / "checks" / self.args.arch / "verified.json").unlink(missing_ok=True)
        # Test the package closure without libraries or defaults from earlier builds.
        with temporary_tree(prefix="verification-", dir=self.root.parent) as directory:
            args = argparse.Namespace(**vars(self.args))
            args.workdir = str(directory)
            runtime = Builder(args)
            runtime.bootstrap()
            with runtime.mounted():
                runtime.verify_installed()

    def verify_installed(self):
        (self.out / "checks" / self.args.arch / "verified.json").unlink(missing_ok=True)
        self.tools("pax-utils", "file", "dbus", "shadow", "wayland-utils", "wlrctl", "wlr-randr", "grim", "procps", "pulseaudio-utils", "libnotify", "py3-gobject3", "py3-atspi", "wtype", "gcc", "musl-dev", "wayland-dev", "libxkbcommon-dev", "desktop-file-utils")
        self.put("etc/apk/keys/uflinux.rsa.pub", (self.out / "repository/uflinux.rsa.pub").read_text())
        repo = self.root / "repo" / self.args.arch
        if repo.exists():
            shutil.rmtree(repo)
        shutil.copytree(self.out / "repository" / self.args.arch, repo)
        self.chroot("apk", "add", "--repository", "/repo", "uflinux", "uflinux-live", "uflinux-installer")
        # Damaging a signed archive must fail verification before installation.
        sample = sorted(repo.glob("uflinux-defaults-*.apk"))[0]
        damaged = bytearray(sample.read_bytes())
        damaged[-12] ^= 1
        (self.root / "tmp/tampered.apk").write_bytes(damaged)
        try:
            self.chroot("apk", "verify", "/tmp/tampered.apk")
        except subprocess.CalledProcessError:
            pass
        else:
            raise RuntimeError("apk accepted a damaged signed archive")
        settings = self.root / "etc/xdg/ironbar/style.css"
        before = settings.read_text()
        settings.write_text(before + "\n/* UFLinux upgrade preservation check */\n")
        self.chroot("apk", "fix", "--reinstall", "--repository", "/repo", "uflinux-defaults")
        if "UFLinux upgrade preservation check" not in settings.read_text():
            raise RuntimeError("apk replaced modified system defaults")
        settings.write_text(before)
        self.chroot("ironbar", "--config", "/etc/xdg/ironbar/config.toml", "--theme", "/etc/xdg/ironbar/style.css", "--validate-config")
        self.chroot("sh", "-ec", "id smoke >/dev/null 2>&1 || adduser -D -u 2000 smoke; addgroup smoke netdev")
        report = self.out / "checks" / self.args.arch
        report.mkdir(parents=True, exist_ok=True)
        try:
            with self.verification_services(report):
                self.chroot("su", "-s", "/bin/sh", "smoke", "-c", "UFLINUX_TEST_CONNMAN_FIXTURE=1 sh -x /project/tests/desktop-smoke.sh")
        finally:
            results = self.root / "tmp/uflinux-smoke-results"
            if results.exists():
                shutil.copytree(results, report / "desktop", dirs_exist_ok=True)
            results = self.root / "tmp/uflinux-settings-smoke-results"
            if results.exists():
                shutil.copytree(results, report / "settings", dirs_exist_ok=True)
        self.chroot("python3", "-m", "unittest", "discover", "-s", "/project/tests", "-p", "test_installer.py")
        self.chroot("python3", "/project/tests/power-acpid-smoke.py")
        self.chroot("scanelf", "--needed", "/usr/bin/ironbar", stdout=(report / "elf.txt").open("w"))
        self.chroot("ldd", "/usr/bin/ironbar", stdout=(report / "libraries.txt").open("w"))
        binary = self.root / "usr/bin/ironbar"
        header = binary.read_bytes()[:64]
        if header[:4] != b"\x7fELF":
            raise RuntimeError("Ironbar is not an ELF executable")
        byte_order = "<" if header[5] == 1 else ">"
        elf_type, machine = struct.unpack_from(byte_order + "HH", header, 16)
        expected = {"x86": 3, "x86_64": 62, "armv7": 40, "aarch64": 183,
                    "ppc64le": 21, "s390x": 22, "riscv64": 243}[self.args.arch]
        if machine != expected or elf_type != 3:
            raise RuntimeError("Ironbar must be a position-independent executable for the target architecture")
        settings_binaries = {}
        for relative in ("usr/bin/uflinux-start-menu", "usr/bin/uflinux-settings", "usr/bin/uflinux-settingsctl",
                         "usr/libexec/uflinux-settingsd", "usr/libexec/uflinux-settings-system", "usr/sbin/connmand"):
            path = self.root / relative
            data = path.read_bytes()[:64]
            order = "<" if data[5] == 1 else ">"
            if data[:4] != b"\x7fELF" or struct.unpack_from(order + "HH", data, 16) != (3, expected):
                raise RuntimeError("Wrong architecture or non-PIE settings executable: " + relative)
            libraries = self.chroot("ldd", "/" + relative, stdout=subprocess.PIPE).stdout.decode()
            if relative.startswith("usr/libexec/") and any(name in libraries for name in ("libgtk-4", "libadwaita")):
                raise RuntimeError("A background settings service links GTK")
            (report / (path.name + "-libraries.txt")).write_text(libraries)
            settings_binaries[path.name] = {"sha256": sha(path), "bytes": path.stat().st_size}
        (report / "verified.json").write_text(json.dumps({"arch": self.args.arch, "version": self.args.version,
            "desktop": "headless smoke passed", "ironbar_bytes": binary.stat().st_size, "elf_machine": machine,
            "signature_rejection": True, "configuration_preserved_on_reinstall": True, "binary_sha256": sha(binary),
            "verified_packages": {p.name: sha(p) for p in sorted(repo.glob("*.apk"))},
            "settings_binaries": settings_binaries,
            "signing": "release" if self.args.release else "development"}, indent=2) + "\n")
        self.manifest("verify")

    @contextlib.contextmanager
    def verification_services(self, report):
        """Own exact child PIDs; never kill unrelated host D-Bus processes by name."""
        processes = []
        logs = []
        try:
            (self.root / "run/dbus").mkdir(parents=True, exist_ok=True)
            for name, command in (("system-bus", ["dbus-daemon", "--system", "--nofork", "--nopidfile"]),
                                  ("connman-fixture", ["python3", "/project/tests/mock-connman.py"]),
                                  ("login1-fixture", ["python3", "/project/tests/mock-login1.py"])):
                log = (report / (name + ".log")).open("w")
                logs.append(log)
                process = subprocess.Popen(["chroot", str(self.root), "/usr/bin/env", "-i",
                    *[f"{key}={value}" for key, value in self.env.items()], *command], stdout=log, stderr=log)
                processes.append(process)
                time.sleep(0.4)
                if process.poll() is not None:
                    raise RuntimeError("Verification service failed to start: " + name)
            yield
        finally:
            for process in reversed(processes):
                if process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait()
            for log in logs:
                log.close()

    def manifest(self, stage):
        directory = self.out / "metadata" / self.args.arch
        directory.mkdir(parents=True, exist_ok=True)
        self.chroot("apk", "info", "-v", stdout=(directory / (stage + "-packages.txt")).open("w"))
        files = {}
        for base in (self.packages_dir, self.out / "repository" / self.args.arch, self.out / "apk-cache" / self.args.arch):
            if base.exists():
                for path in base.glob("*.apk"):
                    files[str(path.relative_to(self.out))] = {"sha256": sha(path), "bytes": path.stat().st_size}
        metadata = {"schema": 1, "arch": self.args.arch, "version": self.args.version,
                    "stage": stage, "package": getattr(self.args, "package", None), "source_date_epoch": int(self.epoch), "sources": LOCK,
                    "repository_url": self.url, "signing": "release" if self.args.release else "development",
                    "files": files, "host": platform.platform(), "source_commit": os.environ.get("GITHUB_SHA", "local")}
        if stage == "image":
            iso = self.out / "images" / self.args.arch / f"uflinux-{self.args.version}-{self.args.arch}.iso"
            metadata["iso_sha256"] = sha(iso)
        (directory / (stage + ".json")).write_text(json.dumps(metadata, indent=2, sort_keys=True) + "\n")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--arch", choices=ARCHES, required=True)
    parser.add_argument("--version", default=LOCK["distribution"]["version"])
    parser.add_argument("--outdir", default=str(PROJECT / "out"))
    parser.add_argument("--package", choices=PACKAGE_NAMES, help="Compile one independent package origin (packages/bootstrap stages only)")
    parser.add_argument("--workdir")
    parser.add_argument("--cache")
    parser.add_argument("--repository-url", default=os.environ.get("APK_REPOSITORY_URL"))
    parser.add_argument("--stage", choices=["bootstrap", "packages", "sign", "image", "verify", "all"], default="all")
    parser.add_argument("--release", action="store_true")
    args = parser.parse_args(argv)
    if args.package and (args.stage not in {"bootstrap", "packages"} or args.release):
        parser.error("--package requires --stage packages or bootstrap, without --release")
    args.workdir = args.workdir or str(PROJECT / ".state" / args.package if args.package else PROJECT / ".state")
    args.cache = args.cache or str(PROJECT / ".cache/build" / args.package if args.package else PROJECT / ".cache/build")
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:_(?:alpha|beta|rc)[0-9]+)?", args.version):
        parser.error("Use an Alpine-compatible version, e.g. 0.1.0 or 0.1.0_rc1")
    if os.geteuid() != 0:
        parser.error("Run on a disposable Linux build host as root (sudo scripts/build.sh ...)")
    builder = Builder(args)
    try:
        builder.bootstrap()
        with builder.mounted():
            for stage in (["packages", "sign", "image", "verify"] if args.stage == "all" else [args.stage]):
                if stage != "bootstrap":
                    try:
                        getattr(builder, "package" if stage == "packages" else stage)()
                    finally:
                        if args.release and stage in {"sign", "image"}:
                            (builder.root / "var/lib/uflinux/keys/uflinux.rsa").unlink(missing_ok=True)
    except (OSError, RuntimeError, ValueError, subprocess.CalledProcessError) as exc:
        print(f"uflinux-build: {exc}", file=sys.stderr)
        return 1
    finally:
        if args.release:
            # Verification executes the packaged binary after the release key is gone.
            (builder.root / "var/lib/uflinux/keys/uflinux.rsa").unlink(missing_ok=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
