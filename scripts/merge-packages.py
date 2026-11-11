#!/usr/bin/env python3
"""Validate and collect independently built package origins before signing."""
import argparse
import json
from pathlib import Path
import re
import shutil
import tempfile

from builder import ARCHES, BUILD_KEY, LOCK, PACKAGE_NAMES, PROJECT, link_or_copy, sha, unsigned_digest
from sbom import apk_metadata


def collect(out, arch, version):
    out = Path(out).resolve()
    shards = out / 'package-shards' / arch
    if {p.name for p in shards.iterdir()} != set(PACKAGE_NAMES):
        raise ValueError('All package shards are required')
    keys, packages, records, baseline = {}, {}, {}, {}
    identity = None
    for origin, expected_names in PACKAGE_NAMES.items():
        shard = shards / origin
        source = shard / 'packages' / arch
        record = json.loads((shard / 'metadata' / arch / 'compile.json').read_text())
        if (record['arch'], record['version'], record['package'], record['sources']) != (arch, version, origin, LOCK):
            raise ValueError('Mismatched build inputs in shard ' + origin)
        current_identity = [record[key] for key in ('source_commit', 'source_date_epoch', 'repository_url')]
        if identity is not None and current_identity != identity:
            raise ValueError('Package shards come from different builds')
        identity = current_identity
        recipe = (PROJECT / 'aports' / origin / 'APKBUILD').read_text()
        revision = re.search(r'^pkgrel=([0-9]+)$', recipe, re.M)[1]
        upstream = {'uflinux-regreet': LOCK['uflinux-regreet']['version'], 'connman': LOCK['connman']['version'],
                    'uflinux-settings': LOCK['settings']['version'], 'uflinux': version}[origin]
        names, payloads = set(), {}
        key = (source / BUILD_KEY).read_text()
        for package in sorted(source.glob('*.apk')):
            info = apk_metadata(package)
            if (info.get('origin'), info.get('pkgver')) != (origin, upstream + '-r' + revision) or info.get('arch') not in {arch, 'noarch'}:
                raise ValueError('Incorrect package metadata: ' + package.name)
            if package.name != info['pkgname'] + '-' + info['pkgver'] + '.apk' or info['pkgname'] in names or package.name in packages:
                raise ValueError('Duplicate or invalid package: ' + package.name)
            relative = str(package.relative_to(shard))
            if record['files'].get(relative, {}).get('sha256') != sha(package):
                raise ValueError('Package differs from compilation record: ' + package.name)
            names.add(info['pkgname'])
            packages[package.name] = package
            keys[package.name] = key
            payloads[package.name] = unsigned_digest(package)
        if names != expected_names:
            raise ValueError('Incomplete or unexpected package set for ' + origin)
        inputs = shard / 'inputs' / arch
        required = {'build-apks.tar.gz', 'sources.tar.gz'}
        if origin in {'uflinux-regreet', 'uflinux-settings'}:
            required.add('cargo-sources.tar.gz')
        if not required.issubset({p.name for p in inputs.glob('*.tar.gz')}):
            raise ValueError('Missing rebuild inputs for ' + origin)
        if arch == 'x86_64':
            proof = json.loads((shard / 'metadata' / arch / 'reproducibility.json').read_text())
            if not proof.get('fresh_compiler_target') or not proof.get('matched') or proof.get('baseline') != payloads or proof.get('rebuilt') != payloads:
                raise ValueError('Reproducibility proof does not cover shard ' + origin)
            baseline.update(payloads)
        records[origin] = record
    # Nothing is copied into the signing input until the entire set validates.
    destination = out / 'packages' / arch
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='merged-', dir=destination.parent) as temporary:
        staging = Path(temporary)
        for name, package in packages.items():
            link_or_copy(package, staging / name)
        (staging / 'build-keys.json').write_text(json.dumps(keys, indent=2) + '\n')
        # Legacy callers use this key only as a fallback. build-keys.json selects
        # the exact shard key for each input APK during cryptographic verification.
        (staging / BUILD_KEY).write_text(next(iter(keys.values())))
        if destination.exists():
            shutil.rmtree(destination)
        staging.rename(destination)
        # TemporaryDirectory tolerates the renamed directory during cleanup.
    metadata = out / 'metadata' / arch
    metadata.mkdir(parents=True, exist_ok=True)
    for origin in PACKAGE_NAMES:
        shard = shards / origin
        shutil.copytree(shard / 'metadata' / arch, metadata / 'package-shards' / origin, dirs_exist_ok=True)
        target = out / 'inputs' / arch / origin
        if target.exists():
            shutil.rmtree(target)
        shutil.copytree(shard / 'inputs' / arch, target)
    merged = dict(next(iter(records.values())))
    merged.update(package=None, package_shards=list(PACKAGE_NAMES), files={
        'packages/' + arch + '/' + name: {'sha256': sha(package), 'bytes': package.stat().st_size}
        for name, package in packages.items()})
    (metadata / 'compile.json').write_text(json.dumps(merged, indent=2) + '\n')
    if arch == 'x86_64':
        (metadata / 'reproducibility.json').write_text(json.dumps({
            'arch': arch, 'version': version, 'fresh_compiler_target': True,
            'signature_streams_excluded': True, 'baseline': baseline, 'rebuilt': baseline,
            'matched': True, 'package_shards': list(PACKAGE_NAMES)}, indent=2) + '\n')
    print(f'Collected {len(packages)} APKs from {len(PACKAGE_NAMES)} independent {arch} jobs')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--arch', choices=ARCHES, required=True)
    parser.add_argument('--version', required=True)
    parser.add_argument('--outdir', default='out')
    args = parser.parse_args()
    collect(args.outdir, args.arch, args.version)


if __name__ == '__main__':
    main()
