import copy
import importlib.util
from pathlib import Path
import unittest


spec = importlib.util.spec_from_file_location('desktop_memory_summary',
    Path(__file__).resolve().parents[1] / 'scripts/desktop-memory-summary.py')
summary = importlib.util.module_from_spec(spec)
spec.loader.exec_module(summary)


def session(mode, pss):
    return {'mode': mode, 'accounting_version': 2, 'cycles': 100,
            'environment': {'fixture': 'same'}, 'binary_sha256': {'settingsd': 'same', 'desktop': mode},
            'memory': {'settled_pss_bytes': pss, 'exercised_pss_bytes': pss,
                       'warmed_pss_bytes': pss, 'cycled_pss_bytes': pss},
            'file_descriptors': {'warmed': 10, 'cycled': 10},
            'idle': {'seconds': 60, 'desktop_percent': .1, 'combined_percent': .2}}


class DesktopMemorySummaryTests(unittest.TestCase):
    def setUp(self):
        self.baseline = [session('baseline', 100 * 1024 * 1024) for _ in range(3)]
        self.replacement = [session('replacement', 89 * 1024 * 1024) for _ in range(3)]

    def test_summarizes_memory_without_claiming_release_approval(self):
        result = summary.summarize(self.baseline, self.replacement)
        self.assertTrue(result['memory_cpu_checks_passed'])
        self.assertIn('not a desktop release gate', result['scope'])

    def test_detects_retention_and_per_session_cpu(self):
        self.replacement[0]['memory']['cycled_pss_bytes'] += 6 * 1024 * 1024
        self.replacement[0]['idle']['desktop_percent'] = 1.1
        result = summary.summarize(self.baseline, self.replacement)
        self.assertEqual(len(result['failures']), 2)

    def test_rejects_inconsistent_or_incomplete_sessions(self):
        changed = copy.deepcopy(self.replacement)
        changed[0]['environment']['fixture'] = 'different'
        with self.assertRaises(ValueError):
            summary.summarize(self.baseline, changed)
        changed = copy.deepcopy(self.replacement)
        changed[0]['cycles'] = 99
        with self.assertRaises(ValueError):
            summary.summarize(self.baseline, changed)


if __name__ == '__main__':
    unittest.main()
