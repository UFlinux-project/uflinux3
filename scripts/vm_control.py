"""Bounded QEMU control connections, independent of guest console tooling."""
import contextlib
import json
import socket


@contextlib.contextmanager
def qmp(state):
    with socket.socket(socket.AF_UNIX) as connection:
        connection.settimeout(10)
        connection.connect(str(state / "qmp.sock"))
        stream = connection.makefile("rwb", buffering=0)
        def request(execute, arguments=None):
            stream.write((json.dumps({"execute": execute, "arguments": arguments or {}}) + "\n").encode())
            while True:
                reply = json.loads(stream.readline())
                if "error" in reply:
                    raise RuntimeError("QMP failed: " + str(reply["error"]))
                if "return" in reply:
                    return reply["return"]
        try:
            json.loads(stream.readline())
            request("qmp_capabilities")
            yield request
        finally:
            # makefile owns another reference to the socket. The yielded closure
            # can outlive this context; close the stream explicitly so QEMU can
            # accept the next control connection.
            stream.close()


def expect_login_prompt(child):
    """Stop immediately when firmware gives up, rather than timing out on login."""
    matched = child.expect([r"login: ", r"No bootable option or device was found\.",
                            r"EFI stub: ERROR: Failed to load initrd: 0x8000000000000009"])
    if matched == 1:
        raise RuntimeError("UEFI firmware could not find a bootable device; inspect the VM console log")
    if matched == 2:
        raise RuntimeError("UEFI ran out of resources loading the initramfs; inspect its size and VM memory")


def memory_mib(arch, installed=False):
    """Diskless live roots need RAM for unpacked APKs as well as the desktop."""
    if not installed:
        # The armv7 virt machine disables highmem and has a 3 GiB RAM limit.
        return 3072 if arch == "armv7" else 4096
    return 512 if arch == "x86_64" else 2048


def check_boot_errors(console):
    fatal = ("failed to write database:", "ERROR: System state may be inconsistent:",
             "Out of memory:", "Kernel panic -")
    errors = []
    for line in console.splitlines():
        positions = [line.index(message) for message in fatal if message in line]
        if positions:
            errors.append(line[min(positions):].strip())
    if errors:
        raise RuntimeError("Guest boot failed before validation: " + "\n".join(errors))
