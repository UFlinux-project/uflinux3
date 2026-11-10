#!/usr/bin/env python3
"""Apply real TLP profiles only inside the installed disposable QEMU guest."""
import json
import os
from pathlib import Path
import subprocess
import sys
import pwd
import time

assert os.geteuid()==0 and 'QEMU' in Path('/sys/class/dmi/id/sys_vendor').read_text()
assert not Path('/run/uflinux-live').exists(), 'Installed-system test only'
def operation(value):
    result=subprocess.run(['uflinux-settingsctl','system-request'],input=json.dumps(value),
        text=True,stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=60)
    if result.returncode: raise AssertionError(result.stderr)
    return json.loads(result.stdout)
if '--restore-buttons' in sys.argv:
    operation({'operation':'set-power-buttons','on_ac':'poweroff','on_battery':'poweroff'})
    sys.exit(0)
initial=operation({'operation':'power-status'})
assert initial['available'] and initial['profile']=='balanced', initial
assert initial['button_handler_running'], initial
handler = int(Path('/run/uflinux-acpid.pid').read_text())
assert Path(f'/proc/{handler}/exe').resolve().name == 'busybox'
assert Path(f'/proc/{handler}/cmdline').read_bytes().split(b'\0')[:2] == [b'/bin/busybox', b'acpid']
for name in ('tlp-pd', 'power-profiles-daemon'):
    assert subprocess.run(['pgrep', '-f', '^(/usr/[^ ]*/)?'+name+'( |$)'],stdout=subprocess.DEVNULL).returncode == 1
user=pwd.getpwnam('alice')
environment=None
for entry in Path('/proc').glob('[0-9]*/exe'):
    try:
        if entry.resolve().name=='uflinux-settingsd' and entry.parent.stat().st_uid==user.pw_uid:
            environment=dict(item.split('=',1) for item in (entry.parent/'environ').read_bytes().decode().split('\0') if '=' in item);break
    except OSError: pass
assert environment, 'The installed Settings session is missing'
def demote():
    os.initgroups(user.pw_name,user.pw_gid);os.setgid(user.pw_gid);os.setuid(user.pw_uid)
def shared_profile():
    return json.loads(subprocess.check_output(['uflinux-settingsctl','status'],env=environment,preexec_fn=demote,text=True,timeout=10))['power']['profile']
seen=[]
for profile,code,label in [('power-saver','SAV','Power saver'),('balanced','BAL','Balanced'),('performance','PRF','Performance')]:
    observed=operation({'operation':'set-power-profile','profile':profile})
    assert observed['profile']==profile, observed
    assert 'TLP_PROFILE_DEFAULT='+code in Path('/etc/tlp.d/90-uflinux-profile.conf').read_text()
    assert json.loads(Path('/etc/uflinux/power.json').read_text())['profile']==profile
    for _ in range(50):
        if shared_profile()==profile:break
        time.sleep(0.1)
    else:raise AssertionError('Settings did not receive the power-profile update')
    seen.append(profile)
operation({'operation':'set-power-profile','profile':'balanced'})
for action in ('suspend','nothing','poweroff'):
    observed=operation({'operation':'set-power-buttons','on_ac':action,'on_battery':action})
    assert observed['policy']['on_ac']==action and observed['policy']['on_battery']==action
# The VM controller now presses the real ACPI power key and verifies no shutdown.
operation({'operation':'set-power-buttons','on_ac':'nothing','on_battery':'nothing'})
version=subprocess.check_output(['tlp-stat','--version'],text=True,timeout=10).strip()
print('UFLINUX_POWER_VM:'+json.dumps({'profiles':seen,'persistent_configuration':True,
    'button_policies':['suspend','nothing','poweroff'],'tlp_version':version,'busybox_acpid':True,
    'settings_service_synchronized':True}),flush=True)
