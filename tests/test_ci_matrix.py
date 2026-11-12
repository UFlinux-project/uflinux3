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
    def test_main_and_prs_boot_only_x86_64_images(self):
        for event,ref in [('push','refs/heads/main'),('pull_request','refs/tags/v0.1.0')]:
            modes={}
            for job in ci.configuration(event,ref)['vm']['include']:
                modes.setdefault(job['arch'],set()).add(job['mode'])
            self.assertEqual(modes,{'x86_64':{'bios','uefi'}})
    def test_manual_runs_cover_all_architectures_without_release_secrets(self):
        result=ci.configuration('workflow_dispatch','refs/heads/main',full=True)
        self.assertEqual(len(result['matrix']['include']),7)
        self.assertEqual(result['release'],'false')
    def test_default_manual_run_smokes_x86_64_only(self):
        result=ci.configuration('workflow_dispatch','refs/heads/main')
        self.assertEqual([m['arch'] for m in result['matrix']['include']],['x86_64'])
        self.assertEqual(result['release'],'false')
    def test_only_valid_pushed_version_tags_enable_release(self):
        result=ci.configuration('push','refs/tags/v0.1.0')
        self.assertEqual(len(result['matrix']['include']),7)
        self.assertEqual(result['release'],'true')
        for tag in ('vlatest','v0.1.0;id','v0.1.0\nmalformed'):
            with self.assertRaises(ValueError):ci.configuration('push','refs/tags/'+tag)

    def test_bios_names_are_used_only_for_x86_vm_tests(self):
        jobs = ci.configuration('workflow_dispatch', 'refs/heads/main', full=True)['vm']['include']
        for arch in ('x86', 'x86_64'):
            self.assertEqual({job['mode'] for job in jobs if job['arch'] == arch}, {'bios', 'uefi'})
        for arch in ('ppc64le', 's390x'):
            self.assertEqual({job['mode'] for job in jobs if job['arch'] == arch}, {'native'})

    def test_each_architecture_has_four_independent_jobs(self):
        for event,ref,full,count in [('pull_request','refs/heads/main',False,5),
                                     ('workflow_dispatch','refs/heads/main',True,35)]:
            result=ci.configuration(event,ref,full)
            jobs=result['packages']['include']
            self.assertEqual(len(jobs),count)
            for architecture in result['matrix']['include']:
                selected=[job for job in jobs if job['arch']==architecture['arch']]
                self.assertEqual({job['package'] for job in selected},{'connman','gpu-screen-recorder','uflinux-regreet','uflinux-settings','uflinux'})
                self.assertTrue(all(job['runner']==architecture['runner'] for job in selected))
