"""Bounded QEMU control connections, independent of guest console tooling."""
import contextlib
import json
import socket
import time


def press_key(request, key):
    """Release each key explicitly, including repeated password characters."""
    for down in (True, False):
        request("input-send-event", {"events": [{"type": "key", "data": {
            "down": down, "key": {"type": "qcode", "data": key}}}]})
        time.sleep(0.05)


def greeter_button(frame):
    """Locate ReGreet's blue Login control in a QEMU PPM frame."""
    magic, dimensions, maximum, pixels = frame.split(b"\n", 3)
    width, height = map(int, dimensions.split())
    if magic != b"P6" or maximum != b"255" or len(pixels) != width * height * 3:
        raise RuntimeError("Unexpected greeter screenshot format")
    coordinates = [(i % width, i // width) for i, (r, g, b) in enumerate(zip(pixels[::3], pixels[1::3], pixels[2::3]))
                   if r < 110 and g > 80 and b > 150 and b > g + 30]
    # The password entry also has a blue focus outline. Its pixels must not
    # pull the detected button upward when waiting for the password prompt.
    if len(coordinates) <= 1000:
        return None
    middle = len(coordinates) // 2
    x = sorted(point[0] for point in coordinates)[middle]
    return x, coordinates[middle][1], width, height


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


def memory_mib(installed=False):
    """Diskless live roots need RAM for unpacked APKs as well as the desktop."""
    return 512 if installed else 4096


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
