#!/usr/bin/env python3
"""Compare desktop performance reports from matching private labwc sessions.

This program evaluates measurements; it does not manufacture them. Each input
contains an environment object and at least three sessions. A session contains
memory (settled_pss_bytes, exercised_pss_bytes, warmed_pss_bytes,
cycled_pss_bytes), idle (seconds, desktop_percent, combined_percent), and
menus. Each of start, network, sound and power has first_ms and at least 100
activation-to-usable-after-paint measurements in samples_ms. Reports for the
new implementation also include stable_resources, set by the harness after
checking widgets, subscriptions and file descriptors across the 100 cycles.
"""
import argparse
import json
import math
from pathlib import Path
import statistics
import sys

MENUS = ('start', 'network', 'sound', 'power')
MIB = 1024 * 1024


def number(value, name):
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or value < 0:
        raise ValueError(f'{name} must be a non-negative finite number')
    return value


def p95(values):
    values = sorted(values)
    return values[math.ceil(len(values) * .95) - 1]


def validate(report, name, replacement=False):
    if report.get('measurement') != 'input-to-usable-after-paint':
        raise ValueError(f'{name}: latency measurement must be input-to-usable-after-paint')
    environment = report.get('environment')
    required = ('renderer', 'compositor', 'scale', 'catalog', 'tray_fixture', 'device_fixture', 'machine')
    if not isinstance(environment, dict) or any(key not in environment for key in required):
        raise ValueError(f'{name}: missing comparison environment fields')
    if environment['renderer'] != 'cairo':
        raise ValueError(f'{name}: native latency gates require the Cairo renderer')
    sessions = report.get('sessions')
    if not isinstance(sessions, list) or len(sessions) < 3:
        raise ValueError(f'{name}: at least three independent sessions are required')
    for index, session in enumerate(sessions):
        context = f'{name} session {index + 1}'
        memory = session['memory']
        for field in ('settled_pss_bytes', 'exercised_pss_bytes', 'warmed_pss_bytes', 'cycled_pss_bytes'):
            number(memory[field], f'{context} {field}')
        idle = session['idle']
        for field in ('seconds', 'desktop_percent', 'combined_percent'):
            number(idle[field], f'{context} idle {field}')
        if idle['seconds'] < 60:
            raise ValueError(f'{context}: idle sample must last at least 60 seconds')
        if replacement and not isinstance(session.get('stable_resources'), bool):
            raise ValueError(f'{context}: stable_resources evidence is required')
        for menu in MENUS:
            data = session['menus'][menu]
            number(data['first_ms'], f'{context} {menu} first_ms')
            samples = data['samples_ms']
            if not isinstance(samples, list) or len(samples) < 100:
                raise ValueError(f'{context}: {menu} requires at least 100 samples')
            for sample in samples:
                number(sample, f'{context} {menu} sample')
    return sessions


def compare(baseline, replacement):
    before = validate(baseline, 'baseline')
    after = validate(replacement, 'replacement', replacement=True)
    if baseline['environment'] != replacement['environment']:
        raise ValueError('baseline and replacement environments differ')
    failures = []
    metrics = {}
    for field in ('settled_pss_bytes', 'exercised_pss_bytes'):
        old = statistics.median(s['memory'][field] for s in before)
        new = statistics.median(s['memory'][field] for s in after)
        metrics[field] = {'baseline': old, 'replacement': new}
        if old == 0 or new > old * .9:
            failures.append(f'{field}: replacement must be at least 10% below baseline')
    base_cpu = statistics.median(s['idle']['combined_percent'] for s in before)
    new_cpu = statistics.median(s['idle']['combined_percent'] for s in after)
    desktop_cpu = statistics.median(s['idle']['desktop_percent'] for s in after)
    metrics['idle_cpu_percent'] = {'baseline_combined': base_cpu, 'replacement_combined': new_cpu, 'desktop': desktop_cpu}
    if desktop_cpu > 1:
        failures.append('desktop idle CPU exceeds 1% of one core')
    if new_cpu > base_cpu + .2:
        failures.append('combined idle CPU exceeds baseline plus 0.2 percentage points')
    for index, session in enumerate(after):
        retained = session['memory']['cycled_pss_bytes'] - session['memory']['warmed_pss_bytes']
        if retained > 5 * MIB:
            failures.append(f'session {index + 1}: retained PSS grew by more than 5 MiB')
        if not session['stable_resources']:
            failures.append(f'session {index + 1}: resources continued growing during menu cycles')
    metrics['menus'] = {}
    for menu in MENUS:
        old = p95([v for s in before for v in s['menus'][menu]['samples_ms']])
        new = p95([v for s in after for v in s['menus'][menu]['samples_ms']])
        first = max(s['menus'][menu]['first_ms'] for s in after)
        metrics['menus'][menu] = {'baseline_p95_ms': old, 'replacement_p95_ms': new, 'worst_first_ms': first}
        if first > 100:
            failures.append(f'{menu}: first opening exceeds 100 ms')
        if new > 50:
            failures.append(f'{menu}: p95 exceeds 50 ms')
        if new > old + max(5, old * .1):
            failures.append(f'{menu}: p95 regresses beyond allowed margin')
    return {'passed': not failures, 'failures': failures, 'metrics': metrics}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('baseline', type=Path)
    parser.add_argument('replacement', type=Path)
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    try:
        result = compare(json.loads(args.baseline.read_text()), json.loads(args.replacement.read_text()))
    except (ValueError, TypeError, KeyError, OSError) as error:
        print(f'Cannot evaluate desktop performance: {error}', file=sys.stderr)
        return 2
    rendered = json.dumps(result, indent=2) + '\n'
    if args.output:
        args.output.write_text(rendered)
    print(rendered, end='')
    return 0 if result['passed'] else 1


if __name__ == '__main__':
    sys.exit(main())
