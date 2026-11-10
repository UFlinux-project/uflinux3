#!/usr/bin/env python3
"""Summarize comparable desktop memory diagnostics, without granting release approval."""
import argparse
import json
from pathlib import Path
import statistics
import sys


def summarize(baselines, replacements):
    if len(baselines) < 3 or len(replacements) < 3:
        raise ValueError('At least three sessions of each implementation are required')
    environment = baselines[0]['environment']
    daemon = baselines[0]['binary_sha256']['settingsd']
    for name, sessions in (('baseline', baselines), ('replacement', replacements)):
        binaries = sessions[0]['binary_sha256']
        for session in sessions:
            if session['mode'] != name or session.get('accounting_version') != 2:
                raise ValueError(f'{name}: incompatible process accounting')
            if session['environment'] != environment or session['binary_sha256']['settingsd'] != daemon:
                raise ValueError('Sessions must use the same environment, fixtures, catalog and Settings daemon')
            if session['binary_sha256'] != binaries:
                raise ValueError(f'{name}: binary changed between sessions')
            if session['cycles'] < 100 or session['idle']['seconds'] < 60:
                raise ValueError(f'{name}: each session needs 100 cycles and 60 seconds of idle measurement')
    median = lambda sessions, group, key: statistics.median(s[group][key] for s in sessions)
    result = {'scope': 'memory and idle CPU only; this is not a desktop release gate',
              'environment': environment, 'sessions': {'baseline': len(baselines), 'replacement': len(replacements)},
              'metrics': {}, 'failures': []}
    for field in ('settled_pss_bytes', 'exercised_pss_bytes'):
        old, new = (median(sessions, 'memory', field) for sessions in (baselines, replacements))
        result['metrics'][field] = {'baseline': old, 'replacement': new}
        if old <= 0 or new > old * .9:
            result['failures'].append(f'{field}: reduction is below 10%')
    old_cpu, new_cpu = (median(sessions, 'idle', 'combined_percent') for sessions in (baselines, replacements))
    desktop_cpu = max(s['idle']['desktop_percent'] for s in replacements)
    result['metrics']['idle_cpu_percent'] = {'baseline_combined_median': old_cpu,
        'replacement_combined_median': new_cpu, 'replacement_desktop_max': desktop_cpu}
    if desktop_cpu > 1:
        result['failures'].append('replacement desktop exceeds 1% idle CPU in a session')
    if new_cpu > old_cpu + .2:
        result['failures'].append('median combined idle CPU exceeds baseline by more than 0.2 percentage points')
    for index, session in enumerate(replacements, 1):
        memory = session['memory']
        growth = memory['cycled_pss_bytes'] - memory['warmed_pss_bytes']
        if growth > 5 * 1024 * 1024:
            result['failures'].append(f'replacement session {index}: retained PSS grew by more than 5 MiB')
        if session['file_descriptors']['cycled'] > session['file_descriptors']['warmed']:
            result['failures'].append(f'replacement session {index}: file descriptor count grew')
    result['memory_cpu_checks_passed'] = not result['failures']
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--baseline', type=Path, action='append', required=True)
    parser.add_argument('--replacement', type=Path, action='append', required=True)
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    try:
        result = summarize([json.loads(path.read_text()) for path in args.baseline],
                           [json.loads(path.read_text()) for path in args.replacement])
    except (OSError, ValueError, TypeError, KeyError) as error:
        print(f'Cannot summarize desktop measurements: {error}', file=sys.stderr)
        return 2
    rendered = json.dumps(result, indent=2) + '\n'
    if args.output:
        args.output.write_text(rendered)
    print(rendered, end='')
    return 0 if result['memory_cpu_checks_passed'] else 1


if __name__ == '__main__':
    sys.exit(main())
