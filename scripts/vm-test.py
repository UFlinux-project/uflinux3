#!/usr/bin/env python3
"""Boot the actual ISO, install into a disposable disk, and reboot from XFS."""
import argparse
import base64
import glob
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import time

import pexpect
from builder import sha, download
from vm_control import qmp, memory_mib, check_boot_errors, expect_login_prompt

ROOT = Path(__file__).resolve().parents[1]
ARCHES = json.loads((ROOT / "config/architectures.json").read_text())
FIRMWARE = {
    "x86_64": (["/usr/share/OVMF/OVMF_CODE_4M.fd", "/usr/share/OVMF/OVMF_CODE.fd"], ["/usr/share/OVMF/OVMF_VARS_4M.fd", "/usr/share/OVMF/OVMF_VARS.fd"]),
    "x86": (["/usr/share/OVMF/OVMF32_CODE_4M.fd", "/usr/share/OVMF/OVMF_CODE_4M.ia32.fd"], ["/usr/share/OVMF/OVMF32_VARS_4M.fd", "/usr/share/OVMF/OVMF_VARS_4M.ia32.fd"]),
    "aarch64": (["/usr/share/AAVMF/AAVMF_CODE.fd"], ["/usr/share/AAVMF/AAVMF_VARS.fd"]),
    "armv7": (["/usr/share/AAVMF/AAVMF32_CODE.fd"], ["/usr/share/AAVMF/AAVMF32_VARS.fd"]),
    "riscv64": (["/usr/share/qemu-efi-riscv64/RISCV_VIRT_CODE.fd"], ["/usr/share/qemu-efi-riscv64/RISCV_VIRT_VARS.fd"]),
}


def locate(patterns, description):
    for pattern in patterns:
        matches = sorted(glob.glob(str(ROOT / ".cache/firmware") + pattern)) or sorted(glob.glob(pattern))
        if matches:
            return matches[0]
    raise RuntimeError("Missing " + description + ": " + ", ".join(patterns))


def command(args, state, installed=False):
    arch = args.arch
    qemu_arch = {"x86": "i386", "armv7": "arm", "ppc64le": "ppc64"}.get(arch, arch)
    emulator = "qemu-system-" + qemu_arch
    if not shutil.which(emulator):
        raise RuntimeError("Missing " + emulator)
    machine = ARCHES[arch]["machine"]
    if arch == "armv7":
        machine += ",highmem=off"
    memory = str(memory_mib(arch, installed))
    cmd = [emulator, "-machine", machine, "-m", memory, "-smp", "2", "-nographic", "-no-reboot",
           "-qmp", f"unix:{state / 'qmp.sock'},server=on,wait=off"]
    if arch in {"aarch64", "armv7"}:
        cmd += ["-cpu", "max" if arch == "aarch64" else "cortex-a15"]
    if arch == "ppc64le":
        cmd += ["-cpu", "power9"]
    if arch in {"x86", "x86_64"}:
        cmd += ["-vga", "none", "-device", "virtio-vga"]
    elif arch == "aarch64":
        cmd += ["-device", "virtio-gpu-pci"]
    if args.mode == "uefi":
        code_patterns, vars_patterns = FIRMWARE[arch]
        code = locate(code_patterns, "UEFI firmware")
        variables = state / "vars.fd"
        if not variables.exists():
            shutil.copyfile(locate(vars_patterns, "UEFI variable template"), variables)
        cmd += ["-drive", f"if=pflash,format=raw,unit=0,readonly=on,file={code}",
                "-drive", f"if=pflash,format=raw,unit=1,file={variables}"]
    device_suffix = "ccw" if arch == "s390x" else "pci"
    cmd += ["-drive", f"if=none,id=target,format=raw,file={state / 'target.raw'}",
            "-device", f"virtio-blk-{device_suffix},drive=target,serial=uflinux-test,bootindex={'1' if installed else '2'}",
            "-netdev", "user,id=net", "-device", f"virtio-net-{device_suffix},netdev=net"]
    if not installed:
        cmd += ["-device", f"virtio-scsi-{device_suffix},id=scsi",
                "-drive", f"if=none,id=cd,format=raw,readonly=on,file={Path(args.iso).resolve()}",
                "-device", "scsi-cd,drive=cd,bus=scsi.0,bootindex=1"]
    return cmd


def guest(child, script, admin=True):
    prefix = "doas sh -ec " if admin else "sh -ec "
    child.sendline(prefix + shlex.quote(script) + "; printf '\\nUFLINUX_RC:%s\\n' $?")
    child.expect(r"\r*\nUFLINUX_RC:([0-9]+)\r*\n")
    result = child.before
    if child.match.group(1) != "0":
        raise RuntimeError("Guest command failed: " + script + "\n" + result)
    return result


def login(child, user, password):
    expect_login_prompt(child)
    child.sendline(user)
    child.expect("Password:")
    child.sendline(password)
    child.expect(r"[$#] ")


def transfer(child, source, destination):
    """Keep serial lines below the guest TTY's canonical input limit."""
    encoded = base64.b64encode(Path(source).read_bytes()).decode()
    temporary = destination + ".b64"
    guest(child, ": > " + shlex.quote(temporary))
    for offset in range(0, len(encoded), 1000):
        guest(child, "printf '%s' " + shlex.quote(encoded[offset:offset+1000]) + " >> " + shlex.quote(temporary))
    guest(child, "base64 -d " + shlex.quote(temporary) + " > " + shlex.quote(destination) + "; rm " + shlex.quote(temporary))


def graphical_login(state):
    """Type into the installed greeter; leave installed autologin disabled."""
    with qmp(state) as request:
        for text in ("\n", "uflinux-test-password\n"):
            for character in text:
                key = {"\n": "ret", "-": "minus"}.get(character, character)
                request("send-key", {"keys": [{"type": "qcode", "data": key}], "hold-time": 30})
                time.sleep(0.07)
            time.sleep(2)


def start(args, state, log, installed=False):
    cmd = command(args, state, installed)
    log.write(shlex.join(cmd) + "\n")
    child = pexpect.spawn(cmd[0], cmd[1:], encoding="utf-8", codec_errors="replace", timeout=args.timeout)
    child.logfile = log
    return child


def splash_smoke(child, state, out):
    # Verify real boot handoff, then reopen only inside this disposable guest to
    # capture two frames without inserting any artificial production boot delay.
    guest(child, "test -f /run/uflinux-splash/completed; ! pgrep -x plymouthd; rc-service greetd stop")
    try:
        guest(child, "/usr/libexec/uflinux-bootsplash start; test -f /run/uflinux-splash/started; plymouth --ping")
        frames = []
        with qmp(state) as request:
            for number in (1, 2):
                time.sleep(0.4)
                path = out / f"bootsplash-{number}.ppm"
                request("screendump", {"filename": str(path)})
                frames.append(path.read_bytes())
        if frames[0] == frames[1]:
            raise RuntimeError("Boot spinner did not animate between captured frames")
        magic, dimensions, maximum, pixels = frames[0].split(b"\n", 3)
        if magic != b"P6" or maximum != b"255":
            raise RuntimeError("Unexpected QEMU splash screenshot format")
        blue = sum(b > 120 and g > 50 and r < 80 for r, g, b in zip(pixels[::3], pixels[1::3], pixels[2::3]))
        white = sum(min(r, g, b) > 200 for r, g, b in zip(pixels[::3], pixels[1::3], pixels[2::3]))
        if blue < 20 or white < 300:
            raise RuntimeError("Splash screenshot lacks the blue spinner or original white leaf")
    finally:
        guest(child, "/usr/libexec/uflinux-bootsplash stop; ! pgrep -x plymouthd; rc-service greetd start")
    return {"animated_frames": True, "original_logo_visible": True, "handoff_without_daemon": True,
            "dimensions": dimensions.decode(), "blue_pixels": blue, "white_pixels": white}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--arch", choices=ARCHES, required=True)
    parser.add_argument("--iso", required=True)
    parser.add_argument("--mode", choices=["bios", "native", "uefi"], required=True)
    parser.add_argument("--outdir", default="out/vm")
    parser.add_argument("--timeout", type=int, default=1800)
    parser.add_argument("--network-tests", action="store_true", help="Run real hwsim Wi-Fi tests in the disposable x86_64 VM")
    args = parser.parse_args()
    if args.arch in {"x86", "x86_64"} and args.mode == "native":
        args.mode = "bios"  # Backward compatibility for existing invocations.
    if args.mode == "bios" and args.arch not in {"x86", "x86_64"}:
        parser.error("BIOS mode is only available for x86 and x86_64")
    source_iso=Path(args.iso).resolve()
    firmware_lock = json.loads((ROOT / "config/firmware.lock.json").read_text())
    if args.mode == "uefi" and args.arch in {"x86", "armv7"}:
        archive = download(firmware_lock[args.arch], ROOT / ".cache/firmware/downloads")
        subprocess.run(["dpkg-deb", "--extract", str(archive), str(ROOT / ".cache/firmware")], check=True)
    out = Path(args.outdir).resolve() / args.arch / args.mode
    out.mkdir(parents=True, exist_ok=True)
    (out / "verified.json").unlink(missing_ok=True)
    began = time.monotonic()
    child = None
    settings_metrics = None
    network_clock = None
    power_report = None
    splash_report = None
    try:
        with tempfile.TemporaryDirectory(prefix="uflinux-vm-") as temporary, (out / "console.log").open("w") as log:
            state = Path(temporary)
            snapshot=state/'boot.iso'
            try: os.link(source_iso,snapshot)
            except OSError: shutil.copyfile(source_iso,snapshot)
            iso_digest=sha(snapshot);iso_bytes=snapshot.stat().st_size
            args.iso=str(snapshot)
            with (state / "target.raw").open("wb") as disk:
                disk.truncate(8 * 1024**3)
            child = start(args, state, log)
            login(child, "live", "live")
            boot_seconds = time.monotonic() - began
            log.flush()
            check_boot_errors(Path(log.name).read_text())
            guest(child, "df -h /; df -i /")
            guest(child, "test -e /run/uflinux-live; test $(id -un) = root; ! rc-service sshd status")
            guest(child, "apk info -e uflinux ironbar labwc")
            guest(child, '. /etc/rc.conf.d/uflinux.conf; test "$rc_parallel" = YES; apk info -e linux-firmware')
            guest(child, "ironbar -c /etc/xdg/ironbar/config.toml -t /etc/xdg/ironbar/style.css --validate-config")
            guest(child, "apk info -e connman iwd busybox-openrc uflinux-settings uflinux-settings-daemon uflinux-settings-system; connmanctl state; connmanctl services; rc-service ntpd status; ! pgrep -x NetworkManager; ! pgrep -x chronyd; ! pgrep -x openntpd; ! pgrep -x wpa_supplicant")
            if args.arch in {"x86", "x86_64", "aarch64"}:
                guest(child, "for n in $(seq 1 30); do pgrep -x labwc >/dev/null && pgrep -x ironbar >/dev/null && break; sleep 1; done; if ! pgrep -x labwc || ! pgrep -x ironbar; then rc-status; cat /var/log/messages; exit 1; fi")
            if args.arch == "x86_64":
                splash_report = splash_smoke(child, state, out)
            if args.network_tests:
                if args.arch != "x86_64":
                    raise RuntimeError("The real Wi-Fi fixture is configured for x86_64 QEMU")
                guest(child, "touch /run/uflinux-test-vm; apk add hostapd dnsmasq iw py3-gobject3")
                transfer(child, ROOT / "tests/settings-network-vm.py", "/run/uflinux-settings-network.py")
                radio = guest(child, "python3 /run/uflinux-settings-network.py")
                transfer(child, ROOT / "tests/settings-clock-vm.py", "/run/uflinux-settings-clock.py")
                clock = guest(child, "python3 /run/uflinux-settings-clock.py")
                network_clock = {"network": json.loads(re.search(r"UFLINUX_NETWORK_VM:(\{[^\r\n]*\})", radio).group(1)),
                                 "clock": json.loads(re.search(r"UFLINUX_CLOCK_VM:(\{[^\r\n]*\})", clock).group(1))}
                (out / "network-clock.json").write_text(json.dumps(network_clock, indent=2) + "\n")
            if args.arch == "ppc64le" or args.mode == "uefi":
                special = "size=512M,type=U" if args.mode == "uefi" else (
                    "size=8M,type=9e1a2d38-c612-4316-aa26-8b49521e5a8b" if args.arch == "ppc64le" else "size=2M,type=21686148-6449-6e6f-744e-656564454649")
                layout = [special, "size=1G,type=L", "size=4G,type=L", "size=64M,type=L"]
                root_part, boot_part, keep_part = 3, 2, 4
            else:
                boot_layout = "size=1G,type=L,attrs=LegacyBIOSBootable" if args.arch in {"x86", "x86_64"} else "size=1G,type=L"
                layout = [boot_layout, "size=4G,type=L", "size=64M,type=L"]
                root_part, boot_part, keep_part = 2, 1, 3
            guest(child, "test -b /dev/vda; printf '%s\\n' " + " ".join(shlex.quote(x) for x in ["label: gpt", *layout]) + " | sfdisk /dev/vda")
            if args.mode == "uefi":
                guest(child, f"mkfs.xfs -f /dev/vda{root_part}; mkfs.vfat -F32 /dev/vda1; mount /dev/vda{root_part} /mnt; mkdir /mnt/boot; mount /dev/vda1 /mnt/boot")
            else:
                guest(child, f"mkfs.xfs -f /dev/vda{root_part}; mkfs.ext4 -q /dev/vda{boot_part}; mount /dev/vda{root_part} /mnt; mkdir /mnt/boot; mount /dev/vda{boot_part} /mnt/boot")
            guest(child, f"printf UFLINUX_PRESERVE_PARTITION | dd of=/dev/vda{keep_part} conv=notrunc status=none")
            hash_output = guest(child, f"sha256sum /dev/vda{keep_part} | tee /tmp/preserved.sha256; sfdisk --dump /dev/vda > /tmp/table.before")
            preserved_hash = re.search(r"([0-9a-f]{64})\s+/dev/vda" + str(keep_part), hash_output).group(1)
            guest(child, f"! uflinux-install --target /mnt --user live --dry-run; uflinux-install --target /mnt --user alice --boot-mode {args.mode} --boot-disk /dev/vda --dry-run")
            child.sendline(f"doas uflinux-install --target /mnt --user alice --boot-mode {args.mode} --boot-disk /dev/vda; printf '\\nUFLINUX_RC:%s\\n' $?")
            child.expect(r"Type INSTALL ([^ ]+) to install:")
            child.sendline("INSTALL " + child.match.group(1))
            child.expect("Password for alice:")
            child.sendline("uflinux-test-password")
            child.expect("Repeat password:")
            child.sendline("uflinux-test-password")
            child.expect(r"\r*\nUFLINUX_RC:0\r*\n")
            guest(child, "test $(findmnt -n -o FSTYPE /mnt) = xfs; ! grep -q '^live:' /mnt/etc/passwd; ! grep -q initial_session /mnt/etc/greetd/config.toml; sha256sum -c /tmp/preserved.sha256; sfdisk --dump /dev/vda > /tmp/table.after; cmp /tmp/table.before /tmp/table.after")
            if args.mode == "uefi":
                guest(child, "test -s /mnt/boot/EFI/BOOT/BOOT*.EFI; test -s /mnt/boot/loader/entries/uflinux.conf; ls -lh /mnt/boot/initramfs-lts; efibootmgr -v")
            if args.arch == "x86_64":
                # Install only the measurement harness interpreter from the signed
                # media. Opening the complete online index can pull kernel firmware
                # recommendations unrelated to this controlled memory test.
                guest(child, 'repo=; for marker in /media/*/apks/.boot_repository; do [ ! -f "$marker" ] || repo=${marker%/.boot_repository}; done; test -n "$repo"; apk --root /mnt --no-network --repositories-file /dev/null --repository "$repo" add python3')
            guest(child, "sync; umount -R /mnt; poweroff")
            child.expect(pexpect.EOF)
            child.close()
            child = start(args, state, log, installed=True)
            login(child, "alice", "uflinux-test-password")
            guest(child, "awk '$2 == \"/\" && $3 == \"xfs\" {ok=1} END {exit !ok}' /proc/mounts; test ! -e /run/uflinux-live; ! id live; apk info -e uflinux ironbar labwc linux-firmware", admin=False)
            # Authenticate doas normally before invoking the root-only test harness.
            guest(child, '. /etc/rc.conf.d/uflinux.conf; test "$rc_parallel" = YES', admin=False)
            child.sendline("doas true")
            child.expect(r"[Pp]assword:")
            child.sendline("uflinux-test-password")
            child.expect(r"[$#] ")
            if args.arch == "x86_64":
                guest(child, "test -f /run/uflinux-splash/completed; ! pgrep -x plymouthd")
                splash_report["installed_handoff_without_daemon"] = True
                graphical_login(state)
                transfer(child, ROOT / "tests/settings-installed.py", "/run/uflinux-settings-installed.py")
                guest(child, "python3 /run/uflinux-settings-installed.py")
                metrics = guest(child, "cat /run/uflinux-settings-installed.json")
                records = re.search(r"UFLINUX_INSTALLED_METRICS:(\{[^\r\n]*\})", metrics)
                if not records:
                    raise RuntimeError("Installed Settings metrics were not recorded")
                settings_metrics = json.loads(records.group(1))
                (out / "settings-512m.json").write_text(json.dumps(settings_metrics, indent=2) + "\n")
                # A real modifier-release opens the installed launcher on the
                # compositor-selected monitor; preserve a clean desktop screenshot.
                with qmp(state) as request:
                    request("send-key", {"keys":[{"type":"qcode","data":"meta_l"}],"hold-time":100})
                time.sleep(3)
                with qmp(state) as request:
                    request("screendump", {"filename":str(out / "start-menu.ppm")})
                    request("send-key", {"keys":[{"type":"qcode","data":"meta_l"}],"hold-time":100})
                time.sleep(1)
                # Launch from labwc so the agent belongs to the actual graphical session.
                with qmp(state) as request:
                    request("send-key", {"keys":[{"type":"qcode","data":"meta_l"},{"type":"qcode","data":"i"}]})
                time.sleep(2)
                transfer(child, ROOT / "tests/settings-auth-vm.py", "/run/uflinux-settings-auth.py")
                guest(child,"python3 /run/uflinux-settings-auth.py start-cancel")
                with qmp(state) as request:
                    request("send-key", {"keys":[{"type":"qcode","data":"alt"},{"type":"qcode","data":"f4"}]})
                guest(child,"python3 /run/uflinux-settings-auth.py check-cancel")
                guest(child,"python3 /run/uflinux-settings-auth.py start-grant")
                with qmp(state) as request:
                    for character in "uflinux-test-password\n":
                        key={"\n":"ret","-":"minus"}.get(character,character)
                        request("send-key",{"keys":[{"type":"qcode","data":key}],"hold-time":30})
                        time.sleep(.07)
                guest(child,"python3 /run/uflinux-settings-auth.py check-grant")
                transfer(child, ROOT / "tests/settings-power-vm.py", "/run/uflinux-settings-power.py")
                power = guest(child, "python3 /run/uflinux-settings-power.py")
                power_report = json.loads(re.search(r"UFLINUX_POWER_VM:(\{[^\r\n]*\})", power).group(1))
                with qmp(state) as request:
                    request("system_powerdown")
                time.sleep(3)
                guest(child, "test -e /run/uflinux-acpid.pid; python3 /run/uflinux-settings-power.py --restore-buttons")
                power_report["physical_key_do_nothing"] = True
                power_report["gtk_authorization_cancel_and_grant"] = True
            # Password-authenticated doas is exercised after installation.
            final_check = f"test $(id -un) = root; printf '%s\\n' '{preserved_hash}  /dev/vda{keep_part}' | sha256sum -c -"
            if args.arch != "x86_64":
                final_check += "; poweroff"
            if args.arch == "x86_64":
                guest(child,final_check)
                with qmp(state) as request:
                    request("system_powerdown")
            else:
                child.sendline("doas sh -ec " + shlex.quote(final_check))
            child.expect(pexpect.EOF)
            if power_report is not None:
                power_report["physical_key_poweroff"] = True
                (out / "power.json").write_text(json.dumps(power_report, indent=2) + "\n")
            child.close()
            child = None
            if sha(snapshot)!=iso_digest:
                raise RuntimeError("The ISO changed while the VM was being tested")
        (out / "verified.json").write_text(json.dumps({"arch": args.arch, "mode": args.mode,
            "boot_seconds": round(boot_seconds, 2), "iso_bytes": iso_bytes,
            "iso_sha256": iso_digest,
            "live_memory_mib": memory_mib(args.arch),
            "installed_memory_mib": memory_mib(args.arch, installed=True),
            "firmware_lock": firmware_lock if args.arch in {"x86", "armv7"} else None,
            "iso_boot": True, "xfs_install_reboot": True, "unrelated_partition_preserved": True,
            "bootsplash": splash_report,
            "settings_512m": settings_metrics, "network_clock": network_clock, "power": power_report}, indent=2) + "\n")
        return 0
    except (OSError, RuntimeError, subprocess.CalledProcessError, pexpect.ExceptionPexpect) as exc:
        print(f"VM validation failed: {exc}; see {out / 'console.log'}", file=sys.stderr)
        return 1
    finally:
        if child is not None and child.isalive():
            child.close(force=True)


if __name__ == "__main__":
    sys.exit(main())
