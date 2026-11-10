"""Fail closed when independent package jobs are missing, mixed or modified."""
import importlib.util
import json
import re
import subprocess
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import Mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'scripts'))
import builder
from test_build_inputs import stream

spec = importlib.util.spec_from_file_location('merge_packages', builder.PROJECT / 'scripts/merge-packages.py')
merge = importlib.util.module_from_spec(spec)
spec.loader.exec_module(merge)


class ShardTests(unittest.TestCase):
    def fixture(self, out):
        for origin, names in builder.PACKAGE_NAMES.items():
            root = out / 'package-shards/x86_64' / origin
            packages = root / 'packages/x86_64'
            packages.mkdir(parents=True)
            upstream = {'uflinux-regreet': builder.LOCK['uflinux-regreet']['version'], 'connman': builder.LOCK['connman']['version'],
                        'uflinux-settings': builder.LOCK['settings']['version'], 'uflinux': '0.1.0'}[origin]
            revision = '1' if origin == 'connman' else '0'
            version = upstream + '-r' + revision
            records, payloads = {}, {}
            for name in names:
                path = packages / (name + '-' + version + '.apk')
                info = f'pkgname = {name}\npkgver = {version}\norigin = {origin}\narch = x86_64\n'.encode()
                path.write_bytes(stream('.SIGN.RSA256.uflinux.rsa.pub', b'sig') + stream('.PKGINFO', info) + stream('usr/share/example', b'data'))
                records[str(path.relative_to(root))] = {'sha256': builder.sha(path)}
                payloads[path.name] = builder.unsigned_digest(path)
            (packages / 'build.rsa.pub').write_text('test key for ' + origin)
            metadata = root / 'metadata/x86_64'
            metadata.mkdir(parents=True)
            record = {'arch': 'x86_64', 'version': '0.1.0', 'package': origin, 'sources': builder.LOCK,
                      'source_commit': 'same-commit', 'source_date_epoch': 123, 'repository_url': 'https://example.org/apk', 'files': records}
            (metadata / 'compile.json').write_text(json.dumps(record))
            (metadata / 'reproducibility.json').write_text(json.dumps({'fresh_compiler_target': True, 'matched': True, 'baseline': payloads, 'rebuilt': payloads}))
            inputs = root / 'inputs/x86_64'
            inputs.mkdir(parents=True)
            for name in ('build-apks.tar.gz', 'sources.tar.gz', 'cargo-sources.tar.gz'):
                (inputs / name).write_bytes(b'fixture retained archive')

    def test_uflinux_recipe_packages_match_shard_allowlist(self):
        recipe = (builder.PROJECT / 'aports/uflinux/APKBUILD').read_text()
        subpackages = re.search(r'^subpackages="([^"]+)"', recipe, re.M).group(1)
        declared = {'uflinux'} | {entry.split(':', 1)[0] for entry in subpackages.split()}
        self.assertEqual(builder.PACKAGE_NAMES['uflinux'], declared)

    def test_complete_set_retains_independent_keys_inputs_and_proofs(self):
        with tempfile.TemporaryDirectory() as temporary:
            out = Path(temporary)
            self.fixture(out)
            merge.collect(out, 'x86_64', '0.1.0')
            packages = out / 'packages/x86_64'
            self.assertEqual(len(list(packages.glob('*.apk'))), sum(map(len, builder.PACKAGE_NAMES.values())))
            keys = json.loads((packages / 'build-keys.json').read_text())
            self.assertEqual(len(set(keys.values())), len(builder.PACKAGE_NAMES))
            proof = json.loads((out / 'metadata/x86_64/reproducibility.json').read_text())
            self.assertEqual(len(proof['baseline']), sum(map(len, builder.PACKAGE_NAMES.values())))
            self.assertEqual(proof['baseline'], proof['rebuilt'])
            self.assertTrue((out / 'inputs/x86_64/uflinux-settings/cargo-sources.tar.gz').exists())

    def test_invalid_inputs_never_replace_existing_signing_inputs(self):
        for mode in ('missing-shard', 'missing-apk', 'tampered-apk', 'mixed-commit', 'wrong-proof', 'missing-cache'):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as temporary:
                out = Path(temporary)
                self.fixture(out)
                shard = out / 'package-shards/x86_64/uflinux-settings'
                if mode == 'missing-shard':
                    shard.rename(shard.with_name('unexpected-origin'))
                elif mode == 'missing-apk':
                    next((shard / 'packages/x86_64').glob('*.apk')).unlink()
                elif mode == 'tampered-apk':
                    path = next((shard / 'packages/x86_64').glob('*.apk'))
                    path.write_bytes(path.read_bytes() + b'changed')
                elif mode == 'missing-cache':
                    (shard / 'inputs/x86_64/cargo-sources.tar.gz').unlink()
                else:
                    path = shard / 'metadata/x86_64' / ('compile.json' if mode == 'mixed-commit' else 'reproducibility.json')
                    record = json.loads(path.read_text())
                    record['source_commit' if mode == 'mixed-commit' else 'rebuilt'] = 'different'
                    path.write_text(json.dumps(record))
                destination = out / 'packages/x86_64'
                destination.mkdir(parents=True)
                (destination / 'sentinel').write_text('keep')
                with self.assertRaises((ValueError, EOFError)):
                    merge.collect(out, 'x86_64', '0.1.0')
                self.assertEqual((destination / 'sentinel').read_text(), 'keep')

    def test_package_outputs_are_isolated_and_serial_layout_is_unchanged(self):
        with tempfile.TemporaryDirectory() as temporary:
            self.assertEqual(builder.package_output(temporary, 'x86_64'), Path(temporary))
            self.assertNotEqual(builder.package_output(temporary, 'x86_64', 'connman'),
                                builder.package_output(temporary, 'x86_64', 'uflinux-settings'))


class InputArchiveTests(unittest.TestCase):
    def test_origin_archives_have_unique_release_asset_names(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            directory=root/'out/inputs/x86_64'
            for origin in builder.PACKAGE_NAMES:
                target=directory/origin
                target.mkdir(parents=True)
                for name in ('build-apks.tar.gz','sources.tar.gz'):
                    (target/name).write_bytes(b'input')
                if origin in {'uflinux-regreet','uflinux-settings'}:
                    (target/'cargo-sources.tar.gz').write_bytes(b'cargo')
            (directory/'image-apks.tar.gz').write_bytes(b'image')
            command=[sys.executable,str(builder.PROJECT/'scripts/release-inputs.py'),'--arch','x86_64','--version','0.1.0']
            subprocess.run(command,cwd=root,check=True,capture_output=True)
            entries=(root/'out/SHA256SUMS-x86_64-inputs').read_text().splitlines()
            self.assertEqual(len(entries),11)
            self.assertEqual(len(list((root/'out').glob('uflinux-*.tar.gz'))),11)
            (directory/'uflinux-settings/cargo-sources.tar.gz').unlink()
            rejected=subprocess.run(command,cwd=root,capture_output=True)
            self.assertNotEqual(rejected.returncode,0)


class EmulatedDependencyTests(unittest.TestCase):
    def test_dependencies_use_root_and_compilation_remains_unprivileged(self):
        instance = object.__new__(builder.Builder)
        instance.chroot = Mock()
        instance.build_origin('connman', {'CARGO_BUILD_JOBS': '1'})
        calls = instance.chroot.call_args_list
        self.assertEqual(calls[0].args, ('sh', '-ec', 'cd /project/aports/connman && abuild -F deps'))
        self.assertEqual(calls[0].kwargs['extra_env']['SUDO_APK'], 'apk')
        self.assertEqual(calls[1].args, ('su', '-s', '/bin/sh', 'builder', '-c', 'cd /project/aports/connman && abuild'))
        self.assertNotIn('SUDO_APK', calls[1].kwargs['extra_env'])
        self.assertTrue(calls[2].args[-1].endswith('abuild -F undeps'))

    def test_failed_compilation_still_cleans_dependencies_and_fails(self):
        instance = object.__new__(builder.Builder)
        instance.chroot = Mock(side_effect=[None, RuntimeError('compiler failed'), None])
        with self.assertRaisesRegex(RuntimeError, 'compiler failed'):
            instance.build_origin('uflinux-settings', {})
        self.assertTrue(instance.chroot.call_args_list[-1].args[-1].endswith('abuild -F undeps'))
