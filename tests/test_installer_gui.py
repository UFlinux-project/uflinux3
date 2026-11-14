import importlib.machinery
import importlib.util
import builtins
import json
import io
import os
import stat
import tempfile
from types import SimpleNamespace
from pathlib import Path
import unittest
from unittest.mock import patch

PATH = Path(__file__).resolve().parents[1] / 'aports/uflinux/uflinux-install-gui-backend'
loader = importlib.machinery.SourceFileLoader('gui_backend', str(PATH))
spec = importlib.util.spec_from_loader(loader.name, loader)
backend = importlib.util.module_from_spec(spec)
loader.exec_module(backend)


class InstallerGuiTests(unittest.TestCase):
    def setUp(self):
        self.disk = {'path': '/dev/vda', 'type': 'disk', 'size': 20 * 1024**3, 'ro': False,
                     'mountpoints': [None], 'diskseq': 10, 'children': [
                         {'path': '/dev/vda1', 'type': 'part', 'fstype': 'vfat', 'size': 1024**3, 'mountpoints': [None]},
                         {'path': '/dev/vda2', 'type': 'part', 'fstype': 'xfs', 'size': 10 * 1024**3, 'mountpoints': [None]}]}
        self.state = {'arch': 'x86_64', 'mode': 'uefi', 'devices': [self.disk]}
        self.request = {'version': 1, 'storage': {'mode': 'erase', 'disk': '/dev/vda',
                        'fingerprint': backend.fingerprint(self.disk), 'root': '', 'boot': ''},
                        'account': {'name': 'Alice', 'user': 'alice', 'hostname': 'uflinux', 'password': 'secret123'},
                        'locale': 'en_US.UTF-8', 'keyboard': 'us', 'timezone': 'Europe/Prague', 'online': False}

    def test_discovery_requests_the_device_tree(self):
        output = json.dumps({'blockdevices': [self.disk]})
        with patch.object(backend.cli, 'output', side_effect=[output, 'x86_64']) as call, \
             patch.object(backend, 'safe_disk', return_value=True), patch.object(backend, 'disk_sequence', return_value=10):
            result = backend.discover()
        self.assertIn('--tree', call.call_args_list[0].args)
        self.assertEqual(result['disks'][0]['device']['children'][0]['path'], '/dev/vda1')

    def test_layouts_cover_supported_firmware(self):
        for arch in ('x86', 'x86_64', 'aarch64', 'armv7', 'riscv64'):
            with self.subTest(arch=arch):
                layout, boot, root = backend.storage_plan(arch, 'uefi')
                self.assertEqual(layout[0], 'size=1G,type=U')
                self.assertEqual((boot, root), (1, 2))
        self.assertIn('LegacyBIOSBootable', backend.storage_plan('x86_64', 'bios')[0][0])
        self.assertEqual(backend.storage_plan('ppc64le', 'native')[1:], (2, 3))
        with self.assertRaises(backend.cli.InstallError):
            backend.storage_plan('armv7', 'native')
        with self.assertRaises(backend.cli.InstallError):
            backend.storage_plan('mips64', 'native')

    def test_mount_on_child_excludes_the_whole_disk(self):
        self.disk['maj:min'] = '8:0'
        self.disk['children'][0]['maj:min'] = '8:1'
        self.disk['children'][0]['mountpoints'] = ['/media/live']
        fake_stat = SimpleNamespace(st_mode=stat.S_IFBLK, st_rdev=os.makedev(8, 0))
        with patch.object(Path, 'exists', return_value=True), patch.object(Path, 'is_dir', return_value=True), \
             patch.object(Path, 'iterdir', side_effect=lambda: iter(())), patch.object(backend.os, 'stat', return_value=fake_stat):
            self.assertFalse(backend.safe_disk(self.disk))

    def test_boot_symlink_and_nonempty_root_are_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            target = Path(tmp)
            (target / 'boot').symlink_to('/tmp')
            with self.assertRaisesRegex(backend.cli.InstallError, 'normal directory'):
                backend.prepare_boot_directory(target)
            (target / 'boot').unlink()
            (target / 'existing').write_text('keep')
            with self.assertRaisesRegex(backend.cli.InstallError, 'empty'):
                backend.prepare_boot_directory(target)
            self.assertFalse((target / 'boot').exists())

    def test_disk_sequence_changes_fingerprint(self):
        before = backend.fingerprint(self.disk)
        self.disk['diskseq'] += 1
        self.assertNotEqual(before, backend.fingerprint(self.disk))

    def test_stale_disk_is_rejected_before_storage_commands(self):
        self.request['storage']['fingerprint'] = 'outdated'
        with patch.object(backend.cli, 'run') as run:
            with self.assertRaisesRegex(backend.cli.InstallError, 'changed'):
                backend.validate_request(self.request, self.state)
            run.assert_not_called()

    def test_mounted_and_readonly_disks_rejected(self):
        for change in ({'mountpoints': ['/media/usb']}, {'ro': True}, {'type': 'lvm'}):
            with self.subTest(change=change):
                disk = dict(self.disk, **change)
                self.assertFalse(backend.safe_disk(disk))

    def test_prepared_requires_correct_partition_types(self):
        self.request['storage'].update(mode='prepared', boot='/dev/vda1', root='/dev/vda2')
        with patch.object(backend, 'safe_disk', return_value=True):
            _, _, boot, root = backend.validate_request(self.request, self.state)
            self.assertEqual((boot, root), ('/dev/vda1', '/dev/vda2'))
            self.request['storage']['root'] = '/dev/vda1'
            with self.assertRaises(backend.cli.InstallError):
                backend.validate_request(self.request, self.state)

    def test_invalid_account_rejected(self):
        for key, value in [('user', 'root'), ('hostname', 'bad host'), ('password', 'short'), ('name', 'bad:name')]:
            with self.subTest(key=key):
                original = self.request['account'][key]
                self.request['account'][key] = value
                with self.assertRaises(backend.cli.InstallError):
                    backend.validate_request(self.request, self.state)
                self.request['account'][key] = original

    def test_rejects_too_small_disk(self):
        self.disk['size'] = 9 * 1024**3
        self.request['storage']['fingerprint'] = backend.fingerprint(self.disk)
        with patch.object(backend, 'safe_disk', return_value=True):
            with self.assertRaisesRegex(backend.cli.InstallError, 'too small'):
                backend.validate_request(self.request, self.state)

    def test_prepared_failure_unmounts_without_formatting(self):
        self.request['storage'].update(mode='prepared', boot='/dev/vda1', root='/dev/vda2')
        real_exists, real_is_dir = Path.exists, Path.is_dir
        real_is_file, real_read = Path.is_file, Path.read_text
        real_open, real_tmp = builtins.open, tempfile.TemporaryDirectory
        def exists(path):
            return True if str(path) == '/run/uflinux-live' else real_exists(path)
        def is_dir(path):
            return True if str(path) == '/sys/firmware/efi/efivars' else real_is_dir(path)
        def is_file(path):
            return True if str(path) == '/media/repo/.boot_repository' else real_is_file(path)
        def read(path, *args, **kwargs):
            return '/media/repo\n' if str(path) == '/etc/apk/repositories' else real_read(path, *args, **kwargs)
        with real_tmp() as temporary:
            def open_file(path, *args, **kwargs):
                return real_open(Path(temporary) / 'lock' if path == '/run/uflinux-install.lock' else path, *args, **kwargs)
            def tempdir(*args, **kwargs):
                kwargs['dir'] = temporary
                return real_tmp(*args, **kwargs)
            selected = (self.disk, [], '/dev/vda1', '/dev/vda2')
            with patch.object(backend.os, 'geteuid', return_value=0), patch.object(Path, 'exists', exists), \
                 patch.object(Path, 'is_dir', is_dir), patch.object(Path, 'is_file', is_file), \
                 patch.object(Path, 'read_text', read), patch('builtins.open', open_file), \
                 patch.object(backend.tempfile, 'TemporaryDirectory', tempdir), \
                 patch.object(backend, 'discover', return_value=self.state), \
                 patch.object(backend, 'validate_request', return_value=selected), \
                 patch.object(backend.cli, 'output', return_value='FAT32'), \
                 patch.object(backend.cli, 'efi_partition'), patch.object(backend.cli, 'preflight', return_value={}), \
                 patch.object(backend.cli, 'install', side_effect=backend.cli.InstallError('failure')), \
                 patch.object(backend.cli, 'run'), patch.object(backend, 'run_with_events') as run, \
                 patch.object(backend, 'event'):
                with self.assertRaisesRegex(backend.cli.InstallError, 'failure'):
                    backend.execute(self.request)
            commands = [call.args[0] for call in run.call_args_list]
            self.assertEqual(commands, ['mount', 'mount', 'umount', 'umount'])

    def test_log_redacts_password(self):
        backend.secret = 'secret123'
        result = unittest.mock.Mock(stdout=io.StringIO('password secret123\n'))
        result.wait.return_value = 0
        with patch.object(backend.subprocess, 'Popen', return_value=result), patch.object(backend, 'event') as event:
            backend.run_with_events('example')
            event.assert_called_once_with('log', 'password [redacted]')


if __name__ == '__main__':
    unittest.main()
