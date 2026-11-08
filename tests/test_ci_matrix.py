"""Keep the full architecture gate and release-secret eligibility independent."""
import importlib.util
from pathlib import Path
import unittest
path=Path(__file__).resolve().parents[1]/'scripts/ci-matrix.py'
spec=importlib.util.spec_from_file_location('ci_matrix',path)
ci=importlib.util.module_from_spec(spec);spec.loader.exec_module(ci)
class MatrixTests(unittest.TestCase):
    def test_main_and_prs_smoke_without_release_secrets(self):
        for event,ref in [('push','refs/heads/main'),('pull_request','refs/tags/v0.1.0')]:
            result=ci.configuration(event,ref)
            self.assertEqual([m['arch'] for m in result['matrix']['include']],['x86_64'])
            self.assertEqual(result['release'],'false')
    def test_manual_runs_cover_all_architectures_without_release_secrets(self):
        result=ci.configuration('workflow_dispatch','refs/heads/main')
        self.assertEqual(len(result['matrix']['include']),8)
        self.assertEqual(result['release'],'false')
    def test_only_valid_pushed_version_tags_enable_release(self):
        result=ci.configuration('push','refs/tags/v0.1.0')
        self.assertEqual(len(result['matrix']['include']),8)
        self.assertEqual(result['release'],'true')
        for tag in ('vlatest','v0.1.0;id','v0.1.0\nmalformed'):
            with self.assertRaises(ValueError):ci.configuration('push','refs/tags/'+tag)
