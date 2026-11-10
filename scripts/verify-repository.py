#!/usr/bin/env python3
"""Check signed repository clients in a fresh root, without a private signing key."""
import argparse
import json
from pathlib import Path
import re
import shutil
import subprocess
import urllib.request

from builder import ARCHES, Builder, PROJECT, repository_url, sha, temporary_tree


def bounded_download(url, limit=1024 * 1024):
    with urllib.request.urlopen(url, timeout=30) as response:
        contents = response.read(limit + 1)
    if len(contents) > limit:
        raise ValueError('Repository metadata exceeds its size limit')
    return contents


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--outdir', default='out')
    parser.add_argument('--version', required=True)
    parser.add_argument('--arch', choices=ARCHES, default='x86_64')
    parser.add_argument('--staging', type=Path, help='Local repository root containing architecture directories')
    parser.add_argument('--repository-url', help='Published HTTPS repository root; inferred from GITHUB_REPOSITORY by default')
    args = parser.parse_args()
    out = Path(args.outdir).resolve()
    expected_key = out / 'repository/uflinux.rsa.pub'
    public_key = expected_key.read_bytes()
    repository = repository_url(args.repository_url)
    if args.staging:
        staging = args.staging.resolve()
        manifest = (staging / 'releases.json').read_bytes()
        if (staging / 'uflinux.rsa.pub').read_bytes() != public_key:
            raise ValueError('Staged repository key differs from the verified image key')
    else:
        manifest = bounded_download(repository + '/releases.json')
        if bounded_download(repository + '/uflinux.rsa.pub', 16384) != public_key:
            raise ValueError('Published repository key differs from the verified image key')
    history = json.loads(manifest)
    if not history or history[0]['version'] != args.version:
        raise ValueError('Repository does not expose the expected release generation')
    versions = []
    for release in history:
        if not re.fullmatch(r'[0-9]+\.[0-9]+\.[0-9]+(?:_(?:alpha|beta|rc)[0-9]+)?', release['version']):
            raise ValueError('Invalid release version')
        names = release['packages'][args.arch]
        defaults = [name for name in names if re.fullmatch(r'uflinux-defaults-' + re.escape(release['version']) + r'-r[0-9]+\.apk', name)]
        if len(defaults) != 1:
            raise ValueError('Expected one defaults package per release generation')
        versions.append(defaults[0].removeprefix('uflinux-defaults-').removesuffix('.apk'))
    report = out / 'checks' / args.arch / ('repository-staging.json' if args.staging else 'repository-published.json')
    report.unlink(missing_ok=True)
    work = PROJECT / '.state/repository-clients'
    work.mkdir(parents=True, exist_ok=True)
    with temporary_tree(prefix='client-', dir=work) as directory:
        builder = Builder(argparse.Namespace(arch=args.arch, version=args.version, outdir=str(out),
            cache=str(PROJECT / '.cache/build'), workdir=str(directory), repository_url=repository, release=False))
        builder.bootstrap()
        root = builder.root
        if args.staging:
            shutil.copytree(staging / args.arch, root / 'published' / args.arch)
            source = '/published'
        else:
            source = repository
        builder.put('etc/apk/repositories', source + '\n')
        # A fresh root and cache ensure an already-trusted index cannot hide a
        # missing key. Keep upstream keys, but deliberately withhold our key.
        rejected = subprocess.run(['chroot', str(root), 'apk', '--no-cache', 'update'], capture_output=True, text=True)
        untrusted_install = subprocess.run(['chroot', str(root), 'apk', '--no-cache', 'add', '--simulate', 'uflinux-defaults=' + versions[0]], capture_output=True, text=True)
        # apk 3 may return zero from update while warning and discarding an
        # untrusted index. The package must still be unavailable to the solver.
        if untrusted_install.returncode == 0 or not re.search(r'untrusted|signature', rejected.stdout + rejected.stderr, re.I):
            raise RuntimeError('The repository index was not rejected without its signing key: ' + rejected.stdout + rejected.stderr)
        builder.put('etc/apk/keys/uflinux.rsa.pub', public_key.decode())
        def apk(*command):
            return builder.chroot('apk', '--no-cache', *command)
        apk('update')
        # The oldest retained generation exercises a real version upgrade after
        # the first release. Initial publication has no previous version to test.
        initial = versions[-1]
        apk('add', 'uflinux-defaults=' + initial)
        system_config = root / 'etc/xdg/labwc/rc.xml'
        marker = '\n/* repository client preservation test */\n'
        system_config.write_text(system_config.read_text() + marker)
        user_config = builder.put('home/validation/.config/labwc/rc.xml', '# personal override\n')
        # Normal world entries are unversioned. Remove the test's initial pin so
        # apk upgrade can select the current version from the signed index.
        world = root / 'etc/apk/world'
        world.write_text('\n'.join('uflinux-defaults' if entry.startswith('uflinux-defaults=') else entry
                                   for entry in world.read_text().splitlines()) + '\n')
        apk('upgrade')
        apk('info', '-e', 'uflinux-defaults=' + versions[0])
        if marker not in system_config.read_text() or user_config.read_text() != '# personal override\n':
            raise RuntimeError('Repository upgrade changed local configuration')
        apk('fix', '--reinstall', 'uflinux-defaults')
        if marker not in system_config.read_text():
            raise RuntimeError('Repository reinstall replaced modified system defaults')
        # Verify corruption rejection against the archive downloaded through this
        # repository layout, rather than an unrelated compiler output.
        name = 'uflinux-defaults-' + versions[0] + '.apk'
        data = (staging / args.arch / name).read_bytes() if args.staging else bounded_download(repository + '/' + args.arch + '/' + name, 16 * 1024 * 1024)
        damaged = bytearray(data)
        damaged[-12] ^= 1
        (root / 'tmp/tampered.apk').write_bytes(damaged)
        result = subprocess.run(['chroot', str(root), 'apk', 'verify', '/tmp/tampered.apk'], capture_output=True)
        if result.returncode == 0:
            raise RuntimeError('Repository client accepted a damaged package')
        report.parent.mkdir(parents=True, exist_ok=True)
        report.write_text(json.dumps({'arch': args.arch, 'version': args.version,
            'source': 'staging' if args.staging else repository, 'public_key_sha256': sha(expected_key),
            'fresh_install': True, 'untrusted_index_rejected': True, 'damaged_package_rejected': True,
            'system_and_user_configuration_preserved': True, 'reinstall': True,
            'upgrade_from': initial if initial != versions[0] else None,
            'upgrade_to': versions[0], 'upgrade_note': 'No earlier release exists' if initial == versions[0] else 'Retained release upgraded through the signed index'}, indent=2) + '\n')
    print('Signed repository client checks passed: ' + str(report))


if __name__ == '__main__':
    main()
