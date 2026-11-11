//! Shared monitor selection and visual treatment for desktop popup cards.
use gtk::prelude::*;
use gtk4_layer_shell::LayerShell;

pub const CSS: &str = ".uflinux-popup-card { background: @theme_bg_color; color: @theme_fg_color; border: 1px solid alpha(@theme_fg_color, 0.18); border-radius: 14px; padding: 12px; box-shadow: 0 4px 16px alpha(black, 0.25); }";

/// Comfortably longer than GTK's tooltip hover timeout, which is 500 ms and
/// drops to 60 ms once tooltip browse mode is on. See
/// [release_window_surface] for why the release has to wait that long.
const SURFACE_RELEASE_DELAY: std::time::Duration = std::time::Duration::from_millis(600);

/// Release a hidden popup window's Wayland surface, renderer and full-output
/// back buffer, which unmapping alone keeps for the whole session. The window
/// is expected to be hidden already; showing it again realizes it afresh.
///
/// The unrealize is deferred rather than done inline, for two GTK 4.22 defects
/// that only surface (`gtk_native_get_surface() == NULL`) once a window is
/// unrealized while it is still alive:
///
/// * The tooltip object is per display and holds a *weak* reference to the
///   window widget, never to its surface. Any pointer motion over the window
///   arms a one-shot popup timer for up to 500 ms (`tooltip_popup_timeout`,
///   `gtktooltip.c`) and unmap does not cancel it, because `_gtk_tooltip_hide`
///   only acts on a tooltip that was actually shown. Unrealizing immediately
///   therefore leaves that timer pointing at a destroyed surface and it fires
///   from the main loop against NULL. Waiting past the timeout means the
///   pending timer runs while the surface is still up, and an unmapped window
///   receives no further motion to arm another one.
/// * Each application window's AT-SPI context carries a `notify::realized`
///   handler (`set_a11y_properties`, `gtkapplication-wayland.c`) that reads the
///   window's current surface without a NULL guard and is never disconnected.
///   Once the window is unrealized, a later context re-realize — triggered when
///   an assistive technology queries the hidden window — runs it against NULL.
///   Dropping the handler as the surface goes away covers that; GTK registers
///   it again, and re-publishes the a11y properties, on the next realize.
///
/// A window that is visible again by the time the delay expires is left as it
/// is, so re-presenting a popup inside the window cancels its release.
pub fn release_window_surface(window: &gtk::ApplicationWindow) {
    let window = window.clone();
    glib::timeout_add_local_once(SURFACE_RELEASE_DELAY, move || {
        if window.is_visible() || !window.is_realized() {
            return;
        }
        unsafe {
            // The context is only present when accessibility is active; with
            // GTK_A11Y=none there is no handler to drop and no abort to avoid.
            let context = gtk::ffi::gtk_accessible_get_at_context(window.as_ptr().cast());
            if !context.is_null() {
                let signal = glib::gobject_ffi::g_signal_lookup(
                    c"notify".as_ptr(),
                    glib::translate::IntoGlib::into_glib(glib::Type::OBJECT),
                );
                let detail = glib::ffi::g_quark_from_static_string(c"realized".as_ptr());
                glib::gobject_ffi::g_signal_handlers_disconnect_matched(
                    context.cast(),
                    glib::gobject_ffi::G_SIGNAL_MATCH_ID
                        | glib::gobject_ffi::G_SIGNAL_MATCH_DETAIL
                        | glib::gobject_ffi::G_SIGNAL_MATCH_DATA,
                    signal,
                    detail,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    window.as_ptr().cast(),
                );
                glib::gobject_ffi::g_object_unref(context.cast());
            }
        }
        gtk::prelude::WidgetExt::unrealize(&window);
    });
}
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
