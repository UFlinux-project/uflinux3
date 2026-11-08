"""Regress retained QMP closures preventing a second power-button connection."""
import json
from pathlib import Path
import socket
import sys
import tempfile
import threading
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
from vm_control import qmp, memory_mib, check_boot_errors


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


class BootTests(unittest.TestCase):
    def test_live_budget_does_not_change_installed_acceptance_budget(self):
        for arch in ("x86", "x86_64", "armv7", "aarch64", "s390x", "ppc64le", "riscv64", "loongarch64"):
            self.assertEqual(memory_mib(arch), 3072 if arch == "armv7" else 4096)
            self.assertEqual(memory_mib(arch, installed=True), 512 if arch == "x86_64" else 2048)

    def test_partial_apk_installation_fails_even_if_login_succeeds(self):
        console = "100%\r\nERROR: System state may be inconsistent: failed to write database: No space left on device\r\nuflinux login: "
        with self.assertRaisesRegex(RuntimeError, "Guest boot failed.*No space left on device"):
            check_boot_errors(console)

    def test_fatal_kernel_errors_fail_and_normal_boot_warnings_do_not(self):
        for message in ("Out of memory: Killed process 42 (apk)", "Kernel panic - not syncing"):
            with self.assertRaises(RuntimeError):
                check_boot_errors(message)
        check_boot_errors("WARNING: opening from cache https://example.org/APKINDEX.tar.gz: No such file or directory\nWelcome to Alpine Linux")
