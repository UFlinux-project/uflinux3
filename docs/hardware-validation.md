# Physical hardware validation

QEMU and headless tests establish software behavior. They cannot establish panel
HDR/VRR activation, I2C reliability, laptop firmware behavior or physical resume.
Run this procedure from an installed UFLinux desktop on each reference device.
Use a test installation and retain its normal monitor layout and power settings.

The [device image targets](device-images.md) have no hardware evidence yet. When
one is tested, record at least: that the board boots from the image as written
(without re-partitioning), that the root partition grew to the media on first
boot, the serial console and getty behaviour, the renderer reported by the
desktop, and whether the display works at all. The SpacemiT K1 image is expected
to have no display output with the current mainline device tree.

- **Graphics:** check the renderer in Displays and About, the actual GPU/driver,
  launch/log out normally, and test supported Vulkan/OpenGL plus the software
  fallback. Record explicit overrides. A configured renderer is not evidence of
  hardware acceleration by itself.
- **Displays:** apply exact advertised refresh modes, scaling, rotation and
  arrangement. Leave a change unconfirmed to check the 15-second rollback. Unplug
  the enabled external display after saving a disabled internal display, then
  reconnect. Confirm that a usable display recovers and the saved layout returns.
- **VRR:** request VRR on compatible hardware and verify it using the panel's
  refresh indicator or an appropriate moving test pattern. Record GPU, connection,
  mode and observation. Protocol support alone is not a physical VRR pass.
- **HDR:** use labwc's global HDR option with compatible hardware Vulkan. Record
  monitor mode and an HDR test-image observation. The app does not claim to report
  per-monitor activation; mark unavailable hardware explicitly.
- **Brightness:** test the internal backlight in both Settings and the flyout,
  then change it with a hardware key. Both views should update from device events.
  Test monitor-specific DDC/CI, disconnect during a request and reconnect. Physical
  monitor-button brightness changes need a rescan: DDC/CI supplies no push event.
- **Power:** switch all three TLP modes and reboot to check the selected default.
  Test charger transitions, then configure each physical power-button action on
  AC and battery. Check inhibitors, suspend and resume, networking/audio after
  resume, and restore the original policy. Firmware-controlled long presses are
  outside the application policy.

Record the results as JSON with these optional keys: `hardware_gpu`, `vrr`, `hdr`,
`ddc_ci`, `backlight`, `battery`, `suspend_resume`. Each value contains a `status`
(`pass`, `fail`, `unavailable`, `unverified`) and `notes`. Pass/fail notes must
identify the device, method and observation. Do not put credentials in the file.

```json
{
  "backlight": {
    "status": "unverified",
    "notes": "Replace with the laptop model, driver, tested actions and observations."
  }
}
```

Run `python3 scripts/hardware-record.py --results results.json` in that installed
session. Python is a validation dependency, not a required background service.
The recorder reads state and metadata; it does not change hardware settings or
include Wi-Fi credentials. Its report distinguishes observed configuration from
operator-supplied evidence and rejects physical passes on known QEMU/VM hardware.
Reports under `out-settings/checks/ARCH/` can accompany the other build records.

For native reference performance, constrain the installed reference machine to
512 MiB, one display and no swap, and record GPU, mode, driver, renderer and
storage. Measure PSS for the GUI, flyout and background components independently.
Record first usable frame after backend state and painting, distinguish a new GUI
process from warm activation, and state whether filesystem caches were cold.
The existing headless/VM figures are separate measurements and must not be
relabelled as native hardware certification.
