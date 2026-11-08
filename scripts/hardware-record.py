#!/usr/bin/env python3
"""Record installed-system hardware evidence without changing device settings."""
import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import platform
import subprocess

FEATURES=('hardware_gpu','vrr','hdr','ddc_ci','backlight','battery','suspend_resume')
parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--results',type=Path,help='Operator results described in docs/hardware-validation.md')
parser.add_argument('--output',type=Path)
args=parser.parse_args()
os_release=Path('/etc/os-release').read_text()
if not any(line.strip() in ('ID=uflinux','ID="uflinux"') for line in os_release.splitlines()):
    parser.error('Run inside an installed UFLinux graphical session')
state=json.loads(subprocess.check_output(['uflinux-settingsctl','status'],text=True,timeout=20))
def read(path):
    try:return Path(path).read_text().replace('\0','').strip()
    except OSError:return None
vendor=read('/sys/class/dmi/id/sys_vendor')
product=read('/sys/class/dmi/id/product_name')
board=read('/proc/device-tree/model')
identity=' '.join(s for s in (vendor,product,board,read('/sys/hypervisor/type'),read('/sys/firmware/devicetree/base/compatible'),read('/proc/cpuinfo')) if s)
virtual=any(name.lower() in identity.lower() for name in ('QEMU','KVM','VMware','VirtualBox','Bochs','linux,dummy-virt','riscv-virtio'))
manual=json.loads(args.results.read_text()) if args.results else {}
if set(manual)-set(FEATURES):parser.error('Unknown hardware result name')
results={}
for name in FEATURES:
    value=manual.get(name,{'status':'unverified','notes':''})
    if not isinstance(value,dict) or set(value)-{'status','notes'}:
        parser.error('Each result accepts only status and notes')
    status=value.get('status','unverified');notes=value.get('notes','')
    if status not in ('pass','fail','unavailable','unverified') or not isinstance(notes,str) or len(notes)>8192:
        parser.error('Invalid hardware result')
    if status in ('pass','fail') and not notes.strip():parser.error('Pass/fail requires the device, method and observation in notes')
    if virtual and status=='pass':parser.error('Virtual-machine results cannot certify physical hardware')
    results[name]={'status':status,'notes':notes,'evidence':'operator-reported' if args.results else 'not-tested'}
report={'schema':1,'recorded_utc':datetime.now(timezone.utc).isoformat(),
    'scope':'physical hardware inventory and operator evidence; not automatic certification',
    'machine':{'architecture':platform.machine(),'kernel':platform.release(),'vendor':vendor,'product':product,
        'board':board,'known_virtual_machine':virtual},
    'observed':{'about':state['about'],'renderer':state['displays']['renderer'],
        'outputs':state['displays']['outputs'],'brightness_devices':state['displays']['brightness'],
        'hdr_requested':state['displays']['hdr_requested'],'hdr_activation':'not reported by compositor',
        'power':state['power']},'results':results}
output=args.output or Path('out-settings/checks')/platform.machine()/'hardware.json'
output.parent.mkdir(parents=True,exist_ok=True)
output.write_text(json.dumps(report,indent=2)+'\n')
print(output)
