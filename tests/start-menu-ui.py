#!/usr/bin/env python3
"""Real GTK/labwc launcher interactions in the private desktop smoke session."""
import json
import os
from pathlib import Path
import subprocess
import time
import sys
import select
import pyatspi
from gi.repository import GLib

results = Path('/tmp/uflinux-settings-smoke-results')
results.mkdir(exist_ok=True)
log = results / 'start-menu.log'
keyboard = subprocess.Popen(['/tmp/uflinux-ui-driver/keyboard-driver'], stdin=subprocess.PIPE,
    stdout=subprocess.PIPE, text=True, bufsize=1)
def keyboard_reply():
    assert select.select([keyboard.stdout], [], [], 20)[0], 'Virtual keyboard timed out'
    return keyboard.stdout.readline()
assert keyboard_reply() == 'ready\n', 'Virtual keyboard did not initialize'
def command(*args):
    if args[0] == 'wtype':
        # Keep the device attached across popups and hide/show cycles. Creating
        # a new virtual keyboard for each key can reset the compositor's focus.
        index = 1
        while index < len(args):
            option, value = args[index:index + 2]
            if option == '-k': request = 'key ' + value
            elif option in ('-M', '-m'): request = 'mod ' + value + (' 1' if option == '-M' else ' 0')
            else: raise AssertionError('Unsupported keyboard option: ' + option)
            keyboard.stdin.write(request + '\n'); keyboard.stdin.flush()
            assert keyboard_reply() == 'ok\n', 'Virtual keyboard request failed: ' + request
            index += 2
        return
    with log.open('a') as stream:
        subprocess.run(args, check=True, stdout=stream, stderr=stream, timeout=20)
def wait(predicate, description):
    for _ in range(100):
        while GLib.MainContext.default().pending(): GLib.MainContext.default().iteration(False)
        try:
            result = predicate()
            if result: return result
        except (RuntimeError, LookupError, AttributeError, GLib.GError): pass
        time.sleep(.05)
    raise AssertionError(description)
# Accessibles live inside the client processes. A resident app (e.g. Settings)
# can destroy a widget between reading a child list and querying the object, so
# every property read must tolerate atspi_error "Object does not exist" and
# treat the node as simply gone.
def role_of(item):
    try: return item.getRoleName()
    except GLib.GError: return None
def showing(item):
    try: return item.getState().contains(pyatspi.STATE_SHOWING)
    except GLib.GError: return False
def name_of(item):
    try: return item.name
    except GLib.GError: return None
def tree(root=None):
    queue = list(root) if root is not None else [a for a in pyatspi.Registry.getDesktop(0) if name_of(a) in ('uflinux-settings', 'uflinux-desktop')]
    found = []
    while queue:
        item = queue.pop(0)
        item_role = role_of(item)
        if item_role is None: continue
        if item_role == 'frame' and not showing(item): continue
        found.append(item)
        try: queue.extend([child for child in item if child])
        except Exception: pass
        assert len(found) < 6000, 'Unbounded launcher widget tree'
    return found
def find(name, role=None):
    for a in tree():
        if name_of(a) != name: continue
        if role is None or role_of(a) == role: return a
    return None
def menu(): return find('Start menu', 'frame')
def click(item):
    action = item.queryAction(); assert action.nActions, item.name
    assert action.doAction(0)
def diagnostics(kind, error, traceback):
    try:
        command('grim', str(results / 'start-menu-failure.png'))
        (results / 'start-menu-tree.json').write_text(json.dumps([(name_of(a), role_of(a)) for a in tree()], ensure_ascii=False, indent=2))
    except Exception: pass
    # A missing menu is usually because the resident desktop is gone: a client
    # then starts its own foreground shell and fails as a timeout instead.
    # Say so, and show the reason, rather than a tree of AT-SPI lookups.
    try:
        pid = int(os.environ['UFLINUX_DESKTOP_PID'])
    except (KeyError, ValueError):
        pid = 0
    # os.kill() would signal the whole process group for pid 0, so only probe a
    # pid that was actually reported.
    if pid and not os.path.isdir('/proc/%d' % pid):
        print('Resident desktop (pid %d) exited during this test' % pid, file=sys.stderr)
        try:
            tail = Path(os.environ['UFLINUX_DESKTOP_LOG']).read_text().splitlines()[-40:]
            print('\n'.join(tail), file=sys.stderr)
        except OSError: pass
    sys.__excepthook__(kind, error, traceback)
sys.excepthook = diagnostics
def open_menu():
    assert not menu(), 'An earlier Start menu remained open'
    command('uflinux-desktop', '--start-menu', '--output', 'HEADLESS-1')
    wait(menu, 'Start menu did not appear')
    wait(lambda: find('Search applications'), 'Search field is missing')
    wait(lambda: not find('Loading applications…'), 'Application catalog did not load')
    assert find('Search applications').queryText().getText(0, -1) == '', 'Search was retained after hiding'
    assert not find('Back to favorites'), 'Application browser was retained after hiding'
    command('grim', str(results / 'start-menu-default.png'))
def search(text):
    field = wait(lambda: find('Search applications'), 'Search field is missing')
    assert field.queryEditableText().setTextContents(text)
    time.sleep(.3)
def close_menu():
    command('uflinux-desktop', '--start-menu')
    wait(lambda: not menu(), 'Super-style toggle failed to close Start')
def pointer(x, y, width=1280, height=720):
    command('/tmp/uflinux-ui-driver/pointer-driver', str(x), str(y), str(x), str(y), str(width), str(height))

data = Path(os.environ['XDG_DATA_HOME'])
apps = data / 'applications'; apps.mkdir(parents=True, exist_ok=True)
helpers = data / 'launch helpers'; helpers.mkdir(exist_ok=True)
helper = helpers / 'record-launch'
helper.write_text('#!/bin/sh\nprintf "%s\\n" "$@" > "$XDG_RUNTIME_DIR/start-launch-args"\n')
helper.chmod(0o755)
marker = Path(os.environ['XDG_RUNTIME_DIR']) / 'start-launch-args'
created = []
def fixture(key, name, extra='', executable=None):
    path = apps / ('uflinux-launcher-test-' + key + '.desktop')
    path.write_text('[Desktop Entry]\nType=Application\nName=' + name + '\nComment=Launch catalog fixture\nKeywords=Unicode;Documents;\nIcon=utilities-terminal\nExec=' + (executable or '"' + str(helper) + '" "argument with spaces" %U') + '\n' + extra)
    created.append(path)
fixture('visible', 'UFLinux Fixture Straße')
fixture('hidden', 'UFLinux Fixture Hidden', 'Hidden=true\n')
fixture('nodisplay', 'UFLinux Fixture NoDisplay', 'NoDisplay=true\n')
fixture('only', 'UFLinux Fixture GNOME', 'OnlyShowIn=GNOME;\n')
fixture('not', 'UFLinux Fixture NotHere', 'NotShowIn=labwc;\n')
fixture('try', 'UFLinux Fixture MissingTryExec', 'TryExec=/uflinux-test-does-not-exist\n')
broken = apps / 'org.uflinux.NonexistentFixture.desktop'
broken.write_text('[Desktop Entry]\nType=Application\nName=UFLinux Fixture Broken\nDBusActivatable=true\n')
created.append(broken)
command('update-desktop-database', str(apps))
try:
    for _ in range(100):
        status = subprocess.check_output(['uflinux-desktop', '--status'], text=True, timeout=10)
        if json.loads(status)['ready']: break
        time.sleep(.05)
    else: raise AssertionError('Resident Start menu was not prepared')
    open_menu()
    assert find('Home', 'push button') or find('Home', 'button')
    assert not find('Power mode') and not find('Power settings…')
    click(find('All Applications', 'button') or find('All Applications', 'push button'))
    wait(lambda: find('Back to favorites'), 'Application browser did not open')
    command('wtype', '-k', 'Escape')
    wait(lambda: find('Favorites'), 'Escape did not return to favorites')
    search('STRASSE documents')
    wait(lambda: find('UFLinux Fixture Straße'), 'Unicode/token search missed the visible fixture')
    assert not find('UFLinux Fixture Hidden')
    command('wtype', '-k', 'Down')
    command('wtype', '-k', 'Return')
    wait(lambda: marker.exists(), 'GIO did not launch the desktop file')
    assert marker.read_text() == 'argument with spaces\n', 'Exec arguments or %U expansion changed'
    wait(lambda: not menu(), 'Launching an application did not close Start')
    open_menu(); search('UFLinux Fixture')
    wait(lambda: find('UFLinux Fixture Straße'), 'Visible desktop entry disappeared')
    for name in ('Hidden', 'NoDisplay', 'GNOME', 'NotHere', 'MissingTryExec'):
        assert not find('UFLinux Fixture ' + name), 'Desktop visibility policy ignored: ' + name
    # Application-monitor refresh while the popup stays open.
    fixture('added', 'UFLinux Fixture Added')
    wait(lambda: find('UFLinux Fixture Added'), 'New desktop file was not discovered while open')
    search('UFLinux Fixture Broken')
    wait(lambda: find('UFLinux Fixture Broken'), 'Broken launcher fixture is not listed')
    command('wtype', '-k', 'Return')
    wait(lambda: any('Could not launch application' in (name_of(a) or '') for a in tree()), 'Launch error was not shown inline')
    assert menu(), 'Failed launch dismissed the menu'
    command('wtype', '-k', 'Escape')
    wait(lambda: find('Favorites'), 'Escape did not clear search')
    # Never actually end a session in this headless build harness.
    shutdown = wait(lambda: find('Shut down', 'button') or find('Shut down', 'push button'), 'Shutdown control missing')
    home = wait(lambda: find('Home', 'button') or find('Home', 'push button'), 'Home control missing')
    def bounds(item):
        rect = item.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
        return (rect.x, rect.y, rect.width, rect.height)
    original_bounds = (bounds(home), bounds(shutdown))
    click(shutdown)
    wait(lambda: find('Cancel', 'push button') or find('Cancel', 'button'), 'Shutdown confirmation missing')
    time.sleep(.2)
    assert (bounds(home), bounds(shutdown)) == original_bounds, 'Confirmation shifted the right column'
    click(find('Cancel', 'push button') or find('Cancel', 'button'))
    wait(lambda: not find('Cancel', 'push button') and not find('Cancel', 'button'), 'Shutdown confirmation did not close')
    logout = wait(lambda: find('Log out'), 'Logout control missing')
    click(logout)
    cancel = wait(lambda: find('Cancel', 'push button') or find('Cancel', 'button'), 'Logout did not ask for confirmation')
    click(cancel)
    wait(lambda: not find('Cancel', 'push button') and not find('Cancel', 'button'), 'Cancel did not dismiss confirmation')
    assert menu(), 'Cancelling logout dismissed the menu'
    assert not Path('/tmp/uflinux-login1-actions.log').read_text(), 'Cancelling an action issued a power request'
    click(logout)
    wait(lambda: find('Cancel', 'push button') or find('Cancel', 'button'), 'Logout confirmation did not reopen')
    command('wtype', '-k', 'Escape')
    wait(lambda: not find('Cancel', 'push button') and not find('Cancel', 'button'), 'Escape did not cancel confirmation')
    command('wtype', '-k', 'Escape')
    wait(lambda: not menu(), 'Escape did not close the empty menu')
    # Consecutive Escape presses cancel confirmation and close the menu.
    open_menu()
    click(wait(lambda: find('Log out'), 'Logout control missing'))
    wait(lambda: find('Cancel', 'push button') or find('Cancel', 'button'), 'Logout confirmation missing')
    command('wtype', '-k', 'Escape', '-k', 'Escape')
    wait(lambda: not menu(), 'Rapid Escape presses did not cancel confirmation and close Start')
    # Catalog changes must also be observed while the resident menu is hidden.
    fixture('resident', 'UFLinux Fixture Resident')
    command('update-desktop-database', str(apps))
    open_menu(); search('UFLinux Fixture Resident')
    wait(lambda: find('UFLinux Fixture Resident'), 'Hidden catalog refresh was missed')
    close_menu()
    open_menu(); close_menu()
    # A typed, confirmed request is denied by the private test-bus fixture.
    open_menu()
    restart = wait(lambda: find('Restart', 'button') or find('Restart', 'push button'), 'Restart control missing')
    click(restart)
    confirms = wait(lambda: [a for a in tree() if name_of(a)=='Restart' and role_of(a) in ('button','push button') and a!=restart], 'Restart confirmation missing')
    click(confirms[-1])
    wait(lambda: any('Fixture denied power operation' in (name_of(a) or '') for a in tree()), 'Authorization denial was not shown inline')
    assert menu(), 'Denial dismissed Start'
    calls = [json.loads(line) for line in Path('/tmp/uflinux-login1-actions.log').read_text().splitlines()]
    assert calls == [{'method':'Reboot','interactive':True}], calls
    close_menu()
    # Modifier-only binds must not steal existing combinations.
    command('wtype', '-k', 'Super_L'); wait(menu, 'Left Super did not open Start')
    command('wtype', '-k', 'Super_L'); wait(lambda: not menu(), 'Left Super did not toggle Start closed')
    command('wtype', '-k', 'Super_R'); wait(menu, 'Right Super did not open Start')
    command('wtype', '-k', 'Super_R'); wait(lambda: not menu(), 'Right Super did not toggle Start closed')
    command('wtype', '-M', 'logo', '-k', 'i', '-m', 'logo')
    time.sleep(.3); assert not menu(), 'Super+I incorrectly triggered the modifier-only menu'
    open_menu()
    settings_shortcut = wait(lambda: next((item for item in tree(menu())
        if name_of(item) == 'Settings' and role_of(item) in ('button', 'push button')), None),
        'Settings shortcut is missing from Start')
    click(settings_shortcut)
    wait(lambda: not menu(), 'Settings shortcut did not dismiss Start')
    wait(lambda: find('Settings', 'frame'), 'Settings shortcut did not open Settings')
    command('uflinux-desktop', '--start-menu', '--output', 'HEADLESS-1')
    wait(menu, 'Desktop command did not open Start while Settings is open')
    command('uflinux-desktop', '--start-menu')
    wait(lambda: not menu(), 'Desktop command did not hide Start')
    command('uflinux-desktop', '--panel', 'network', '--output', 'HEADLESS-1')
    wait(lambda: find('Network', 'frame'), 'Network panel did not appear')
    pointer(100, 100)
    wait(lambda: not find('Network', 'frame'), 'Outside click did not dismiss the Network panel')
    for panel, title, control in [('network', 'Network', 'Network Settings…'), ('sound', 'Sound', 'Sound Settings…'), ('power', 'Power', 'Power Settings…')]:
        command('uflinux-desktop', '--panel', panel, '--output', 'HEADLESS-1')
        frame = wait(lambda: find(title, 'frame'), title + ' panel did not appear')
        wait(lambda: find(control), title + ' control missing')
        snapshot = json.loads(subprocess.check_output(['uflinux-settingsctl', 'status'], text=True))
        def shown(name):
            return any(name_of(a) == name and showing(a) for a in tree(frame))
        if panel == 'network':
            wifi = any(t['kind'] == 'wifi' for t in snapshot['network']['technologies']) or snapshot['network']['wifi_hardware_blocked'] or snapshot['network']['wifi_software_blocked']
            wait(lambda: shown('Wi-Fi') == wifi, 'Wi-Fi visibility did not match the hardware')
            assert shown('Scan') == wifi, 'Wi-Fi scan was shown without Wi-Fi hardware'
        elif panel == 'sound':
            wait(lambda: shown('Choose input device') == bool(snapshot['audio']['inputs']), 'Input visibility did not match the hardware')
            wait(lambda: shown('Choose output device') == bool(snapshot['audio']['outputs']), 'Output visibility did not match the hardware')
        else:
            wait(lambda: shown('Brightness') == bool(snapshot['displays']['brightness']), 'Brightness visibility did not match the hardware')
            wait(lambda: shown('Power mode') == snapshot['power']['available'], 'Power profile visibility did not match availability')
    command('uflinux-desktop', '--panel', 'power', '--output', 'HEADLESS-1')
    wait(lambda: not find('Power', 'frame'), 'Power panel did not toggle closed')
    open_menu()
    command('uflinux-desktop', '--panel', 'network', '--output', 'HEADLESS-1')
    wait(lambda: not menu(), 'Quick settings did not replace Start')
    command('uflinux-desktop', '--panel', 'network', '--output', 'HEADLESS-1')
    open_menu()
    pointer(1160, 700)
    wait(lambda: any(find(name, 'frame') for name in ('Network', 'Sound', 'Power')), 'Other taskbar controls were blocked by Start')
    assert not menu(), 'Taskbar quick-settings click left Start open'
    command('uflinux-desktop', '--start-menu', '--output', 'HEADLESS-1')
    wait(menu, 'Start did not replace panel')
    command('uflinux-desktop', '--start-menu')
    open_menu()
    command('grim', str(results / 'start-menu.png'))
    # A genuine outside click dismisses the transparent layer.
    pointer(900, 80)
    wait(lambda: not menu(), 'Clicking outside the card did not dismiss it')
    # Click the real Start button, then the same point again. The second
    # click is captured by the dismissal layer, without reopening on release.
    pointer(25, 700)
    wait(menu, 'Desktop Start button did not open the launcher')
    pointer(25, 700)
    time.sleep(.5); assert not menu(), 'Repeated Start click reopened instead of toggling closed'
    # The selected monitor disappearing must close its layer immediately.
    command('wtype', '-M', 'ctrl', '-M', 'alt', '-k', 'F10', '-m', 'alt', '-m', 'ctrl')
    wait(lambda: 'UF-TEST' in subprocess.check_output(['wlr-randr'], text=True), 'Virtual monitor was not added')
    command('uflinux-desktop', '--start-menu', '--output', 'UF-TEST')
    wait(menu, 'Start did not open on the second output')
    command('wtype', '-M', 'ctrl', '-M', 'alt', '-k', 'F11', '-m', 'alt', '-m', 'ctrl')
    wait(lambda: not menu(), 'Start survived removal of its monitor')
    command('wlr-randr', '--output', 'HEADLESS-1', '--scale', '1.25')
    open_menu(); command('grim', str(results / 'start-menu-scaled.png')); close_menu()
    command('wlr-randr', '--output', 'HEADLESS-1', '--scale', '1')
    # Exercise bounded height on a small output, then restore its mode.
    command('wlr-randr', '--output', 'HEADLESS-1', '--custom-mode', '640x480@60Hz')
    open_menu(); command('grim', str(results / 'start-menu-small.png')); close_menu()
    command('wlr-randr', '--output', 'HEADLESS-1', '--custom-mode', '1280x720@60Hz')
    (results / 'start-menu.json').write_text(json.dumps({'gio_launch': True, 'unicode_search': True, 'visibility': True,
        'catalog_refresh': True, 'inline_errors': True, 'confirmation_cancel': True, 'super_keys': True,
        'outside_click': True, 'panel_toggle': True, 'small_display': True, 'fractional_scaling': True, 'output_removal': True, 'application_browser': True}, indent=2) + '\n')
except BaseException:
    diagnostics(*sys.exc_info())
    sys.excepthook = sys.__excepthook__
    raise
finally:
    try:
        for path in created: path.unlink(missing_ok=True)
        marker.unlink(missing_ok=True)
        if menu(): close_menu()
        command('uflinux-settings', '--page', 'network')
    finally:
        keyboard.stdin.close()
        try: keyboard.wait(timeout=10)
        except subprocess.TimeoutExpired:
            keyboard.kill(); keyboard.wait(timeout=10)
print('Start menu catalog, GIO launch, search, confirmation, keyboard and pointer interactions passed')
