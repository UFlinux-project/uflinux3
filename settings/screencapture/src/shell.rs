//! Layer-shell plumbing shared by the capture overlay, the area selection
//! surface, the shutter flash and the recording bar. The tool has no
//! `GtkApplication`, so every surface is a plain `GtkWindow` that anchors
//! itself to an output.

use gtk::prelude::*;
use gtk::{gdk, glib};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

pub use gtk4_layer_shell::KeyboardMode as Keyboard;

pub const OVERLAY_NAMESPACE: &str = "uflinux-screencapture";

/// The card style, matching the popups in UFLinux Settings.
pub const CSS: &str = ".uflinux-capture-card { background: @theme_bg_color; color: @theme_fg_color; border: 1px solid alpha(@theme_fg_color, 0.18); border-radius: 14px; padding: 12px; box-shadow: 0 4px 16px alpha(black, 0.25); }";

/// Turn a plain window into a full-output overlay surface.
pub fn popup(window: &gtk::Window, monitor: Option<&gdk::Monitor>, keyboard: KeyboardMode) {
    anchor(window, monitor, keyboard, Layer::Overlay, true);
}

/// Turn a plain window into a strip floating above every window, used for the
/// recording bar. It is anchored to the top edge only so it keeps its height.
pub fn top_bar(window: &gtk::Window, monitor: Option<&gdk::Monitor>) {
    anchor(window, monitor, KeyboardMode::OnDemand, Layer::Top, false);
}

fn anchor(
    window: &gtk::Window,
    monitor: Option<&gdk::Monitor>,
    keyboard: KeyboardMode,
    layer: Layer,
    full_output: bool,
) {
    window.set_resizable(false);
    window.add_css_class("osd");
    window.init_layer_shell();
    window.set_namespace(Some(OVERLAY_NAMESPACE));
    window.set_layer(layer);
    window.set_exclusive_zone(-1);
    window.set_anchor(Edge::Left, full_output);
    window.set_anchor(Edge::Right, full_output);
    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Bottom, full_output);
    window.set_keyboard_mode(keyboard);

    let monitor = monitor.cloned().or_else(primary_monitor);
    if let Some(monitor) = monitor {
        window.set_monitor(Some(&monitor));
        // Hot-unplugging the output this surface belongs to ends the capture.
        let weak = window.downgrade();
        monitor.connect_invalidate(move |_| {
            if let Some(window) = weak.upgrade() {
                window.close();
            }
        });
    }
}

/// Close a surface.
pub fn dismiss(window: &gtk::Window) {
    window.close();
}

/// Install our CSS on the display.
pub fn add_css() {
    let Some(display) = gdk::Display::default() else {
        return;
    };
    let provider = gtk::CssProvider::new();
    provider.load_from_string(CSS);
    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

/// Cancel with Escape.
pub fn add_escape_key(window: &gtk::Window, on_escape: impl Fn() + 'static) {
    let controller = gtk::EventControllerKey::new();
    controller.connect_key_pressed(move |_, key, _, _| {
        if key == gdk::Key::Escape {
            on_escape();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    window.add_controller(controller);
}

/// The monitor the pointer is over, falling back to the first one.
pub fn pointed_monitor() -> Option<gdk::Monitor> {
    let display = gdk::Display::default()?;
    let pointer = display.default_seat().and_then(|seat| seat.pointer())?;
    if let Some(surface) = pointer.surface_at_position().0 {
        if let Some(monitor) = display.monitor_at_surface(&surface) {
            return Some(monitor);
        }
    }
    primary_monitor()
}

/// The first output, which the compositor treats as the primary one.
pub fn primary_monitor() -> Option<gdk::Monitor> {
    let display = gdk::Display::default()?;
    let monitors = display.monitors();
    for index in 0..monitors.n_items() {
        let Some(monitor) = monitors.item(index).and_downcast::<gdk::Monitor>() else {
            continue;
        };
        if monitor.is_valid() {
            return Some(monitor);
        }
    }
    None
}

/// The connector name of a monitor, which is what the backend takes as `-w`.
pub fn monitor_name(monitor: &gdk::Monitor) -> String {
    monitor.connector().map(String::from).unwrap_or_default()
}

/// The output to capture when the user did not pick one: the one under the
/// pointer, or the backend's own "first monitor" alias as a last resort.
pub fn fallback_monitor_name() -> String {
    let name = pointed_monitor().as_ref().map(monitor_name);
    match name {
        Some(name) if !name.is_empty() => name,
        _ => "screen".to_string(),
    }
}
