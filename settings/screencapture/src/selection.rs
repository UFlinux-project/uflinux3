//! Rubber-band selection of a screen area.
//!
//! The rectangle is tracked in logical pixels, the same space the backend uses
//! to choose the monitor and to capture a region, so the selection is handed
//! over unchanged.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;

use crate::backend::Target;
use crate::shell;

/// Smaller selections than this are treated as a mis-click and cancelled.
const MIN_SIZE: f64 = 8.0;

type Done = Box<dyn FnOnce(Option<Target>)>;

struct Drag {
    start_x: f64,
    start_y: f64,
    offset_x: f64,
    offset_y: f64,
}

/// Ask the user to drag out a rectangle on `monitor`. `done` is called exactly
/// once, with `None` when the selection was cancelled.
pub fn pick(monitor: Option<&gdk::Monitor>, done: impl FnOnce(Option<Target>) + 'static) {
    let Some(monitor) = monitor.cloned().or_else(shell::primary_monitor) else {
        done(None);
        return;
    };
    let geometry = monitor.geometry();
    let name = shell::monitor_name(&monitor);
    let origin = (f64::from(geometry.x()), f64::from(geometry.y()));

    let window = gtk::Window::new();
    shell::popup(&window, Some(&monitor), shell::Keyboard::Exclusive);
    shell::add_escape_key(&window, {
        let window = window.clone();
        move || shell::dismiss(&window)
    });

    let area = gtk::DrawingArea::new();
    area.set_content_width(geometry.width());
    area.set_content_height(geometry.height());
    area.set_cursor_from_name(Some("crosshair"));

    let drag = Rc::new(RefCell::new(None::<Drag>));
    let caption = gtk::Label::builder()
        .label("Drag to select an area")
        .css_classes(["caption"])
        .build();

    let state = drag.clone();
    area.set_draw_func(move |_, cr, area_width, area_height| {
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.35);
        match rectangle(&state) {
            // Dim everything but the selection, then outline the selection.
            Some((x, y, width, height)) => {
                cr.set_fill_rule(gtk::cairo::FillRule::EvenOdd);
                cr.rectangle(0.0, 0.0, f64::from(area_width), f64::from(area_height));
                cr.new_sub_path();
                cr.rectangle(x, y, width, height);
                let _ = cr.fill_preserve();
                cr.set_fill_rule(gtk::cairo::FillRule::Winding);
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.95);
                cr.set_line_width(2.0);
                let _ = cr.stroke();
            }
            None => {
                let _ = cr.paint();
            }
        }
    });

    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&area));
    overlay.add_overlay(&caption);
    overlay.set_halign(gtk::Align::Center);
    window.set_child(Some(&overlay));

    let once: Rc<RefCell<Option<Done>>> = Rc::new(RefCell::new(Some(Box::new(done))));

    let gesture = gtk::GestureDrag::new();
    let state = drag.clone();
    let area_weak = area.downgrade();
    gesture.connect_drag_begin(move |_, x, y| {
        *state.borrow_mut() = Some(Drag {
            start_x: x,
            start_y: y,
            offset_x: 0.0,
            offset_y: 0.0,
        });
        if let Some(area) = area_weak.upgrade() {
            area.queue_draw();
        }
    });
    let state = drag.clone();
    let area_weak = area.downgrade();
    let caption_weak = caption.downgrade();
    gesture.connect_drag_update(move |_, offset_x, offset_y| {
        if let Some(drag) = state.borrow_mut().as_mut() {
            drag.offset_x = offset_x;
            drag.offset_y = offset_y;
        }
        if let (Some(area), Some(caption)) = (area_weak.upgrade(), caption_weak.upgrade()) {
            if let Some((_, _, width, height)) = rectangle(&state) {
                caption.set_text(&format!("{} × {}", width as i32, height as i32));
            }
            area.queue_draw();
        }
    });
    let state = drag;
    let once_for_end = once.clone();
    let window_for_end = window.clone();
    gesture.connect_drag_end(move |_, _, _| {
        let target = rectangle(&state).and_then(|(x, y, width, height)| {
            if width < MIN_SIZE || height < MIN_SIZE {
                return None;
            }
            Some(Target::region(
                &name,
                (origin.0 + x) as i32,
                (origin.1 + y) as i32,
                width as i32,
                height as i32,
            ))
        });
        shell::dismiss(&window_for_end);
        if let Some(done) = once_for_end.borrow_mut().take() {
            done(target);
        }
    });
    area.add_controller(gesture);

    let once_for_close = once;
    window.connect_destroy(move |_| {
        if let Some(done) = once_for_close.borrow_mut().take() {
            done(None);
        }
    });
    window.present();
}

/// The current selection in surface coordinates: `(x, y, width, height)`.
fn rectangle(drag: &RefCell<Option<Drag>>) -> Option<(f64, f64, f64, f64)> {
    let drag = drag.borrow();
    let drag = drag.as_ref()?;
    let x = drag.start_x.min(drag.start_x + drag.offset_x);
    let y = drag.start_y.min(drag.start_y + drag.offset_y);
    Some((x, y, drag.offset_x.abs(), drag.offset_y.abs()))
}
