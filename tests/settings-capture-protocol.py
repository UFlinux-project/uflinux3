#!/usr/bin/env python3
"""Write, validate, reject and reset screen capture preferences over D-Bus."""
import json
import os
from pathlib import Path
import time
from gi.repository import Gio, GLib

bus = Gio.bus_get_sync(Gio.BusType.SESSION)


def call(method, action=None):
    value = GLib.Variant('(s)', (json.dumps(action),)) if action else None
    return json.loads(bus.call_sync('org.uflinux.Settings1', '/org/uflinux/Settings',
                                    'org.uflinux.Settings1', method, value, None,
                                    Gio.DBusCallFlags.NO_AUTO_START, 15000, None).unpack()[0])


def wait(predicate, description):
    for _ in range(80):
        try:
            if predicate():
                return
        except GLib.Error:
            pass
        time.sleep(.1)
    raise AssertionError(description)


previous = call('GetState')['screen_capture']
stored = Path(os.environ['XDG_CONFIG_HOME']) / 'uflinux/screen-capture.json'
# Every field is listed because the section rejects unknown and missing keys.
capture = {
    'image_format': 'png',
    'screenshot_dir': '/tmp/uflinux-capture-protocol',
    'copy_to_clipboard': False,
    'flash': False,
    'delay_seconds': 3,
    'include_cursor': False,
    'target': 'window',
    'advanced_view': True,
    'quality': 'constant-bitrate',
    'bitrate_kbps': 25000,
    'fps': 30,
    'audio_source': 'output-and-input',
    'video_codec': 'h264',
    'container': 'mkv',
    'audio_codec': 'aac',
    'framerate_mode': 'content',
    'color_range': 'full',
}
try:
    call('Execute', {'action': 'set-screen-capture', 'screen_capture': capture})
    wait(lambda: call('GetState')['screen_capture'] == capture,
         'Screen capture preferences did not converge')
    assert json.loads(stored.read_text()) == capture
    assert stored.stat().st_mode & 0o777 == 0o600, 'Capture preferences are not private'

    # The probe must answer even when the backend cannot report anything, which is
    # the case on a headless machine without a usable GPU.
    capabilities = call('Execute', {'action': 'screen-capture-capabilities'})
    assert isinstance(capabilities['backend_available'], bool)
    assert all(key in capabilities for key in
               ('image_formats', 'video_codecs', 'containers'))

    for rejected in ({**capture, 'fps': 0}, {**capture, 'fps': 241},
                     {**capture, 'bitrate_kbps': 0}, {**capture, 'delay_seconds': 31},
                     {**capture, 'screenshot_dir': 'Screenshots'},
                     {**capture, 'video_codec': 'not-a-codec'},
                     {**capture, 'container': 'avi'},
                     {**capture, 'image_format': 'tiff'}):
        try:
            call('Execute', {'action': 'set-screen-capture', 'screen_capture': rejected})
            raise AssertionError('Rejected payload %r was accepted' % rejected)
        except GLib.Error:
            pass
    assert call('GetState')['screen_capture'] == capture, 'A rejected write changed the section'
    assert json.loads(stored.read_text()) == capture, 'A rejected write changed the file'

    call('Execute', {'action': 'reset-screen-capture'})
    defaults = call('GetState')['screen_capture']
    assert json.loads(stored.read_text()) == defaults
    # The shipped defaults are the backend's own recommendations.
    assert defaults['image_format'] == 'webp' and defaults['copy_to_clipboard']
    assert defaults['fps'] == 60 and defaults['quality'] == 'very-high'
    assert defaults['advanced_view'] is False and defaults['target'] == 'screen'
    print('Screen capture preference persistence, validation and reset passed')
finally:
    call('Execute', {'action': 'set-screen-capture', 'screen_capture': previous})
