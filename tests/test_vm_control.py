"""Regress retained QMP closures preventing a second power-button connection."""
import json
from pathlib import Path
import socket
import sys
import tempfile
import threading
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))
from vm_control import qmp, memory_mib, check_boot_errors, expect_login_prompt, greeter_button, press_key


class GreeterTests(unittest.TestCase):
    def frame(self, password=False, button=True):
        width, height = 300, 230
        pixels = bytearray(bytes((33, 33, 36)) * width * height)
        def blue(x, y):
            offset = (y * width + x) * 3
            pixels[offset:offset + 3] = bytes((53, 132, 228))
        if button:
            for y in range(170 if password else 140, 200 if password else 170):
                for x in range(150, 230):
                    blue(x, y)
        if password:
            for y in range(80, 158):
                for x in range(30, 270):
                    if y < 82 or y >= 156 or x < 32 or x >= 268:
                        blue(x, y)
        return f'P6\n{width} {height}\n255\n'.encode() + pixels

    def test_password_focus_outline_does_not_hide_prompt_transition(self):
        initial = greeter_button(self.frame())
        password = greeter_button(self.frame(password=True))
        self.assertGreaterEqual(password[1], initial[1] + 10)
        self.assertIn(password[0], range(150, 230))
        self.assertIn(password[1], range(170, 200))

    def test_unrendered_frame_is_not_ready(self):
        self.assertIsNone(greeter_button(self.frame(button=False)))

    def test_truncated_frame_is_rejected(self):
        with self.assertRaisesRegex(RuntimeError, 'screenshot format'):
            greeter_button(self.frame()[:-1])


class ControlTests(unittest.TestCase):
    def test_repeated_password_letters_have_separate_key_presses(self):
        held = set()
        typed = []
        def keyboard(command, arguments):
            self.assertEqual(command, 'input-send-event')
            for event in arguments['events']:
                key = event['data']['key']['data']
                if event['data']['down']:
                    if key not in held:
                        typed.append(key)
                    held.add(key)
                else:
                    held.discard(key)
        with patch('vm_control.time.sleep'):
            press_key(keyboard, 's')
            press_key(keyboard, 's')
        self.assertEqual(typed, ['s', 's'])
        self.assertFalse(held)

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
    def test_firmware_failure_does_not_wait_for_login_timeout(self):
        class Console:
            def __init__(self, result): self.result = result
            def expect(self, patterns):
                self.patterns = patterns
                return self.result
        console = Console(1)
        with self.assertRaisesRegex(RuntimeError, "could not find a bootable device"):
            expect_login_prompt(console)
        self.assertTrue(any("No bootable option" in pattern for pattern in console.patterns))
        with self.assertRaisesRegex(RuntimeError, "out of resources loading the initramfs"):
            expect_login_prompt(Console(2))
        expect_login_prompt(Console(0))

    def test_live_budget_does_not_change_installed_acceptance_budget(self):
        for arch in ("x86", "x86_64", "armv7", "aarch64", "s390x", "ppc64le", "riscv64"):
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
