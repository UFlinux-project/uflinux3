#!/usr/bin/env python3
"""Exercise BusyBox's actual ACPI event parser against a harmless private hook."""
import os
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import time

data=Path(__file__).resolve().parents[1]/'settings/data'
with tempfile.TemporaryDirectory(prefix='uflinux-acpid-test-') as directory:
    assert os.geteuid()==0, 'Run this private chroot test as root'
    root=Path(directory)
    for name in ('dev/input','bin','lib','run','etc/acpi'): (root/name).mkdir(parents=True,exist_ok=True)
    shutil.copy2('/bin/busybox',root/'bin/busybox');(root/'bin/sh').symlink_to('busybox')
    for loader in [*Path('/lib').glob('ld-musl-*.so.1'),*Path('/lib').glob('libc.musl-*.so.1')]:
        shutil.copy2(loader,root/'lib'/loader.name,follow_symlinks=True)
    fifo=root/'dev/input/event0';os.mkfifo(fifo)
    (root/'actions').write_bytes((data/'uflinux-acpid.conf').read_bytes())
    (root/'map').write_bytes((data/'uflinux-acpi.map').read_bytes())
    record=root/'forwarded';hook=root/'etc/acpi/power'
    hook.write_text('#!/bin/sh\nprintf "handled\\n" >> /forwarded\n');hook.chmod(0o755)
    writer=os.open(fifo,os.O_RDWR|os.O_NONBLOCK)
    child=subprocess.Popen(['chroot',str(root),'/bin/busybox','acpid','-f','-c','/etc/acpi',
        '-a','/actions','-M','/map','-p','/run/pid','-l','/log'])
    try:
        time.sleep(0.2)
        for count in range(1,4):
            os.write(writer,struct.pack('@llHHi',0,0,1,116,1))
            for _ in range(50):
                if record.exists() and len(record.read_text().splitlines())==count: break
                assert child.poll() is None, (root/'log').read_text()
                time.sleep(0.02)
            else: raise AssertionError('BusyBox did not forward KEY_POWER: '+(root/'log').read_text())
        for code,value in ((116,0),(116,2),(142,1)):
            os.write(writer,struct.pack('@llHHi',0,0,1,code,value))
        time.sleep(0.2)
        assert len(record.read_text().splitlines())==3, 'Release, repeat or unowned events reached the power hook'
    finally:
        child.terminate();child.wait(timeout=5);os.close(writer)
print('BusyBox ACPI power-event forwarding and unrelated-event rejection passed')
