//! Filtered eudev events integrated with GLib; no polling thread or GTK dependency.
use glib::{ControlFlow, IOCondition, SourceId};
use std::{
    cell::Cell,
    ffi::{CStr, c_char, c_int, c_void},
    rc::Rc,
};
type Ptr = *mut c_void;
#[link(name = "udev")]
unsafe extern "C" {
    fn udev_new() -> Ptr;
    fn udev_unref(context: Ptr) -> Ptr;
    fn udev_monitor_new_from_netlink(context: Ptr, name: *const c_char) -> Ptr;
    fn udev_monitor_unref(monitor: Ptr) -> Ptr;
    fn udev_monitor_filter_add_match_subsystem_devtype(
        monitor: Ptr,
        subsystem: *const c_char,
        devtype: *const c_char,
    ) -> c_int;
    fn udev_monitor_enable_receiving(monitor: Ptr) -> c_int;
    fn udev_monitor_get_fd(monitor: Ptr) -> c_int;
    fn udev_monitor_receive_device(monitor: Ptr) -> Ptr;
    fn udev_device_get_subsystem(device: Ptr) -> *const c_char;
    fn udev_device_unref(device: Ptr) -> Ptr;
}
#[derive(Default, Clone, Copy)]
pub struct Change {
    pub backlight: bool,
    pub drm: bool,
    pub power: bool,
}
struct Inner {
    context: Ptr,
    monitor: Ptr,
}
impl Drop for Inner {
    fn drop(&mut self) {
        unsafe {
            if !self.monitor.is_null() {
                udev_monitor_unref(self.monitor);
            }
            if !self.context.is_null() {
                udev_unref(self.context);
            }
        }
    }
}
pub struct Monitor {
    source: Option<SourceId>,
    active: Rc<Cell<bool>>,
}
impl Drop for Monitor {
    fn drop(&mut self) {
        if self.active.replace(false) {
            if let Some(source) = self.source.take() {
                source.remove();
            }
        }
    }
}
impl Monitor {
    pub fn new(callback: impl Fn(Change) + 'static) -> Result<Self, String> {
        let inner = Rc::new(unsafe {
            let context = udev_new();
            if context.is_null() {
                return Err("Cannot initialize device events".into());
            }
            Inner {
                context,
                monitor: udev_monitor_new_from_netlink(context, c"udev".as_ptr()),
            }
        });
        if inner.monitor.is_null() {
            return Err("Cannot subscribe to device events".into());
        }
        for subsystem in [c"backlight", c"drm", c"power_supply"] {
            if unsafe {
                udev_monitor_filter_add_match_subsystem_devtype(
                    inner.monitor,
                    subsystem.as_ptr(),
                    std::ptr::null(),
                )
            } < 0
            {
                return Err("Cannot filter device events".into());
            }
        }
        if unsafe { udev_monitor_enable_receiving(inner.monitor) } < 0 {
            return Err("Cannot enable device events".into());
        }
        let fd = unsafe { udev_monitor_get_fd(inner.monitor) };
        if fd < 0 {
            return Err("Cannot read device events".into());
        }
        let active = Rc::new(Cell::new(true));
        let alive = active.clone();
        let source = glib::source::unix_fd_add_local(
            fd,
            IOCondition::IN | IOCondition::HUP | IOCondition::ERR,
            move |_, condition| {
                if condition.intersects(IOCondition::HUP | IOCondition::ERR) {
                    alive.set(false);
                    return ControlFlow::Break;
                }
                let mut change = Change::default();
                loop {
                    unsafe {
                        let device = udev_monitor_receive_device(inner.monitor);
                        if device.is_null() {
                            break;
                        }
                        let subsystem = udev_device_get_subsystem(device);
                        if !subsystem.is_null() {
                            match CStr::from_ptr(subsystem).to_bytes() {
                                b"backlight" => change.backlight = true,
                                b"drm" => change.drm = true,
                                b"power_supply" => change.power = true,
                                _ => {}
                            }
                        }
                        udev_device_unref(device);
                    }
                }
                callback(change);
                ControlFlow::Continue
            },
        );
        Ok(Self {
            source: Some(source),
            active,
        })
    }
}

pub fn backlights(root: &std::path::Path) -> Vec<settings_core::Brightness> {
    let mut devices = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-.:".contains(&b))
            {
                return None;
            }
            let read = |file| {
                std::fs::read_to_string(entry.path().join(file))
                    .ok()?
                    .trim()
                    .parse::<u64>()
                    .ok()
                    .map(|value| value as f64)
            };
            let maximum = read("max_brightness")?;
            let value = read("actual_brightness").or_else(|| read("brightness"))?;
            if maximum <= 0. {
                return None;
            }
            Some(settings_core::Brightness {
                id: format!("backlight:{name}"),
                name,
                percent: (value / maximum * 100.).clamp(0., 100.),
                ddc: false,
            })
        })
        .collect::<Vec<_>>();
    devices.sort_by(|a, b| a.id.cmp(&b.id));
    devices
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn external_values_and_device_removal_are_read_from_actual_brightness() {
        let root = std::env::temp_dir().join(format!(
            "uf-backlight-{}-{}",
            std::process::id(),
            settings_core::files::unique()
        ));
        let panel = root.join("panel0");
        std::fs::create_dir_all(&panel).unwrap();
        std::fs::write(panel.join("max_brightness"), "100").unwrap();
        std::fs::write(panel.join("brightness"), "80").unwrap();
        std::fs::write(panel.join("actual_brightness"), "30").unwrap();
        assert_eq!(backlights(&root)[0].percent, 30.);
        std::fs::write(panel.join("actual_brightness"), "61").unwrap();
        assert_eq!(backlights(&root)[0].percent, 61.);
        std::fs::remove_file(panel.join("actual_brightness")).unwrap();
        assert_eq!(backlights(&root)[0].percent, 80.);
        std::fs::remove_dir_all(&root).unwrap();
        assert!(backlights(&root).is_empty());
    }
}
