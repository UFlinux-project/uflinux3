use super::*;
use std::cell::{Cell, RefCell};

pub fn page(app: &Rc<App>, parent: &gtk::Box) {
    parent.append(&description("Arrange displays by dragging, or edit their positions below. Changes revert automatically unless confirmed within 15 seconds."));
    let draft = Rc::new(RefCell::new(Vec::<OutputConfig>::new()));
    let dirty = Rc::new(Cell::new(false));
    type PositionControls = std::collections::BTreeMap<
        String,
        (
            glib::WeakRef<gtk::SpinButton>,
            glib::WeakRef<gtk::SpinButton>,
        ),
    >;
    let positions = Rc::new(RefCell::new(PositionControls::new()));
    let syncing = Rc::new(Cell::new(false));
    let canvas = gtk::DrawingArea::builder()
        .content_height(190)
        .hexpand(true)
        .build();
    let preview = gtk::Frame::builder().child(&canvas).build();
    preview.update_property(&[gtk::accessible::Property::Label("Display layout")]);
    parent.append(&preview);
    let rectangles = Rc::new(RefCell::new(Vec::<(String, f64, f64, f64, f64, f64)>::new()));
    let boxes = rectangles.clone();
    let configurations = draft.clone();
    canvas.set_draw_func(move |_, cr, width, height| {
        let configs = configurations.borrow();
        let enabled = configs.iter().filter(|c| c.enabled).collect::<Vec<_>>();
        boxes.borrow_mut().clear();
        if enabled.is_empty() {
            return;
        }
        let extent = |c: &OutputConfig| {
            let m = c.mode.as_ref();
            let mut w = m.map(|m| m.width as f64 / c.scale).unwrap_or(1024.);
            let mut h = m.map(|m| m.height as f64 / c.scale).unwrap_or(768.);
            if c.transform % 2 == 1 {
                std::mem::swap(&mut w, &mut h);
            }
            (w, h)
        };
        let left = enabled
            .iter()
            .map(|c| c.x as f64)
            .fold(f64::INFINITY, f64::min);
        let top = enabled
            .iter()
            .map(|c| c.y as f64)
            .fold(f64::INFINITY, f64::min);
        let right = enabled
            .iter()
            .map(|c| c.x as f64 + extent(c).0)
            .fold(f64::NEG_INFINITY, f64::max);
        let bottom = enabled
            .iter()
            .map(|c| c.y as f64 + extent(c).1)
            .fold(f64::NEG_INFINITY, f64::max);
        let scale = ((width as f64 - 24.) / (right - left).max(1.))
            .min((height as f64 - 24.) / (bottom - top).max(1.));
        for c in enabled {
            let (w, h) = extent(c);
            let x = 12. + (c.x as f64 - left) * scale;
            let y = 12. + (c.y as f64 - top) * scale;
            cr.set_source_rgb(0.10, 0.32, 0.55);
            cr.rectangle(x, y, w * scale, h * scale);
            let _ = cr.fill_preserve();
            cr.set_source_rgb(0.35, 0.70, 1.);
            cr.set_line_width(2.);
            let _ = cr.stroke();
            cr.set_source_rgb(1., 1., 1.);
            cr.move_to(x + 8., y + 20.);
            let _ = cr.show_text(&c.name);
            boxes
                .borrow_mut()
                .push((c.name.clone(), x, y, w * scale, h * scale, scale));
        }
    });
    let drag = gtk::GestureDrag::new();
    let picked = Rc::new(RefCell::new(None::<(String, i32, i32, f64)>));
    let active = picked.clone();
    let boxes = rectangles.clone();
    let configs = draft.clone();
    drag.connect_drag_begin(move |_, x, y| {
        active.replace(None);
        let selected = boxes
            .borrow()
            .iter()
            .find(|(_, rx, ry, w, h, _)| x >= *rx && x <= rx + w && y >= *ry && y <= ry + h)
            .cloned();
        if let Some((name, _, _, _, _, factor)) = selected {
            if let Some(c) = configs.borrow().iter().find(|c| c.name == name) {
                active.replace(Some((name, c.x, c.y, factor)));
            }
        }
    });
    let active = picked.clone();
    let configs = draft.clone();
    let changed = dirty.clone();
    let target = canvas.downgrade();
    let fields = positions.clone();
    let guard = syncing.clone();
    drag.connect_drag_update(move |_, dx, dy| {
        if let Some((name, x, y, factor)) = &*active.borrow() {
            let updated = {
                let mut draft = configs.borrow_mut();
                draft.iter_mut().find(|c| &c.name == name).map(|c| {
                    c.x = (*x as f64 + dx / factor).round().clamp(-32768., 32768.) as i32;
                    c.y = (*y as f64 + dy / factor).round().clamp(-32768., 32768.) as i32;
                    changed.set(true);
                    (c.x, c.y)
                })
            };
            if let (Some((x, y)), Some((horizontal, vertical))) =
                (updated, fields.borrow().get(name))
            {
                guard.set(true);
                if let Some(control) = horizontal.upgrade() {
                    control.set_value(x as f64);
                }
                if let Some(control) = vertical.upgrade() {
                    control.set_value(y as f64);
                }
                guard.set(false);
            }
            if let Some(canvas) = target.upgrade() {
                canvas.queue_draw();
            }
        }
    });
    canvas.add_controller(drag);
    let group = column();
    parent.append(&group);
    let target = group.downgrade();
    let weak = Rc::downgrade(app);
    let configurations = draft.clone();
    let changed = dirty.clone();
    let drawing = canvas.downgrade();
    let signature = RefCell::new(String::new());
    let controls = positions.clone();
    let controls_sync = syncing.clone();
    app.bind_for(Surface::Full, &group, move |s| {
        if !changed.get() {
            configurations.replace(s.displays.outputs.iter().map(Output::config).collect());
        }
        controls_sync.set(true);
        for config in configurations.borrow().iter() {
            if let Some((x, y)) = controls.borrow().get(&config.name) {
                if let Some(x) = x.upgrade() {
                    x.set_value(config.x as f64);
                }
                if let Some(y) = y.upgrade() {
                    y.set_value(config.y as f64);
                }
            }
        }
        controls_sync.set(false);
        if let Some(canvas) = drawing.upgrade() {
            canvas.queue_draw();
        }
        let next = s
            .displays
            .outputs
            .iter()
            .map(|o| {
                format!(
                    "{}:{}:{:?}",
                    o.name,
                    o.adaptive_sync_control,
                    o.modes
                        .iter()
                        .map(|m| (m.width, m.height, m.refresh))
                        .collect::<Vec<_>>()
                )
            })
            .collect::<String>();
        if *signature.borrow() == next {
            return;
        }
        signature.replace(next);
        let (Some(_app), Some(group)) = (weak.upgrade(), target.upgrade()) else {
            return;
        };
        clear(&group);
        controls.borrow_mut().clear();
        if !s.displays.available {
            group.append(&description(
                "Output configuration is unavailable in this session.",
            ));
            return;
        }
        configurations.replace(s.displays.outputs.iter().map(Output::config).collect());
        changed.set(false);
        for output in &s.displays.outputs {
            let form = column();
            form.add_css_class("settings-group");
            section(
                &form,
                &format!("{} · {} {}", output.name, output.make, output.model),
            );
            let enabled = gtk::CheckButton::with_label("Enabled");
            enabled.set_active(output.enabled);
            form.append(&enabled);
            let modes = output
                .modes
                .iter()
                .map(|m| {
                    format!(
                        "{} × {} · {:.3} Hz{}",
                        m.width,
                        m.height,
                        m.refresh as f64 / 1000.,
                        if m.preferred { " · preferred" } else { "" }
                    )
                })
                .collect::<Vec<_>>();
            let strings = modes.iter().map(String::as_str).collect::<Vec<_>>();
            let mode = gtk::DropDown::from_strings(&strings);
            mode.set_selected(
                output
                    .current
                    .as_ref()
                    .and_then(|current| {
                        output.modes.iter().position(|m| {
                            m.width == current.width
                                && m.height == current.height
                                && m.refresh == current.refresh
                        })
                    })
                    .unwrap_or(0) as u32,
            );
            form.append(&row("Resolution and refresh rate", &mode));
            let scale = gtk::SpinButton::with_range(0.5, 4., 0.25);
            scale.set_digits(2);
            scale.set_value(output.scale);
            form.append(&row("Scale (1.25 = 125%)", &scale));
            let rotation = choose(
                &form,
                "Orientation",
                &[
                    ("0", "Normal"),
                    ("1", "90°"),
                    ("2", "180°"),
                    ("3", "270°"),
                    ("4", "Flipped"),
                    ("5", "Flipped 90°"),
                    ("6", "Flipped 180°"),
                    ("7", "Flipped 270°"),
                ],
                &output.transform.to_string(),
            );
            let x = gtk::SpinButton::with_range(-32768., 32768., 10.);
            x.set_value(output.x as f64);
            let y = gtk::SpinButton::with_range(-32768., 32768., 10.);
            y.set_value(output.y as f64);
            form.append(&row("Horizontal position", &x));
            form.append(&row("Vertical position", &y));
            x.update_property(&[gtk::accessible::Property::Label(&format!(
                "Horizontal position · {}",
                output.name
            ))]);
            y.update_property(&[gtk::accessible::Property::Label(&format!(
                "Vertical position · {}",
                output.name
            ))]);
            controls
                .borrow_mut()
                .insert(output.name.clone(), (x.downgrade(), y.downgrade()));
            let vrr = gtk::CheckButton::with_label("Request variable refresh rate (VRR)");
            vrr.set_active(output.adaptive_sync);
            vrr.set_sensitive(output.adaptive_sync_control);
            form.append(&vrr);
            form.append(&description(if output.adaptive_sync {
                "VRR is currently enabled."
            } else {
                "VRR is currently off; support depends on the display, driver and compositor."
            }));
            let name = output.name.clone();
            let state = configurations.clone();
            let dirty = changed.clone();
            let draw = drawing.clone();
            enabled.connect_toggled(move |b| {
                if let Some(c) = state.borrow_mut().iter_mut().find(|c| c.name == name) {
                    c.enabled = b.is_active();
                    dirty.set(true);
                }
                if let Some(draw) = draw.upgrade() {
                    draw.queue_draw();
                }
            });
            let name = output.name.clone();
            let state = configurations.clone();
            let dirty = changed.clone();
            let available = output.modes.clone();
            let draw = drawing.clone();
            mode.connect_selected_notify(move |d| {
                if let (Some(c), Some(m)) = (
                    state.borrow_mut().iter_mut().find(|c| c.name == name),
                    available.get(d.selected() as usize),
                ) {
                    c.mode = Some(m.clone());
                    dirty.set(true);
                }
                if let Some(draw) = draw.upgrade() {
                    draw.queue_draw();
                }
            });
            let name = output.name.clone();
            let state = configurations.clone();
            let dirty = changed.clone();
            let draw = drawing.clone();
            scale.connect_value_changed(move |b| {
                if let Some(c) = state.borrow_mut().iter_mut().find(|c| c.name == name) {
                    c.scale = b.value();
                    dirty.set(true);
                }
                if let Some(draw) = draw.upgrade() {
                    draw.queue_draw();
                }
            });
            let name = output.name.clone();
            let state = configurations.clone();
            let dirty = changed.clone();
            let draw = drawing.clone();
            rotation.connect_selected_notify(move |b| {
                if let Some(c) = state.borrow_mut().iter_mut().find(|c| c.name == name) {
                    c.transform = b.selected();
                    dirty.set(true);
                }
                if let Some(draw) = draw.upgrade() {
                    draw.queue_draw();
                }
            });
            for (control, horizontal) in [(x, true), (y, false)] {
                let name = output.name.clone();
                let state = configurations.clone();
                let dirty = changed.clone();
                let draw = drawing.clone();
                let guard = controls_sync.clone();
                control.connect_value_changed(move |b| {
                    if guard.get() {
                        return;
                    }
                    if let Some(c) = state.borrow_mut().iter_mut().find(|c| c.name == name) {
                        if horizontal {
                            c.x = b.value_as_int();
                        } else {
                            c.y = b.value_as_int();
                        }
                        dirty.set(true);
                    }
                    if let Some(draw) = draw.upgrade() {
                        draw.queue_draw();
                    }
                });
            }
            let name = output.name.clone();
            let state = configurations.clone();
            let dirty = changed.clone();
            vrr.connect_toggled(move |b| {
                if let Some(c) = state.borrow_mut().iter_mut().find(|c| c.name == name) {
                    c.adaptive_sync = b.is_active();
                    dirty.set(true);
                }
            });
            group.append(&form);
        }
    });
    let hdr = gtk::CheckButton::with_label("Use HDR automatically where supported (all displays)");
    hdr.set_active(app.state.borrow().displays.hdr_requested);
    parent.append(&hdr);
    parent.append(&description("HDR requires hardware Vulkan. This labwc version does not report per-monitor HDR activation or expose reliable display mirroring."));
    let configs = draft.clone();
    parent.append(&button(app, "Apply display settings", move || {
        Action::ApplyDisplays {
            outputs: configs.borrow().clone(),
            hdr: hdr.is_active(),
        }
    }));
    parent.append(&button(app, "Revert pending changes", || {
        Action::RevertDisplays
    }));
    brightness(app, parent, Surface::Full);
    app.execute(
        Action::System {
            operation: SystemAction::DdcDiscover,
        },
        None,
    );
}
pub fn brightness(app: &Rc<App>, parent: &gtk::Box, surface: Surface) {
    section(parent, "Brightness");
    let group = column();
    parent.append(&group);
    let target = group.downgrade();
    let weak = Rc::downgrade(app);
    let signature = RefCell::new(String::new());
    app.bind_for(surface, &group, move |s| {
        let next = s
            .displays
            .brightness
            .iter()
            .map(|b| b.id.as_str())
            .collect::<Vec<_>>()
            .join("|");
        if *signature.borrow() == next && !next.is_empty() {
            return;
        }
        signature.replace(next);
        let (Some(app), Some(group)) = (weak.upgrade(), target.upgrade()) else {
            return;
        };
        clear(&group);
        if s.displays.brightness.is_empty() {
            group.append(&description(if s.displays.brightness_status.is_empty() {
                "No controllable backlight or DDC/CI display detected."
            } else {
                &s.displays.brightness_status
            }));
        }
        for device in &s.displays.brightness {
            let id = device.id.clone();
            let source = id.clone();
            slider(
                &app,
                surface,
                &group,
                &format!(
                    "{}{}",
                    device.name,
                    if device.ddc { " (DDC/CI)" } else { "" }
                ),
                move |s| {
                    s.displays
                        .brightness
                        .iter()
                        .find(|b| b.id == source)
                        .map(|b| b.percent)
                        .unwrap_or(0.)
                },
                move |percent| Action::Brightness {
                    id: id.clone(),
                    percent,
                },
            );
        }
    });
}
pub fn confirmation(app: &Rc<App>, id: u64) {
    let (window, parent) = dialog(app, "Keep these display settings?");
    window.set_default_height(180);
    parent.append(&description(
        "The previous configuration returns automatically in 15 seconds if you do not confirm.",
    ));
    let keep = button(app, "Keep settings", move || Action::ConfirmDisplays { id });
    keep.add_css_class("suggested-action");
    parent.append(&keep);
    parent.append(&button(app, "Revert", || Action::RevertDisplays));
    let weak = window.downgrade();
    let application = Rc::downgrade(app);
    glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
        let (Some(window), Some(app)) = (weak.upgrade(), application.upgrade()) else {
            return glib::ControlFlow::Break;
        };
        if app.state.borrow().displays.pending != Some(id) {
            window.close();
            app.open_page("displays");
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
    window.present();
}
