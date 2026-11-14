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
        self.assertEqual(len(result['matrix']['include']),6)
        self.assertEqual(result['release'],'false')
    def test_default_manual_run_smokes_x86_64_only(self):
        result=ci.configuration('workflow_dispatch','refs/heads/main')
        self.assertEqual([m['arch'] for m in result['matrix']['include']],['x86_64'])
        self.assertEqual(result['release'],'false')
    def test_only_valid_pushed_version_tags_enable_release(self):
        result=ci.configuration('push','refs/tags/v0.1.0')
        self.assertEqual(len(result['matrix']['include']),6)
        self.assertEqual(result['release'],'true')
        for tag in ('vlatest','v0.1.0;id','v0.1.0\nmalformed'):
            with self.assertRaises(ValueError):ci.configuration('push','refs/tags/'+tag)

    def test_full_matrix_runs_still_boot_only_the_x86_64_images(self):
        for full in (False, True):
            jobs = ci.configuration('workflow_dispatch', 'refs/heads/main', full)['vm']['include']
            self.assertEqual([(job['arch'], job['mode']) for job in jobs],
                             [('x86_64', 'bios'), ('x86_64', 'uefi')])
        release = ci.configuration('push', 'refs/tags/v0.1.0')['vm']['include']
        self.assertEqual({job['arch'] for job in release}, {'x86_64'})

    def test_device_images_are_supplementary_outside_releases(self):
        for event,ref,full,devices in [('push','refs/heads/main',False,False),
                                      ('pull_request','refs/heads/main',False,True),
                                      ('workflow_dispatch','refs/heads/main',True,False)]:
            result=ci.configuration(event,ref,full,devices)
            self.assertEqual(result['devices']['include'],[])
            self.assertEqual(result['devices_enabled'],'false')
    def test_manual_full_run_can_build_device_images_without_releasing(self):
        result=ci.configuration('workflow_dispatch','refs/heads/main',True,True)
        self.assertEqual(result['release'],'false')
        self.assertEqual(result['devices_enabled'],'true')
        self.assertEqual(len(result['devices']['include']),5)
    def test_release_tags_build_every_device_and_label_experimental_targets(self):
        result=ci.configuration('push','refs/tags/v0.1.0')
        self.assertEqual(result['devices_enabled'],'true')
        jobs=result['devices']['include']
        self.assertEqual(sorted((job['device'],job['arch']) for job in jobs),
                         [('qcs6490','aarch64'),('rpi','aarch64'),('spacemit-k1','riscv64'),
                          ('vm','aarch64'),('vm','x86_64')])
        self.assertEqual({job['device'] for job in jobs if job['experimental']=='true'},
                         {'qcs6490','spacemit-k1'})
        for job in jobs:
            self.assertEqual(job['runner'],ci.arches[job['arch']]['runner'])

    def test_each_architecture_has_four_independent_jobs(self):
        for event,ref,full,count in [('pull_request','refs/heads/main',False,5),
                                     ('workflow_dispatch','refs/heads/main',True,30)]:
            result=ci.configuration(event,ref,full)
            jobs=result['packages']['include']
            self.assertEqual(len(jobs),count)
            for architecture in result['matrix']['include']:
                selected=[job for job in jobs if job['arch']==architecture['arch']]
                self.assertEqual({job['package'] for job in selected},{'connman','gpu-screen-recorder','uflinux-regreet','uflinux-settings','uflinux'})
                self.assertTrue(all(job['runner']==architecture['runner'] for job in selected))
