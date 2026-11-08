"""Regress retained QMP closures preventing a second power-button connection."""
import json
from pathlib import Path
import socket
import sys
import tempfile
import threading
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
from vm_control import qmp


class ControlTests(unittest.TestCase):
    def test_context_releases_connection_while_callback_remains_referenced(self):
        with tempfile.TemporaryDirectory() as directory:
            state = Path(directory)
            closed = threading.Event()
            failures = []
            with socket.socket(socket.AF_UNIX) as server:
                server.bind(str(state / "qmp.sock"))
                server.listen(2)
                server.settimeout(2)

                def serve():
                    try:
                        for _ in range(2):
                            connection, _ = server.accept()
                            with connection, connection.makefile("rwb", buffering=0) as stream:
                                connection.settimeout(2)
                                stream.write(b'{"QMP": {}}\n')
                                while raw := stream.readline():
                                    request = json.loads(raw)
                                    stream.write(json.dumps({"return": request["execute"]}).encode() + b"\n")
                            closed.set()
                    except Exception as error:
                        failures.append(error)
                        closed.set()

                worker = threading.Thread(target=serve, daemon=True)
                worker.start()
                with qmp(state) as first:
                    self.assertEqual(first("query-status"), "query-status")
                self.assertTrue(closed.wait(1), "The retained callback still owns the QMP stream")
                self.assertIsNotNone(first)  # Match the VM runner's local-variable lifetime.
                with qmp(state) as second:
                    self.assertEqual(second("query-status"), "query-status")
                worker.join(3)
                self.assertFalse(worker.is_alive())
                self.assertEqual(failures, [])
