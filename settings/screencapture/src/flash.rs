//! The shutter flash: a white surface over the captured output for a moment.
//!
//! The backend has no delay option and takes the shot as soon as it is asked,
//! so the flash is deliberately shown *after* the capture instead of before it,
//! which is what a user expects from a camera.

use std::time::Duration;

use gtk::prelude::*;
use gtk::{gdk, glib};

use crate::shell;

const FLASH_DURATION: Duration = Duration::from_millis(150);

/// Show the flash over `monitor`. Does nothing when no output geometry is known.
pub fn show(monitor: Option<&gdk::Monitor>) {
    let Some((width, height)) = geometry(monitor) else {
        return;
    };
    let window = gtk::Window::new();
    shell::popup(&window, monitor, shell::Keyboard::None);

    let area = gtk::DrawingArea::new();
    area.set_content_width(width);
    area.set_content_height(height);
    area.set_can_target(false);
    area.set_draw_func(|_, cr, _, _| {
        cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
        let _ = cr.paint();
    });
    window.set_child(Some(&area));

    let weak = window.downgrade();
    glib::timeout_add_local_once(FLASH_DURATION, move || {
        if let Some(window) = weak.upgrade() {
            shell::dismiss(&window);
        }
    });
    window.present();
}

/// The size of the output in logical pixels.
fn geometry(monitor: Option<&gdk::Monitor>) -> Option<(i32, i32)> {
    let monitor = monitor.cloned().or_else(shell::primary_monitor)?;
    let geometry = monitor.geometry();
    let (width, height) = (geometry.width(), geometry.height());
    if width > 0 && height > 0 {
        Some((width, height))
    } else {
        None
    }
}
