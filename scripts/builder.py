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
DEVICES = json.loads((PROJECT / "config/devices.json").read_text())
DEVICE_LOCK = json.loads((PROJECT / "config/devices.lock.json").read_text())
ESP_START = 2048
ESP_SECTORS = 1048576
SECTOR = 512
GPT_ESP_TYPE = "C12A7328-F81F-11D2-BA4B-00A0C93EC9B0"
GPT_LINUX_TYPE = "0FC63DAF-8483-477B-8E3F-43138715FD6D"
# The device rootfs is assembled from a throwaway chroot whose transient mounts
# and build inputs must not become part of the installed filesystem.
DEVICE_ROOT_EXCLUDES = ("proc", "sys", "dev", "cache", "repo", "nura", "project",
                        "device-esp", "device-image", ".uflinux-bootstrap", "etc/apk/cache")
DEVICE_ESP = "/device-esp"
DEVICE_PROJECT_DIRS = ("aports", "profiles", "config", "scripts", "patches")
PROJECT_DIRS = ("aports", "profiles", "config", "scripts", "tests", "patches", "settings")
# Mirrors the table in profiles/device-rootfs.sh: systemd-boot ships one bare
# EFI binary per architecture and the ESP has to carry it under that name.
EFI_SUFFIX = {"aarch64": "aa64", "x86_64": "x64"}
# A device image is several gigabytes of already-compressed packages, so the
# highest level would cost hours of CPU for a few percent of size.
DEVICE_IMAGE_ZSTD_LEVEL = 12
# Stage names as they appear on the command line, mapped to Builder methods.
STAGE_METHODS = {"packages": "package", "device": "device_image"}


def device_target(devices, name, arch):
    device = devices[name]
    if arch not in device["arches"]:
        raise ValueError(f"device {name} does not support {arch}")
    return device


def device_image_name(version, name, arch):
    return f"uflinux-{version}-{name}-{arch}.img.zst"


def device_uuids(seed):
    """Deterministic disk, partition and filesystem identifiers for one build."""
    def uuid(role):
        digest = hashlib.sha256(f"{seed}:{role}".encode()).hexdigest()[:32]
        return f"{digest[0:8]}-{digest[8:12]}-{digest[12:16]}-{digest[16:20]}-{digest[20:32]}".upper()
    serial = hashlib.sha256(f"{seed}:serial".encode()).hexdigest()[0:8].lower()
    return {"disk": uuid("disk"), "esp": uuid("esp"), "root": uuid("root"), "serial": serial}


def device_partitions(size_mib):
    """ESP and root partition geometry in 512 byte sectors."""
    total = size_mib * (1024 * 1024 // SECTOR)
    if size_mib * 1024 * 1024 % SECTOR or total < ESP_SECTORS + 4096:
        raise ValueError(f"invalid image size {size_mib} MiB")
    root_start = ESP_START + ESP_SECTORS
    return {"esp_start": ESP_START, "esp_size": ESP_SECTORS, "root_start": root_start,
            "root_size": total - root_start - ESP_START, "total": total}


def device_partition_script(layout, uuids):
    return (
        "label: gpt\n"
        f"label-id: {uuids['disk']}\n"
        f"start={layout['esp_start']}, size={layout['esp_size']}, type={GPT_ESP_TYPE}, "
        f"uuid={uuids['esp']}, name=ESP, bootable\n"
        f"start={layout['root_start']}, size={layout['root_size']}, type={GPT_LINUX_TYPE}, "
        f"uuid={uuids['root']}, name=ROOT\n"
    )


def device_repositories(url, arch, nura=None):
    """Package sources for a device rootfs: release repo, Alpine, then optional Nura."""
    alpine = LOCK["alpine"]
    lines = [url, f"{alpine['mirror']}/{alpine['branch']}/main",
             f"{alpine['mirror']}/{alpine['branch']}/community"]
    if nura:
        lines.append(f"{nura['mirror']}/{nura['branch']}/{arch}")
    return "\n".join(lines) + "\n"


def device_packages(device, arch):
    """Everything the device rootfs needs beyond the live system's package set."""
    packages = ["uflinux-desktop-meta", "uflinux-device"] + list(device["packages"])
    # The same desktop the live ISO installs, minus the live-only pieces.
    packages += ["musl-locales", "musl-locales-lang", "glib-lang", "gtk4.0-lang",
                 "labwc-lang", "firefox", "ublock-origin", "vulkan-loader", "xfsprogs"]
    packages += ["alpine-base", "apk-tools", "zstd", "util-linux", "dosfstools", "mtools",
                 "e2fsprogs", "e2fsprogs-extra", "cloud-utils-growpart", "linux-firmware"]
    if device["nura"]:
        packages += list(device["nura_packages"])
    if arch in ("x86_64", "aarch64") and "efibootmgr" not in packages:
        packages.append("efibootmgr")
    return sorted(set(packages))


def device_environment(name, device, arch, version, root_partuuid, esp):
    """Contract consumed by profiles/device-rootfs.sh."""
    env = {"UFLINUX_DEVICE": name, "UFLINUX_ARCH": arch,
           "UFLINUX_BOOT": device["boot"], "UFLINUX_SERIAL": device["serial"],
           "UFLINUX_CMDLINE": device["cmdline"], "UFLINUX_VERSION": version,
           "UFLINUX_ROOT_PARTUUID": root_partuuid, "UFLINUX_ESP": esp,
           "UFLINUX_PACKAGES": " ".join(device_packages(device, arch))}
    if device["nura"]:
        env["UFLINUX_NURA_PACKAGES"] = " ".join(device["nura_packages"])
        # The Nura dtbloader EFI driver installs a board DTB that the Alpine kernel
        # does not ship (the Dragon Q6A among them), so the ESP has to carry the tree.
        env["UFLINUX_DTBLOADER"] = "/usr/share/dtbloader/dtbloader.efi"
    if device["boot"] == "extlinux":
        env["UFLINUX_DTB"] = device["dtb"]
    return env


def device_snapshot(source, destination):
    """Copy an installed rootfs into an image staging tree, minus build artefacts."""
    destination.mkdir(parents=True, exist_ok=True)
    excludes = [f"--exclude=./{name}" for name in DEVICE_ROOT_EXCLUDES]
    reader = subprocess.Popen(("tar", "-C", str(source), *excludes, "-p", "-f", "-", "-c", "."),
                              stdout=subprocess.PIPE)
    writer = subprocess.Popen(("tar", "-C", str(destination), "--numeric-owner",
                               "-xpf", "-"), stdin=reader.stdout)
    reader.stdout.close()
    writer.communicate()
    reader.wait()
    if reader.returncode or writer.returncode:
        raise RuntimeError(f"failed to snapshot {source}")
    return destination


def device_extract(image, destination, esp_start=ESP_START):
    """Read the ESP of an assembled disk image without mounting anything."""
    destination.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, MTOOLS_SKIP_CHECK="1")
    run(["mcopy", "-s", "-n", "-i", f"{image}@@{esp_start}s::", str(destination) + "/"], env=env)
    return destination


def device_layout(image):
    """Return the GPT geometry of *image* as reported by sfdisk."""
    result = run(["sfdisk", "--dump", "--json", str(image)], stdout=subprocess.PIPE, text=True)
    table = json.loads(result.stdout).get("partitiontable", {})
    if table.get("unit") != "sectors" or int(table.get("sectorsize", 0)) != SECTOR:
        raise ValueError(f"{image} is not a {SECTOR}-byte sector disk")
    partitions = table.get("partitions", [])
    if len(partitions) != 2:
        raise ValueError(f"{image} must have an ESP and a root partition")
    esp, root = partitions
    if esp.get("type", "").upper() != GPT_ESP_TYPE or root.get("type", "").upper() != GPT_LINUX_TYPE:
        raise ValueError(f"{image} has the wrong partition types")
    return {"disk": table.get("id", "").upper(), "esp": esp, "root": root}


def device_verify_image(image, device, arch, uuids, layout):
    """Check the assembled disk: geometry, boot files and a Zstd initramfs."""
    actual = device_layout(image)
    for name, expected in (("ESP", layout["esp_start"]), ("ROOT", layout["root_start"])):
        found = (actual["esp"] if name == "ESP" else actual["root"])["start"]
        if int(found) != expected:
            raise ValueError(f"{image}: {name} starts at sector {found}, expected {expected}")
    for key, role in (("esp", "ESP"), ("root", "ROOT")):
        if actual[key].get("uuid", "").upper() != uuids[key]:
            raise ValueError(f"{image}: {role} partition UUID changed")
    if actual["disk"] != uuids["disk"]:
        raise ValueError(f"{image}: disk GUID changed")
    stage = image.parent / (image.name + "-esp")
    device_extract(image, stage)
    boot = {}
    for name in ("boot/vmlinuz", "boot/initramfs"):
        path = stage / name
        if not path.is_file():
            raise ValueError(f"{image}: the ESP has no {name}")
        boot[name.split("/", 1)[1] + "_bytes"] = path.stat().st_size
    with (stage / "boot/initramfs").open("rb") as handle:
        magic = handle.read(4)
    if magic != b"\x28\xb5\x2f\xfd":
        raise ValueError(f"{image}: the initramfs is not a Zstandard frame")
    bootloader = {"efi": "EFI/boot/boot" + EFI_SUFFIX[arch] + ".efi",
                  "rpi": "cmdline.txt",
                  "extlinux": "extlinux/extlinux.conf"}[device["boot"]]
    if not (stage / bootloader).is_file():
        raise ValueError(f"{image}: the ESP has no {bootloader}")
    remove_tree(stage)
    return {"device_image_format": "gpt-ext4", "initramfs_compression": "zstd",
            "arch": arch, "boot": device["boot"], "disk_guid": actual["disk"],
            "partitions": {"esp": {"start": layout["esp_start"], "size": layout["esp_size"],
                                   "uuid": uuids["esp"]},
                           "root": {"start": layout["root_start"], "size": layout["root_size"],
                                    "uuid": uuids["root"]}},
                   **boot}
# apk v2 has no ECDSA signature: apk-tools 3.0.9 knows only the stream names
# RSA512, RSA256, RSA and DSA, and resolves a trusted key by its filename from a
# plain public-key PEM (an X.509 certificate is not accepted). UFLinux therefore
# signs with an EC P-256 key carried in the SHA-256 stream, whose name is fixed
# by that format limitation rather than by the algorithm actually used.
SIGN_STREAM = "RSA256"
SIGNING_KEY = "uflinux.ec"
SIGNING_PRIVATE = "/var/lib/uflinux/keys/" + SIGNING_KEY
SIGNING_PUBLIC = SIGNING_PRIVATE + ".pub"
TRUSTED_KEY = "etc/apk/keys/" + SIGNING_KEY + ".pub"
BUILD_KEY = "build.ec.pub"
PACKAGE_NAMES = {
    "connman": {"connman", "connman-dbg", "connman-dev", "connman-doc", "connman-openrc"},
    "uflinux-regreet": {"uflinux-regreet"},
    "gpu-screen-recorder": {"gpu-screen-recorder", "gpu-screen-recorder-dbg",
                            "gpu-screen-recorder-dev", "gpu-screen-recorder-doc"},
    "uflinux-settings": {"uflinux-settings", "uflinux-settings-daemon", "uflinux-settings-dbg",
                         "uflinux-settings-doc", "uflinux-settings-system", "uflinux-settings-system-openrc", "uflinux-installer-ui", "uflinux-desktop", "uflinux-screencapture"},
    "uflinux": {"uflinux", "uflinux-defaults", "uflinux-desktop-meta", "uflinux-installer",
                "uflinux-live", "uflinux-live-openrc", "uflinux-release", "uflinux-bootsplash-openrc",
                "uflinux-initramfs", "uflinux-device"},
}


def omit_modloop_firmware(update_kernel):
    """Adjust the image-only helper after its existing compression patch."""
    marker = "# Refuse an image that its selected kernel cannot decompress."
    if update_kernel.count(marker) != 1:
        raise ValueError("Unexpected update-kernel compression section")
    cleanup = r'''# Firmware is supplied by the live root's linux-firmware packages. Preserve
# wireless-regdb, which is a separate package and not in linux-firmware.
if [ -d "$MODLOOP/modules/firmware" ]; then
    find "$MODLOOP/modules/firmware" -mindepth 1 -maxdepth 1 \
        ! -name 'regulatory.db*' -exec rm -rf {} +
fi

'''
    return update_kernel.replace(marker, cleanup + marker, 1)


def check_modloop_firmware_listing(listing):
    """Only the separately packaged wireless regulatory database may remain."""
    prefix = "squashfs-root/modules/firmware/"
    for line in listing.splitlines():
        if prefix not in line:
            continue
        relative = line.split(prefix, 1)[1].split(" -> ", 1)[0]
        if "/" in relative or not relative.startswith("regulatory.db"):
            raise ValueError("Device firmware remains in the modloop: " + relative)


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
    return download_pinned(spec["url"], expected, algorithm, cache / (expected + ".tar.gz"))


def download_pinned(url, expected, algorithm, path):
    """Fetch one file into *path*, accepting an already cached copy only if it verifies."""
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    if not path.exists():
        temporary = path.with_suffix(".partial")
        for attempt in range(3):
            try:
                with urllib.request.urlopen(url, timeout=120) as response, temporary.open("wb") as stream:
                    shutil.copyfileobj(response, stream)
                if sha(temporary, algorithm) != expected:
                    raise RuntimeError("Checksum mismatch: " + url)
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


def signature_names(path):
    """The .SIGN.<stream>.<key> names carried by an APK, in file order."""
    names = []
    for _, raw in gzip_members(Path(path).read_bytes()):
        with tarfile.open(fileobj=io.BytesIO(raw)) as archive:
            names.extend(name for name in archive.getnames() if name.startswith(".SIGN."))
    return names


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
                    "sigtype": SIGN_STREAM, "ARCH": args.arch}
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
        for name in PROJECT_DIRS:
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
            # Do not propagate nested device binds into the host's /dev or other
            # build roots when the runner uses shared mount propagation.
            run(["mount", "--make-private", self.root / "dev"])
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
        private = directory / SIGNING_KEY
        public = directory / (SIGNING_KEY + ".pub")
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
            actual = self.chroot("openssl", "pkey", "-in", SIGNING_PRIVATE, "-pubout", stdout=subprocess.PIPE).stdout.decode().strip()
            if actual != expected:
                raise RuntimeError("Release public and private signing keys do not match")
            # apk resolves a trusted key by filename alone, so a key published as
            # uflinux.ec.pub has to be the algorithm that name advertises.
            described = self.chroot("openssl", "pkey", "-in", SIGNING_PRIVATE,
                                    "-text_pub", "-noout", stdout=subprocess.PIPE)
            if "ASN1 OID: prime256v1" not in described.stdout.decode():
                raise RuntimeError("The release signing key is not an EC P-256 key")
            public.write_text(expected + "\n")
        elif not private.exists():
            self.chroot("openssl", "genpkey", "-algorithm", "EC",
                        "-pkeyopt", "ec_paramgen_curve:P-256", "-out", SIGNING_PRIVATE)
            result = self.chroot("openssl", "pkey", "-in", SIGNING_PRIVATE, "-pubout", stdout=subprocess.PIPE)
            public.write_bytes(result.stdout)
        for path in (directory, private, public):
            os.chown(path, owner, owner)
        private.chmod(0o600)
        public.chmod(0o644)
        shutil.copy2(public, self.root / TRUSTED_KEY)
        self.env.update(PACKAGER_PRIVKEY=SIGNING_PRIVATE, PACKAGER_PUBKEY=SIGNING_PUBLIC)
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
        self.put("home/builder/.abuild/abuild.conf", 'PACKAGER="UFLinux contributors <noreply@example.invalid>"\nPACKAGER_PRIVKEY="' + SIGNING_PRIVATE + '"\nPACKAGER_PUBKEY="' + SIGNING_PUBLIC + '"\n')
        apkbuild = source / "uflinux/APKBUILD"
        apkbuild.write_text(re.sub(r"^pkgver=.*$", "pkgver=" + self.args.version, apkbuild.read_text(), flags=re.M))
        # Only fetch the selected origin's sources in a package shard.
        selected = (self.args.package,) if getattr(self.args, "package", None) else tuple(PACKAGE_NAMES)
        for origin in ("connman", "uflinux-regreet", "gpu-screen-recorder"):
            if origin in selected:
                archive = download(LOCK["sources"][origin], self.cache / "downloads")
                suffix = "tar.xz" if origin == "connman" else "tar.gz"
                shutil.copy2(archive, self.root / "var/cache/distfiles" / f"{origin}-{LOCK[origin]['version']}.{suffix}")
        if "uflinux" in selected:
            artwork = download(LOCK["sources"]["gnome-backgrounds"], self.cache / "downloads")
            shutil.copy2(artwork, self.root / "var/cache/distfiles/gnome-backgrounds-3.38.0.tar.xz")
            theme = download(LOCK["sources"]["labwc-themes"], self.cache / "downloads")
            shutil.copy2(theme, self.root / "var/cache/distfiles/labwc-themes-6a2e3634.tar.gz")
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
               "CARGO_BUILD_JOBS": os.environ.get("CARGO_BUILD_JOBS", "1")}
        for package in selected:
            self.build_origin(package, env)
        from sbom import apk_metadata
        versions = {}
        for origin, version in (("uflinux-regreet", LOCK["uflinux-regreet"]["version"]), ("connman",LOCK["connman"]["version"]), ("gpu-screen-recorder", LOCK["gpu-screen-recorder"]["version"]), ("uflinux-settings", LOCK["settings"]["version"]), ("uflinux", self.args.version)):
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
        shutil.copy2(self.root / TRUSTED_KEY, self.packages_dir / BUILD_KEY)
        self.manifest("compile")
        dependency_record = self.cache / self.args.arch / "target/uflinux-build-packages.txt"
        if dependency_record.exists():
            shutil.copy2(dependency_record, self.out / "metadata" / self.args.arch / "cargo-build-packages.txt")
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
        shutil.copy2(self.packages_dir / BUILD_KEY, incoming / BUILD_KEY)
        # The original signature names the key by its published filename.
        (incoming / (SIGNING_KEY + ".pub")).write_bytes((incoming / BUILD_KEY).read_bytes())
        public = self.signing_key(release=self.args.release)
        repo = self.root / "repo" / self.args.arch
        if repo.exists():
            shutil.rmtree(repo)
        repo.mkdir(parents=True, exist_ok=True)
        for package in sorted(self.packages_dir.glob("*.apk")):
            if keys is not None:
                (incoming / (SIGNING_KEY + ".pub")).write_text(keys[package.name])
            copied = self.root / "incoming/package.apk"
            shutil.copy2(package, copied)
            self.chroot("apk", "--keys-dir", "/incoming", "verify", "/incoming/package.apk")
            control, data = split_apk(copied)
            control_path = self.root / "incoming/control.tar.gz"
            control_path.write_bytes(control)
            self.chroot("abuild-sign", "-t", SIGN_STREAM, "/incoming/control.tar.gz")
            (repo / package.name).write_bytes(control_path.read_bytes() + data)
            self.chroot("apk", "verify", "/repo/" + self.args.arch + "/" + package.name)
        self.index(repo)
        final = self.out / "repository" / self.args.arch
        if final.exists():
            shutil.rmtree(final)
        shutil.copytree(repo, final)
        shutil.copy2(public, self.out / "repository" / (SIGNING_KEY + ".pub"))
        self.manifest("sign")

    def index(self, repo):
        relative = "/" + str(repo.relative_to(self.root))
        self.chroot("apk", "index", "--description", "UFLinux " + self.args.version,
                    "--rewrite-arch", repo.name, "--no-warnings", "--output", relative + "/APKINDEX.tar.gz",
                    *[relative + "/" + p.name for p in sorted(repo.glob("*.apk"))])
        self.chroot("abuild-sign", "-t", SIGN_STREAM, relative + "/APKINDEX.tar.gz")

    def image(self):
        self.tools("xorriso", "mtools", "squashfs-tools", "zstd", "util-linux", "syslinux" if self.args.arch in {"x86", "x86_64"} else "util-linux")
        if self.args.arch == "s390x":
            self.chroot("apk", "add", "s390-tools", "file")
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
        update_kernel = image_tools / "update-kernel"
        update_kernel.write_text(omit_modloop_firmware(update_kernel.read_text()))
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
        public = self.out / "repository" / (SIGNING_KEY + ".pub")
        if public.read_bytes() != (self.root / TRUSTED_KEY).read_bytes():
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
            if path.name.startswith(("uflinux-", "connman-", "gpu-screen-recorder-")):
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
        listing = self.chroot("unsquashfs", "-ll", "/compression-check/modloop-lts",
                              stdout=subprocess.PIPE, text=True).stdout
        check_modloop_firmware_listing(listing)
        compression["modloop_device_firmware"] = False
        self.chroot("zstd", "--test", "/compression-check/initramfs-lts")
        report = self.out / "checks" / self.args.arch / "compression.json"
        report.parent.mkdir(parents=True, exist_ok=True)
        report.write_text(json.dumps(compression, indent=2) + "\n")

    def device_bootstrap(self, device):
        """Prepare an installed rootfs for *device*: repos, keys and /project."""
        arch = self.args.arch
        root = self.root
        fingerprint = hashlib.sha256(json.dumps([LOCK, DEVICE_LOCK, device],
                                                sort_keys=True).encode()).hexdigest()
        marker = root / ".uflinux-bootstrap"
        if marker.exists() and marker.read_text() != fingerprint:
            raise RuntimeError("Device build inputs changed: use a new --workdir")
        root.mkdir(parents=True, exist_ok=True)
        if not marker.exists():
            if any(root.iterdir()):
                raise RuntimeError("Incomplete device rootfs: remove only this device's "
                                   "workdir and retry")
            archive = download(LOCK["rootfs"][arch], self.cache / "downloads")
            run(["tar", "--numeric-owner", "-xpf", archive, "-C", root])
        marker.write_text(fingerprint)
        resolv = root / "etc/resolv.conf"
        resolv.unlink(missing_ok=True)
        resolv.write_bytes(Path("/etc/resolv.conf").read_bytes())
        # Executing apk is also a check of the kernel's binfmt registration.
        self.chroot("apk", "--print-arch")
        nura = DEVICE_LOCK["nura"] if device["nura"] else None
        if nura and arch not in nura["arches"]:
            raise ValueError(f"the Nura repository has no {arch} suite")
        keys = root / "etc/apk/keys"
        keys.mkdir(parents=True, exist_ok=True)
        for source in Path("/usr/share/apk/keys").glob(arch + "/*"):
            shutil.copy2(source, keys / source.name)
        public = self.out / "repository" / (SIGNING_KEY + ".pub")
        if not public.is_file():
            raise RuntimeError("Missing signed repository; run the sign stage first")
        self.put(TRUSTED_KEY, public.read_text())
        if nura:
            # The key file name is what apk records in the signature check.
            name = urllib.parse.urlparse(nura["repository_key"]).path.rsplit("/", 1)[-1]
            key = self.cache / "downloads" / ("nura-" + name)
            download_pinned(nura["repository_key"], nura["repository_key_sha256"], "sha256", key)
            (keys / name).write_bytes(key.read_bytes())
            (keys / name).chmod(0o644)
            index = nura["index_sha256"][arch]
            download_pinned(f"{nura['mirror']}/{nura['branch']}/{arch}/APKINDEX.tar.gz", index,
                            "sha256", self.cache / "downloads" / f"nura-APKINDEX-{arch}.tar.gz")
        self.put("etc/apk/repositories", device_repositories(self.url, arch, nura))
        for name in DEVICE_PROJECT_DIRS:
            destination = root / "project" / name
            shutil.rmtree(destination, ignore_errors=True)
            shutil.copytree(PROJECT / name, destination,
                            ignore=shutil.ignore_patterns("__pycache__", "*.pyc"))
        (root / "project/LICENSE").write_bytes((PROJECT / "LICENSE").read_bytes())
        self.put("project/repository-url", self.url + "\n")
        # No other stage refreshes the indexes: apk 3 needs them before the first add.
        self.chroot("apk", "update")

    def device_image(self):
        """Assemble one installed GPT disk image for the selected device."""
        name = self.args.device
        device = device_target(DEVICES, name, self.args.arch)
        arch = self.args.arch
        self.tools("dosfstools", "mtools", "zstd", "util-linux")
        uuids = device_uuids(f"{self.args.version}:{name}:{arch}")
        layout = device_partitions(device["img_size_mib"])
        args = argparse.Namespace(**vars(self.args))
        args.workdir = str(Path(self.args.workdir).resolve() / "device" / name)
        # The device rootfs pulls kernels and firmware that no other stage uses;
        # keeping them out of the shared cache keeps the ISO inputs unchanged.
        args.cache = str(Path(self.args.cache).resolve() / "device" / name)
        root = Builder(args)
        root.device_bootstrap(device)
        with root.mounted():
            root.chroot("mkdir", "-p", DEVICE_ESP)
            root.chroot("sh", "/project/profiles/device-rootfs.sh", extra_env=dict(
                device_environment(name, device, arch, self.args.version, uuids["root"],
                                   DEVICE_ESP),
                features_dir="/project/aports/uflinux /etc/mkinitfs/features.d",
                complevel_zstd="-" + str(LOCK["image"]["initramfs_level"])))
            root.device_info = {"device": name, "device_label": device["label"],
                                "device_status": device["status"]}
            root.manifest("device")
            snapshot = self.root / "device-image" / name
            remove_tree(snapshot)
            staging = snapshot / "rootfs"
            esp = snapshot / "esp"
            device_snapshot(root.root, staging)
            device_snapshot(root.root / DEVICE_ESP.lstrip("/"), esp)
        disk = snapshot / "disk"
        disk.parent.mkdir(parents=True, exist_ok=True)
        disk.unlink(missing_ok=True)
        run(["truncate", "-s", f"{device['img_size_mib']}M", disk])
        run(["sfdisk", "--quiet", "--force", str(disk)],
            input=device_partition_script(layout, uuids), text=True)
        # The explicit block count bounds the FAT to its partition; mkfs.vfat
        # notices the disk is larger and warns, which is what we want here.
        run(["mkfs.vfat", "-n", "UFLINUX", "-i", uuids["serial"], "-F", "32", "-S", "512",
             "--offset", str(layout["esp_start"]), str(disk), str(layout["esp_size"] // 2)],
            stderr=subprocess.DEVNULL)
        mcopy = ["mcopy", "-s", "-n", "-i", f"{disk}@@{layout['esp_start']}s::"]
        mcopy += sorted(str(path) for path in esp.iterdir())
        run(mcopy + ["::"], env=dict(os.environ, MTOOLS_SKIP_CHECK="1"))
        run(["mke2fs", "-q", "-Fn", "-t", "ext4", "-d", staging, "-L", "UFLINUX_ROOT",
             "-U", uuids["root"], "-E",
             f"offset={layout['root_start'] * SECTOR},root_owner=0:0,lazy_itable_init=0",
             str(disk), str(layout["root_size"] // 2)])
        report = device_verify_image(disk, device, arch, uuids, layout)
        target = self.out / "devices" / arch
        target.mkdir(parents=True, exist_ok=True)
        image = target / device_image_name(self.args.version, name, arch)
        partial = image.with_name(image.name + ".partial")
        with partial.open("wb") as handle:
            run(["zstd", "-q", "-f", f"-{DEVICE_IMAGE_ZSTD_LEVEL}", "-T0", "-c", str(disk)],
                stdout=handle)
        partial.replace(image)
        digest = sha(image)
        (target / (image.name + ".sha256")).write_text(f"{digest}  {image.name}\n")
        report.update({"device": name, "image": str(image.relative_to(self.out)),
                       "sha256": digest, "bytes": image.stat().st_size,
                       "uncompressed_bytes": layout["total"] * SECTOR})
        checks = self.out / "checks" / arch / f"device-{name}.json"
        checks.parent.mkdir(parents=True, exist_ok=True)
        checks.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
        remove_tree(snapshot)

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
        self.put(TRUSTED_KEY, (self.out / "repository" / (SIGNING_KEY + ".pub")).read_text())
        repo = self.root / "repo" / self.args.arch
        if repo.exists():
            shutil.rmtree(repo)
        shutil.copytree(self.out / "repository" / self.args.arch, repo)
        self.chroot("apk", "add", "--repository", "/repo", "uflinux", "uflinux-live", "uflinux-installer", "uflinux-installer-ui")
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
        settings = self.root / "etc/xdg/labwc/rc.xml"
        before = settings.read_text()
        settings.write_text(before + "\n<!-- UFLinux upgrade preservation check -->\n")
        self.chroot("apk", "fix", "--reinstall", "--repository", "/repo", "uflinux-defaults")
        if "UFLinux upgrade preservation check" not in settings.read_text():
            raise RuntimeError("apk replaced modified system defaults")
        settings.write_text(before)
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
        self.chroot("python3", "-m", "unittest", "discover", "-s", "/project/tests", "-p", "test_installer*.py")
        self.chroot("python3", "/project/tests/power-acpid-smoke.py")
        self.chroot("scanelf", "--needed", "/usr/bin/uflinux-desktop", stdout=(report / "elf.txt").open("w"))
        self.chroot("ldd", "/usr/bin/uflinux-desktop", stdout=(report / "libraries.txt").open("w"))
        binary = self.root / "usr/bin/uflinux-desktop"
        header = binary.read_bytes()[:64]
        if header[:4] != b"\x7fELF":
            raise RuntimeError("Desktop is not an ELF executable")
        byte_order = "<" if header[5] == 1 else ">"
        elf_type, machine = struct.unpack_from(byte_order + "HH", header, 16)
        expected = {"x86": 3, "x86_64": 62, "armv7": 40, "aarch64": 183,
                    "ppc64le": 21, "s390x": 22, "riscv64": 243}[self.args.arch]
        if machine != expected or elf_type != 3:
            raise RuntimeError("Desktop must be a position-independent executable for the target architecture")
        settings_binaries = {}
        for relative in ("usr/bin/uflinux-installer", "usr/bin/uflinux-settings", "usr/bin/uflinux-settingsctl",
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
            "desktop": "headless smoke passed", "desktop_bytes": binary.stat().st_size, "elf_machine": machine,
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
            # The system settings service owns a bus name that a privileged
            # operation activates on demand. Under qemu-user that activation
            # cannot elevate through the target setuid helper, so run it as a
            # managed resident process here, exactly like the D-Bus fixtures,
            # instead of letting the bus daemon start it.
            for name, command in (("system-bus", ["dbus-daemon", "--system", "--nofork", "--nopidfile"]),
                                  ("connman-fixture", ["python3", "/project/tests/mock-connman.py"]),
                                  ("login1-fixture", ["python3", "/project/tests/mock-login1.py"]),
                                  ("settings-system", ["/usr/libexec/uflinux-settings-system", "--resident"])):
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
        # Several devices share an architecture, so their records must not overwrite
        # each other when the release job merges the per-device artifacts.
        name = stage
        if stage == "device" and getattr(self.args, "device", None):
            name = stage + "-" + self.args.device
        with (directory / (name + "-packages.txt")).open("w") as record:
            self.chroot("apk", "info", "-v", stdout=record)
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
        if stage == "device":
            metadata.update(getattr(self, "device_info", {}))
            metadata["image_format"] = "raw-gpt"
        (directory / (name + ".json")).write_text(json.dumps(metadata, indent=2, sort_keys=True) + "\n")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--arch", choices=ARCHES, required=True)
    parser.add_argument("--version", default=LOCK["distribution"]["version"])
    parser.add_argument("--outdir", default=str(PROJECT / "out"))
    parser.add_argument("--package", choices=PACKAGE_NAMES, help="Compile one independent package origin (packages/bootstrap stages only)")
    parser.add_argument("--device", choices=sorted(DEVICES), help="Build a device disk image instead of an ISO (--stage device, or --stage all)")
    parser.add_argument("--workdir")
    parser.add_argument("--cache")
    parser.add_argument("--repository-url", default=os.environ.get("APK_REPOSITORY_URL"))
    parser.add_argument("--stage", choices=["bootstrap", "packages", "sign", "image", "device", "verify", "all"], default="all")
    parser.add_argument("--release", action="store_true")
    args = parser.parse_args(argv)
    if args.package and (args.stage not in {"bootstrap", "packages"} or args.release):
        parser.error("--package requires --stage packages or bootstrap, without --release")
    if args.stage == "device" and args.device is None:
        parser.error("--stage device requires --device")
    if args.device is not None and args.stage not in {"device", "all"}:
        parser.error("--device is used by --stage device and by --stage all only")
    if args.device:
        try:
            device_target(DEVICES, args.device, args.arch)
        except ValueError as exc:
            parser.error(str(exc))
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
            stages = ["packages", "sign", "image", "verify"]
            if args.device is not None:
                # A device build replaces the ISO: verify_installed() exercises the
                # live media and the installer, neither of which is on a board image.
                stages = ["packages", "sign", "device"]
            for stage in (stages if args.stage == "all" else [args.stage]):
                if stage != "bootstrap":
                    try:
                        getattr(builder, STAGE_METHODS.get(stage, stage))()
                    finally:
                        if args.release and stage in {"sign", "image", "device"}:
                            (builder.root / "var/lib/uflinux/keys" / SIGNING_KEY).unlink(missing_ok=True)
    except (OSError, RuntimeError, ValueError, subprocess.CalledProcessError) as exc:
        print(f"uflinux-build: {exc}", file=sys.stderr)
        return 1
    finally:
        if args.release:
            # Verification executes the packaged binary after the release key is gone.
            (builder.root / "var/lib/uflinux/keys" / SIGNING_KEY).unlink(missing_ok=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
