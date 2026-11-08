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
