#!/usr/bin/env python3
"""Test the real GTK polkit agent in the installed disposable graphical session."""
import json
import os
from pathlib import Path
import pwd
import subprocess
import sys
import time
assert os.geteuid()==0 and 'QEMU' in Path('/sys/class/dmi/id/sys_vendor').read_text()
user=pwd.getpwnam('alice')
def operation(value):
    reply=subprocess.run(['uflinux-settingsctl','system-request'],input=json.dumps(value),text=True,capture_output=True,timeout=60)
    assert reply.returncode==0,reply.stderr
    return json.loads(reply.stdout)
def environment():
    for path in Path('/proc').glob('[0-9]*/exe'):
        try:
            if path.resolve().name=='uflinux-settingsd' and path.parent.stat().st_uid==user.pw_uid:
                return dict(item.split('=',1) for item in (path.parent/'environ').read_bytes().decode().split('\0') if '=' in item)
        except OSError:pass
    raise AssertionError('Graphical Settings service is missing')
def demote():
    os.initgroups(user.pw_name,user.pw_gid);os.setgid(user.pw_gid);os.setuid(user.pw_uid)
mode=sys.argv[1]
record=Path('/run/uflinux-auth-'+('grant' if 'grant' in mode else 'cancel')+'.json')
if mode.startswith('start-'):
    operation({'operation':'set-power-buttons','on_ac':'poweroff','on_battery':'poweroff'})
    # A separate root harness process owns the CLI/result file while QMP types.
    child=os.fork()
    if child==0:
        result=subprocess.run(['uflinux-settingsctl','request'],input=json.dumps({'action':'system','operation':{'operation':'set-power-buttons','on_ac':'nothing','on_battery':'nothing'}}),
            env=environment(),preexec_fn=demote,text=True,capture_output=True,timeout=60)
        record.write_text(json.dumps({'returncode':result.returncode,'stderr':result.stderr}))
        os._exit(0)
    time.sleep(2)
    assert not record.exists(), 'Authorization completed without showing the GTK agent prompt'
    print('The real polkit request is pending',flush=True)
else:
    for _ in range(100):
        if record.exists():break
        time.sleep(.1)
    else:raise AssertionError('Authorization did not finish')
    result=json.loads(record.read_text());status=operation({'operation':'power-status'})
    if mode=='check-cancel':
        assert result['returncode']!=0 and status['policy']['on_ac']=='poweroff',result
    else:
        assert result['returncode']==0 and status['policy']['on_ac']=='nothing',result
        operation({'operation':'set-power-buttons','on_ac':'poweroff','on_battery':'poweroff'})
    print('GTK polkit '+mode+' passed',flush=True)
