use crate::app::{App, Surface};
use gtk::prelude::*;
use settings_core::*;
use std::{cell::Cell, rc::Rc, time::Duration};

pub fn column() -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Vertical, 8)
}
pub fn tr(text: &str) -> glib::GString {
    glib::dgettext(Some("uflinux-settings"), text)
}
pub fn description(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(&tr(text)));
    label.set_wrap(true);
    label.set_xalign(0.);
    label.add_css_class("description");
    label
}
pub fn title(text: &str) -> gtk::Label {
    let label = description(text);
    label.remove_css_class("description");
    label.add_css_class("page-title");
    label
}
pub fn section(parent: &gtk::Box, text: &str) {
    let label = description(text);
    label.add_css_class("section-title");
    parent.append(&label);
}
pub fn row(text: &str, control: &impl IsA<gtk::Widget>) -> gtk::Box {
    control
        .as_ref()
        .update_property(&[gtk::accessible::Property::Label(&tr(text))]);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.add_css_class("settings-row");
    let label = gtk::Label::new(Some(&tr(text)));
    label.set_xalign(0.);
    label.set_wrap(true);
    label.set_hexpand(true);
    row.append(&label);
    row.append(control);
    row
}
pub fn entry(parent: &gtk::Box, name: &str, value: &str) -> gtk::Entry {
    let entry = gtk::Entry::builder()
        .text(value)
        .hexpand(true)
        .width_chars(18)
        .build();
    entry.update_property(&[gtk::accessible::Property::Label(name)]);
    parent.append(&row(name, &entry));
    entry
}
pub fn secret(parent: &gtk::Box, name: &str) -> gtk::Entry {
    let entry = entry(parent, name, "");
    entry.set_visibility(false);
    entry.set_input_purpose(gtk::InputPurpose::Password);
    entry
}
pub fn button(app: &Rc<App>, text: &str, action: impl Fn() -> Action + 'static) -> gtk::Button {
    let button = gtk::Button::with_label(&tr(text));
    let weak = Rc::downgrade(app);
    button.connect_clicked(move |button| {
        if let Some(app) = weak.upgrade() {
            app.execute(action(), Some(button.clone().upcast()));
        }
    });
    button
}
pub fn toggle(
    app: &Rc<App>,
    surface: Surface,
    parent: &gtk::Box,
    name: &str,
    get: impl Fn(&Snapshot) -> bool + 'static,
    action: impl Fn(bool) -> Action + 'static,
) {
    let switch = gtk::Switch::new();
    switch.set_valign(gtk::Align::Center);
    switch.update_property(&[gtk::accessible::Property::Label(name)]);
    let guard = Rc::new(Cell::new(false));
    let weak = Rc::downgrade(app);
    let flag = guard.clone();
    switch.connect_state_set(move |_, value| {
        if !flag.get() {
            if let Some(app) = weak.upgrade() {
                app.execute(action(value), None);
            }
        }
        glib::Propagation::Proceed
    });
    let target = switch.downgrade();
    app.bind_for(surface, &switch, move |s| {
        if let Some(widget) = target.upgrade() {
            guard.set(true);
            widget.set_active(get(s));
            guard.set(false);
        }
    });
    parent.append(&row(name, &switch));
}
pub fn slider(
    app: &Rc<App>,
    surface: Surface,
    parent: &gtk::Box,
    name: &str,
    get: impl Fn(&Snapshot) -> f64 + 'static,
    action: impl Fn(f64) -> Action + 'static,
) {
    let scale = slider_control(app, surface, name, get, action);
    parent.append(&row(name, &scale));
}

pub fn slider_control(
    app: &Rc<App>,
    surface: Surface,
    name: &str,
    get: impl Fn(&Snapshot) -> f64 + 'static,
    action: impl Fn(f64) -> Action + 'static,
) -> gtk::Scale {
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0., 100., 1.);
    scale.set_hexpand(true);
    scale.set_width_request(150);
    scale.set_draw_value(true);
    scale.set_digits(0);
    scale.update_property(&[gtk::accessible::Property::Label(name)]);
    let guard = Rc::new(Cell::new(false));
    let serial = Rc::new(Cell::new(0u64));
    let weak = Rc::downgrade(app);
    let flag = guard.clone();
    let action = Rc::new(action);
    scale.connect_value_changed(move |scale| {
        if flag.get() {
            return;
        }
        let next = serial.get() + 1;
        serial.set(next);
        let serial = serial.clone();
        let weak = weak.clone();
        let value = scale.value();
        let action = action.clone();
        glib::timeout_add_local_once(Duration::from_millis(180), move || {
            if serial.get() == next {
                if let Some(app) = weak.upgrade() {
                    app.execute(action(value), None);
                }
            }
        });
    });
    let target = scale.downgrade();
    app.bind_for(surface, &scale, move |state| {
        if let Some(widget) = target.upgrade() {
            if !widget.has_focus() && !widget.state_flags().contains(gtk::StateFlags::ACTIVE) {
                guard.set(true);
                widget.set_value(get(state));
                guard.set(false);
            }
        }
    });
    scale
}
pub fn choose(
    parent: &gtk::Box,
    label: &str,
    values: &[(&str, &str)],
    selected: &str,
) -> gtk::DropDown {
    let texts = values.iter().map(|(_, label)| *label).collect::<Vec<_>>();
    let drop = gtk::DropDown::from_strings(&texts);
    drop.set_selected(
        values
            .iter()
            .position(|(id, _)| *id == selected)
            .unwrap_or(0) as u32,
    );
    parent.append(&row(label, &drop));
    drop
}
pub fn clear(parent: &gtk::Box) {
    while let Some(child) = parent.first_child() {
        parent.remove(&child);
    }
}
