//! Shared monitor selection and visual treatment for desktop popup cards.
use gtk::prelude::*;
use gtk4_layer_shell::LayerShell;

pub const CSS: &str = ".uflinux-popup-card { background: @theme_bg_color; color: @theme_fg_color; border: 1px solid alpha(@theme_fg_color, 0.18); border-radius: 14px; padding: 12px; box-shadow: 0 4px 16px alpha(black, 0.25); }";
pub const PANEL_HEIGHT: i32 = 40;

pub fn select_monitor(window: &gtk::ApplicationWindow, output: &str) -> Option<gtk::gdk::Monitor> {
    let display = gtk::gdk::Display::default()?;
    let monitors = display.monitors();
    let mut first = None;
    for index in 0..monitors.n_items() {
        if let Some(monitor) = monitors.item(index).and_downcast::<gtk::gdk::Monitor>() {
            if first.is_none() {
                first = Some(monitor.clone());
            }
            if !output.is_empty() && monitor.connector().as_deref() == Some(output) {
                if gtk4_layer_shell::is_supported() {
                    window.set_monitor(Some(&monitor));
                }
                return Some(monitor);
            }
        }
    }
    first
}

/// Leave the bar interactive while capturing clicks outside a popup card.
pub fn set_input_region(surface: &gtk::gdk::Surface, width: i32, height: i32, start_width: i32) {
    let region = gtk::cairo::Region::create_rectangle(&gtk::cairo::RectangleInt::new(
        0,
        0,
        width,
        (height - PANEL_HEIGHT).max(0),
    ));
    if start_width > 0 {
        let _ = region.union_rectangle(&gtk::cairo::RectangleInt::new(
            0,
            (height - PANEL_HEIGHT).max(0),
            start_width.min(width),
            PANEL_HEIGHT.min(height),
        ));
    }
    surface.set_input_region(&region);
}
