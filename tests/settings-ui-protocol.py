#!/usr/bin/env python3
"""Operate real GTK controls over AT-SPI and the private Wayland input protocol."""
import json
import os
from pathlib import Path
import subprocess
import time
import pyatspi
from gi.repository import Gio,GLib

def command(*args):
    subprocess.run(args,check=True,stdout=subprocess.DEVNULL,stderr=subprocess.PIPE,timeout=15)
def state(): return json.loads(subprocess.check_output(['uflinux-settingsctl','status'],text=True,timeout=15))
def wait(predicate,description):
    for _ in range(80):
        while GLib.MainContext.default().pending(): GLib.MainContext.default().iteration(False)
        try:
            value=predicate()
            if value:return value
        except (RuntimeError,LookupError,AttributeError):pass
        time.sleep(.05)
    raise AssertionError(description)
def tree():
    root=pyatspi.Registry.getDesktop(0)
    queue=[a for a in root if a.name=='uflinux-settings']
    result=[]
    while queue:
        a=queue.pop();result.append(a)
        try:queue.extend(list(a))
        except Exception:pass
        assert len(result)<2000,'Unbounded accessible tree'
    return result
def find(name,role=None):
    return next((a for a in tree() if a.name==name and (not role or a.getRoleName()==role)),None)
def click(a):
    action=a.queryAction()
    assert action.nActions, 'Control has no keyboard/accessibility action: '+a.name
    assert action.doAction(0)
def page(name):
    command('uflinux-settings','--page',name);time.sleep(.2)

command('wlrctl','toplevel','focus','app_id:org.uflinux.Settings')
command('wtype','-M','ctrl','-M','alt','-k','F9','-m','alt','-m','ctrl')
command('wlrctl','toplevel','maximize','app_id:org.uflinux.Settings')
time.sleep(.3)
page('displays')
canvas=wait(lambda:find('Display layout'),'The layout preview must have an accessible name')
outputs=state()['displays']['outputs']
first=next(o for o in outputs if o['enabled'])
field=wait(lambda:find('Horizontal position · '+first['name'],'spin button'),'Horizontal position is unavailable')
before=field.queryValue().currentValue
# The frame contains the fitted layout; choose a point within its first monitor.
rect=canvas.queryComponent().getExtents(pyatspi.WINDOW_COORDS)
width=max(o['x']+o['current']['width']/o['scale'] for o in outputs if o['enabled'])
height=max(o['y']+o['current']['height']/o['scale'] for o in outputs if o['enabled'])
factor=min((rect.width-24)/width,(rect.height-24)/height)
x=int(rect.x+12+(first['x']+first['current']['width']/first['scale']*.3)*factor)
y=int(rect.y+12+(first['y']+first['current']['height']/first['scale']*.4)*factor)
driver=str(Path('/tmp/uflinux-ui-driver/pointer-driver'))
command(driver,str(x),str(y),str(x+30),str(y+8),str(int(width)),str(int(height)))
wait(lambda:field.queryValue().currentValue!=before,'Dragging did not update the numeric position control')
new_x=int(field.queryValue().currentValue)
click(find('Apply display settings','button'))
wait(lambda:state()['displays']['pending'],'Dragging was not applied')
wait(lambda:next(o['x'] for o in state()['displays']['outputs'] if o['name']==first['name'])==new_x,'Numeric field and applied layout differ')
click(wait(lambda:find('Revert','button'),'Missing display confirmation dialog'))
wait(lambda:state()['displays']['pending'] is None,'Display revert did not finish')
wait(lambda:find('Keep these display settings?','frame') is None,'Display confirmation dialog did not close')
time.sleep(.3)

# Install two harmless fixture applications, preserving a third association.
applications=Path(os.environ['XDG_DATA_HOME'])/'applications';applications.mkdir(parents=True,exist_ok=True)
for name in ('A','B'):
    (applications/('uflinux-test-'+name+'.desktop')).write_text('[Desktop Entry]\nType=Application\nName=UFLinux Test '+name+'\nExec=/bin/true %U\nMimeType=x-scheme-handler/http;x-scheme-handler/https;application/x-uflinux-unrelated;\n')
command('update-desktop-database',str(applications))
a=Gio.DesktopAppInfo.new('uflinux-test-A.desktop')
assert a and a.set_as_default_for_type('x-scheme-handler/http')
assert a.set_as_default_for_type('application/x-uflinux-unrelated')
page('default-apps')
label=wait(lambda:find('Web browser','label'),'Default browser row is missing')
row=label.parent
combo=next(child for child in row if child.getRoleName()=='combo box')
click(next(child for child in combo if child.getRoleName()=='toggle button'))
wait(lambda:find('UFLinux Test B','label'),'The browser dropdown did not open')
target=find('UFLinux Test B','label')
while target.getRoleName()!='list item':
    target=target.parent
    assert target is not None, 'The browser item is outside the dropdown list'
wait(lambda:target.getState().contains(pyatspi.STATE_SHOWING),'Browser choice is not visible')
# Keep one virtual keyboard alive across popup creation and activation. Creating
# a new keyboard after opening the popup can reset focus on wlroots compositors.
command('wtype','-k','Escape','-s','100','-k','space','-s','100','-k','Down','-s','100','-k','Return','-s','200')
wait(lambda:(Gio.AppInfo.get_default_for_type('x-scheme-handler/http',False).get_id()=='uflinux-test-B.desktop'),'The browser selection was not saved through GIO')
assert Gio.AppInfo.get_default_for_type('x-scheme-handler/https',False).get_id()=='uflinux-test-B.desktop'
assert Gio.AppInfo.get_default_for_type('application/x-uflinux-unrelated',False).get_id()=='uflinux-test-A.desktop'
print('GTK display drag/numeric synchronization, apply/revert and default-app association preservation passed')
