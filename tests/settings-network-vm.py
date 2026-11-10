#!/usr/bin/env python3
"""Real ConnMan+iwd/hostapd integration, ONLY in the disposable QEMU live VM.

Requires hostapd, dnsmasq, iw, py3-gobject3 and mac80211_hwsim. Unlike the D-Bus
fixture, this exercises association, WPA/EAP, DHCP and actual kernel routes.
"""
import json
import os
from pathlib import Path
import pwd
import shutil
import subprocess
import tempfile
import time
import sys

from gi.repository import Gio, GLib

assert os.geteuid() == 0 and Path('/run/uflinux-test-vm').is_file(), 'Disposable VM only'
assert 'QEMU' in Path('/sys/class/dmi/id/sys_vendor').read_text(), 'QEMU only'
bus = Gio.bus_get_sync(Gio.BusType.SYSTEM)
children = []
records = []
user = pwd.getpwnam('live')

def command(*args):
    return subprocess.check_output(args, text=True, timeout=45)

def system(operation):
    reply = bus.call_sync('org.uflinux.Settings.System1', '/org/uflinux/Settings/System',
                          'org.uflinux.Settings.System1', 'Execute',
                          GLib.Variant('(s)', (json.dumps(operation),)), GLib.VariantType('(s)'),
                          Gio.DBusCallFlags.NONE, 45000, None)
    return json.loads(reply.unpack()[0])

def live_environment():
    for entry in Path('/proc').glob('[0-9]*/exe'):
        try:
            if entry.resolve().name == 'uflinux-settingsd' and entry.parent.stat().st_uid == user.pw_uid:
                return dict(item.split('=', 1) for item in (entry.parent/'environ').read_bytes().decode().split('\0') if '=' in item)
        except OSError:
            pass
    raise AssertionError('Live Settings session is missing')

def demote():
    os.initgroups(user.pw_name, user.pw_gid)
    os.setgid(user.pw_gid)
    os.setuid(user.pw_uid)

environment = live_environment()
def request(action=None):
    result = subprocess.run(['uflinux-settingsctl', 'request' if action else 'status'],
        input=json.dumps(action) if action else None, env=environment, preexec_fn=demote,
        text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=100)
    if result.returncode:
        # Never print requests: enterprise operations contain passwords and keys.
        raise AssertionError('Session request failed: ' + result.stderr)
    return json.loads(result.stdout)

def wait_for(predicate, description, seconds=45):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(0.5)
    raise AssertionError('Timed out: ' + description)

def services():
    return request()['network']['services']

def find(ssid):
    return next((s for s in services() if s['name'] == ssid), None)

def discover(ssid):
    deadline = time.monotonic() + 90
    next_scan = 0
    last_scan_error = None
    while time.monotonic() < deadline:
        if time.monotonic() >= next_scan:
            try:
                request({'action':'scan'})
                last_scan_error = None
            except AssertionError as error:
                if not any(word in str(error) for word in ('Busy','InProgress','in progress','NotSupported')):raise
                last_scan_error = str(error)
            next_scan = time.monotonic() + 4
        service=find(ssid)
        if service:return service
        time.sleep(0.5)
    raise AssertionError('Could not discover '+ssid+'; last scan error: '+str(last_scan_error)+
                         '; visible services: '+str([s['name'] for s in services()]))

def connected(ssid):
    s = find(ssid)
    return s if s and s['state'] in ('ready', 'online') else None

def connect(service, ssid):
    # Answer the real ConnMan->iwd credential agent while Connect is pending.
    child=subprocess.Popen(['uflinux-settingsctl','request'],env=environment,preexec_fn=demote,
        stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
    child.stdin.write(json.dumps({'action':'connect','service':service}));child.stdin.close()
    deadline=time.monotonic()+90
    while child.poll() is None and time.monotonic()<deadline:
        auth=request()['network']['authentication']
        if auth:
            values={}
            for field in auth['fields']:
                name=field['name']
                assert name in ('Passphrase','Password','Identity','Username','Name','SSID'), 'Unexpected agent field: '+name
                values[name]=ssid if name in ('Name','SSID') else 'test' if name in ('Identity','Username') else 'uflinux-test-password'
            request({'action':'auth-reply','id':auth['id'],'values':values})
        time.sleep(0.2)
    if child.poll() is None: child.kill();child.wait();raise AssertionError('Association timed out for '+ssid)
    assert child.returncode==0, 'Association failed for '+ssid+': '+child.stderr.read()

original_iwd = Path('/etc/conf.d/iwd').read_bytes()
original_main = Path('/etc/iwd/main.conf').read_bytes()
original_connman = Path('/etc/conf.d/connman').read_bytes()
command('modprobe', 'mac80211_hwsim', 'radios=2')
phys=sorted(p.name for p in Path('/sys/class/ieee80211').iterdir())
assert len(phys)==2, 'The isolated VM must have exactly two simulated Wi-Fi radios'
ap, station = 'ufap0', 'ufsta0'
# The headless CLI has no polkit agent. Permit only this disposable VM's live
# user to perform the connect/forget authorization exercised by this fixture.
policy_rule = Path('/etc/polkit-1/rules.d/10-uflinux-network-vm.rules')
assert not policy_rule.exists(), 'Refusing to overwrite an existing polkit rule'
try:
    policy_rule.parent.mkdir(parents=True, exist_ok=True)
    policy_rule.write_text("polkit.addRule(function(action, subject) {\n"
        "    if (action.id == 'org.uflinux.settings.connect' && subject.user == 'live')\n"
        "        return polkit.Result.YES;\n"
        "});\n")
    policy_rule.chmod(0o644)
    command('rc-service', 'connman', 'stop')
    command('rc-service', 'iwd', 'stop')
    for entry in Path('/sys/class/net').iterdir():
        if (entry/'phy80211').exists(): command('iw','dev',entry.name,'del')
    command('iw','phy',phys[0],'interface','add',ap,'type','__ap')
    command('iw','phy',phys[1],'interface','add',station,'type','managed')
    for radio in Path('/sys/class/rfkill').glob('rfkill*'):
        if (radio/'type').read_text().strip()=='wlan': (radio/'soft').write_text('0')
    Path('/etc/iwd/main.conf').write_bytes(original_main+b'\n[DriverQuirks]\nDefaultInterface=mac80211_hwsim\n')
    Path('/etc/conf.d/iwd').write_text('command_args="--phys=' + phys[1] + ' --interfaces=' + station + '"\n')
    Path('/etc/conf.d/connman').write_bytes(original_connman+b'\ncommand_args="$command_args --nodevice=ufap0"\n')
    command('ip', 'address', 'add', '10.42.0.1/24', 'dev', ap)
    command('ip', 'link', 'set', ap, 'up')
    command('rc-service', 'iwd', 'start')
    command('rc-service', 'connman', 'start')
    wait_for(lambda: request()['network']['available'], 'ConnMan service restart')
    try: request({'action':'wifi', 'enabled':False})
    except AssertionError as error:
        if 'AlreadyDisabled' not in str(error): raise
    time.sleep(0.3)
    request({'action':'wifi', 'enabled':True})
    for radio in Path('/sys/class/rfkill').glob('rfkill*'):
        if (radio/'type').read_text().strip()=='wlan': (radio/'soft').write_text('0')
    command('ip', 'address', 'replace', '10.42.0.1/24', 'dev', ap)
    command('ip', 'link', 'set', ap, 'up')
    def station_ready():
        objects=bus.call_sync('net.connman.iwd','/','org.freedesktop.DBus.ObjectManager','GetManagedObjects',None,None,0,5000,None).unpack()[0]
        return any(interfaces.get('net.connman.iwd.Device',{}).get('Name')==station and
            interfaces.get('net.connman.iwd.Device',{}).get('Powered') and 'net.connman.iwd.Station' in interfaces for interfaces in objects.values())
    wait_for(station_ready,'iwd station initialization')
    with tempfile.TemporaryDirectory(prefix='uflinux-radio-test-') as directory:
        root = Path(directory)
        def spawn(name, args):
            log = (root/(name+'.log')).open('w+')
            child = subprocess.Popen(args, stdout=log, stderr=log)
            children.append((child, log))
            time.sleep(1)
            assert child.poll() is None, name + ' did not start: ' + (root/(name+'.log')).read_text()
            return child
        spawn('dhcp', ['dnsmasq','--keep-in-foreground','--conf-file=/dev/null','--user=root','--port=0','--interface='+ap,
            '--bind-interfaces','--dhcp-range=10.42.0.10,10.42.0.30,255.255.255.0,2m',
            '--dhcp-leasefile='+str(root/'leases'),'--pid-file='+str(root/'dhcp.pid')])
        # CA/server/client keys are ephemeral and never leave this disposable VM.
        for name, subject in [('ca','/CN=UFLinux Test CA'), ('server','/CN=radius.uflinux.test'), ('client','/CN=test')]:
            args=['openssl','req','-new','-newkey','rsa:2048','-nodes','-keyout',str(root/(name+'.key')),
                  '-out',str(root/(name+'.csr')),'-subj',subject]
            command(*args)
            if name == 'ca':
                (root/'ca.ext').write_text('basicConstraints=critical,CA:TRUE\nkeyUsage=critical,keyCertSign,cRLSign\nsubjectKeyIdentifier=hash\n')
                command('openssl','x509','-req','-in',str(root/'ca.csr'),'-signkey',str(root/'ca.key'),'-days','1',
                    '-extfile',str(root/'ca.ext'),'-out',str(root/'ca.pem'))
            else:
                ext = root/(name+'.ext')
                ext.write_text('basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage='+('serverAuth\nsubjectAltName=DNS:radius.uflinux.test\n' if name=='server' else 'clientAuth\n'))
                command('openssl','x509','-req','-in',str(root/(name+'.csr')),'-CA',str(root/'ca.pem'),
                        '-CAkey',str(root/'ca.key'),'-CAcreateserial','-days','1','-extfile',str(ext),'-out',str(root/(name+'.pem')))
        (root/'users').write_text('* TLS,PEAP,TTLS\n"test" MSCHAPV2,TTLS-PAP "uflinux-test-password" [2]\n')
        for mode in ('psk', 'sae', 'hidden', 'PEAP', 'TTLS', 'TLS'):
            print('Testing real Wi-Fi '+mode,flush=True)
            ssid = 'UFLinux-' + mode
            config = f'interface={ap}\ndriver=nl80211\nssid={ssid}\nhw_mode=g\nchannel=1\nauth_algs=1\nwpa=2\nrsn_pairwise=CCMP\n'
            if mode in ('psk','hidden','sae'):
                config += 'wpa_key_mgmt='+('SAE\nieee80211w=2' if mode=='sae' else 'WPA-PSK')+'\nwpa_passphrase=uflinux-test-password\n'
                if mode=='hidden': config += 'ignore_broadcast_ssid=1\n'
            else:
                config += f'wpa_key_mgmt=WPA-EAP\nieee8021x=1\neap_server=1\neap_user_file={root}/users\nca_cert={root}/ca.pem\nserver_cert={root}/server.pem\nprivate_key={root}/server.key\n'
            (root/'hostapd.conf').write_text(config)
            hostapd=spawn('hostapd-'+mode,['hostapd',str(root/'hostapd.conf')])
            def ap_ready():
                output = (root/('hostapd-'+mode+'.log')).read_text()
                if hostapd.poll() is not None:
                    raise AssertionError('hostapd exited before enabling '+ssid+': '+output)
                return 'AP-ENABLED' in output
            wait_for(ap_ready, 'hostapd AP-ENABLED for '+ssid)
            if mode == 'hidden':
                system({'operation':'hidden-profile','ssid':ssid,'passphrase':'uflinux-test-password'})
            elif mode in ('PEAP','TTLS','TLS'):
                profile={'ssid':ssid,'method':mode,'identity':'test','anonymous_identity':'anonymous',
                    'password':'uflinux-test-password','server_domain':'radius.uflinux.test',
                    'ca_pem':(root/'ca.pem').read_text(),'client_certificate_pem':(root/'client.pem').read_text(),
                    'private_key_pem':(root/'client.key').read_text(),'key_password':''}
                system({'operation':'enterprise','profile':profile})
            service=discover(ssid)
            if mode=='PEAP':
                invalid=dict(profile,server_domain='wrong.uflinux.test')
                system({'operation':'enterprise','profile':invalid})
                try:connect(service['path'],ssid)
                except AssertionError:pass
                else:raise AssertionError('Enterprise TLS accepted a mismatched server domain')
                system({'operation':'enterprise','profile':profile})
                records.append('enterprise TLS server-domain mismatch rejection')
            connect(service['path'],ssid)
            service=wait_for(lambda:connected(ssid), 'associate and DHCP '+mode)
            assert service['ipv4'].get('Address','').startswith('10.42.0.'), service['ipv4']
            request({'action':'disconnect','service':service['path']})
            connect(service['path'],ssid)
            wait_for(lambda:connected(ssid),'reconnect '+mode)
            if mode == 'psk':
                config={'ipv4':{'Method':'manual','Address':'10.42.0.50','Netmask':'255.255.255.0','Gateway':'10.42.0.1'},
                    'ipv6':{'Method':'auto'},'dns':['192.0.2.53'],'domains':['uflinux.test'],
                    'proxy':{'method':'manual','url':'','servers':['http://10.42.0.1:3128'],'excludes':['localhost']}}
                request({'action':'configure-connection','service':service['path'],'config':config})
                wait_for(lambda:find(ssid)['ipv4'].get('Address')=='10.42.0.50','manual IPv4 application')
                observed=find(ssid)['config']
                assert observed['dns']==config['dns'] and observed['domains']==config['domains']
                assert observed['proxy']['method']=='manual' and observed['proxy']['servers']==config['proxy']['servers']
                request({'action':'configure-connection','service':service['path'],'config':{
                    'ipv4':{'Method':'dhcp'},'ipv6':{'Method':'auto'},'dns':[],'domains':[],
                    'proxy':{'method':'direct','url':'','servers':[],'excludes':[]}}})
                wait_for(lambda:connected(ssid) and find(ssid)['ipv4'].get('Address','').startswith('10.42.0.'),'restore DHCP')
                records.append('manual addressing, DNS/search domains and proxy configuration')
                foreign='198.51.100.0/24'
                command('ip','route','add',foreign,'dev',station,'proto','static','metric','333')
                route={'destination':'203.0.113.0/24','gateway':None,'metric':123}
                system({'operation':'set-routes','service':service['path'],'routes':[route]})
                assert '203.0.113.0/24' in command('ip','route','show','proto','242')
                request({'action':'disconnect','service':service['path']})
                connect(service['path'],ssid)
                wait_for(lambda:'203.0.113.0/24' in command('ip','route','show','proto','242'),'restore owned route')
                # Interface-down may remove unrelated routes in the kernel; add it
                # again before testing that UFLinux's removal preserves it.
                if foreign not in command('ip','route','show'): command('ip','route','add',foreign,'dev',station,'proto','static','metric','333')
                system({'operation':'set-routes','service':service['path'],'routes':[]})
                assert foreign in command('ip','route','show')
                command('ip','route','del',foreign,'dev',station,'proto','static','metric','333')
                records.append('static-route persistence and unrelated-route preservation')
            request({'action':'disconnect','service':service['path']})
            request({'action':'forget','service':service['path']})
            if mode in ('PEAP', 'TTLS', 'TLS'):
                known = Path('/var/lib/uflinux-settings/credentials.json')
                assert not known.exists() or ssid not in json.loads(known.read_text()), 'Enterprise credentials were not forgotten'
            hostapd.terminate();hostapd.wait(timeout=5)
            records.append(mode+' association, DHCP, reconnect and forget')
        request({'action':'wifi','enabled':False})
        request({'action':'wifi','enabled':True})
        records.append('radio power and service restart')
    assert all(p.stat().st_mode & 0o077 == 0 for p in Path('/var/lib/iwd').glob('*.8021x'))
    print('UFLINUX_NETWORK_VM:'+json.dumps({'passed':records}),flush=True)
finally:
    policy_rule.unlink(missing_ok=True)
    for child, log in reversed(children):
        if child.poll() is None: child.terminate();child.wait(timeout=5)
        if sys.exc_info()[0] is not None:
            log.seek(0);print('Network fixture '+Path(log.name).name+':\n'+log.read(32768),flush=True)
        log.close()
    if sys.exc_info()[0] is not None:
        print('Network system log tail:\n'+Path('/var/log/messages').read_text()[-16000:],flush=True)
    # Restore the distro's normal service selection before installation tests.
    command('rc-service','connman','stop');command('rc-service','iwd','stop')
    Path('/etc/conf.d/iwd').write_bytes(original_iwd)
    Path('/etc/iwd/main.conf').write_bytes(original_main)
    Path('/etc/conf.d/connman').write_bytes(original_connman)
    command('modprobe','-r','mac80211_hwsim')
    command('rc-service','iwd','start');command('rc-service','connman','start')
