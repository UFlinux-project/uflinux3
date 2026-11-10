import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('desktop_performance_gate', Path(__file__).resolve().parents[1] / 'scripts/desktop-performance-gate.py')
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


def report(pss=100 * gate.MIB, latency=20):
    session = {
        'memory': {'settled_pss_bytes': pss, 'exercised_pss_bytes': pss,
                   'warmed_pss_bytes': pss, 'cycled_pss_bytes': pss},
        'idle': {'seconds': 60, 'desktop_percent': .1, 'combined_percent': .2},
        'stable_resources': True,
        'menus': {name: {'first_ms': latency, 'samples_ms': [latency] * 100} for name in gate.MENUS},
    }
    return {'measurement': 'input-to-usable-after-paint',
            'environment': {'renderer': 'cairo', 'compositor': 'labwc', 'scale': 1, 'catalog': 'fixture',
                            'tray_fixture': 'fixture', 'device_fixture': 'fixture', 'machine': 'test'},
            'sessions': [copy.deepcopy(session) for _ in range(3)]}


class DesktopPerformanceGateTests(unittest.TestCase):
    def test_comparable_improvement_passes(self):
        result = gate.compare(report(), report(89 * gate.MIB))
        self.assertTrue(result['passed'], result)

    def test_regressions_fail_gate(self):
        replacement = report(95 * gate.MIB)
        replacement['sessions'][0]['stable_resources'] = False
        replacement['sessions'][0]['memory']['cycled_pss_bytes'] += 6 * gate.MIB
        for session in replacement['sessions']:
            session['menus']['network']['samples_ms'] = [30] * 100
            session['idle']['combined_percent'] = .5
        result = gate.compare(report(), replacement)
        self.assertFalse(result['passed'])
        self.assertTrue(any('network' in failure for failure in result['failures']))
        self.assertTrue(any('retained PSS' in failure for failure in result['failures']))
        self.assertTrue(any('combined idle CPU' in failure for failure in result['failures']))

    def test_incomplete_or_unmatched_measurements_are_rejected(self):
        baseline = report()
        replacement = report(89 * gate.MIB)
        replacement['environment']['scale'] = 2
        with self.assertRaises(ValueError):
            gate.compare(baseline, replacement)
        replacement = report(89 * gate.MIB)
        replacement['sessions'][0]['menus']['start']['samples_ms'] = [1] * 99
        with self.assertRaises(ValueError):
            gate.compare(baseline, replacement)


if __name__ == '__main__':
    unittest.main()
