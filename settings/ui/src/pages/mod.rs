mod audio;
mod displays;
mod network;
mod power;
mod preferences;
use crate::{
    app::{App, Surface},
    widgets::*,
};
pub use displays::confirmation as display_confirmation;
use gtk::prelude::*;
pub use network::authentication as network_auth;
use settings_core::*;
use std::rc::Rc;

pub fn build(app: &Rc<App>, page: &str, parent: &gtk::Box) {
    let name = crate::app::PAGES
        .iter()
        .find(|p| p.0 == page)
        .map(|p| p.1)
        .unwrap_or("Settings");
    parent.append(&title(name));
    match page {
        "network" => network::page(app, parent),
        "audio" => audio::page(app, parent),
        "power" => power::page(app, parent),
        "displays" => displays::page(app, parent),
        "default-apps" => preferences::defaults(app, parent),
        "datetime" => preferences::datetime(app, parent),
        "appearance" => preferences::appearance(app, parent),
        "about" => preferences::about(app, parent),
        _ => {}
    }
}
pub fn quick(app: &Rc<App>, parent: &gtk::Box) {
    parent.append(&title("Quick settings"));
    toggle(
        app,
        Surface::Quick,
        parent,
        "Networking",
        |s| !s.network.offline,
        |enabled| Action::Networking { enabled },
    );
    toggle(
        app,
        Surface::Quick,
        parent,
        "Wi-Fi",
        |s| {
            s.network
                .technologies
                .iter()
                .any(|t| t.kind == "wifi" && t.powered)
        },
        |enabled| Action::Wifi { enabled },
    );
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    actions.append(&button(app, "Scan", || Action::Scan));
    let settings = gtk::Button::with_label("All settings");
    let weak = Rc::downgrade(app);
    settings.connect_clicked(move |_| {
        if let Some(app) = weak.upgrade() {
            app.open_page("network");
        }
    });
    actions.append(&settings);
    parent.append(&actions);
    network::list(app, parent, Surface::Quick, true);
    let auth = column();
    parent.append(&auth);
    app.quick_auth.replace(Some(auth));
    audio::basic(app, parent, Surface::Quick);
    displays::brightness(app, parent, Surface::Quick);
    let links = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    for (id, label) in [("displays", "Displays"), ("audio", "Sound")] {
        let b = gtk::Button::with_label(label);
        let weak = Rc::downgrade(app);
        b.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.open_page(id);
            }
        });
        links.append(&b);
    }
    parent.append(&links);
}
pub fn dialog(app: &Rc<App>, name: &str) -> (gtk::ApplicationWindow, gtk::Box) {
    let window = gtk::ApplicationWindow::builder()
        .application(&app.application)
        .title(name)
        .default_width(560)
        .default_height(460)
        .modal(true)
        .build();
    window.add_css_class("uflinux-settings");
    if let Some(parent) = app.parent() {
        window.set_transient_for(Some(&parent));
    }
    let content = column();
    content.set_margin_start(20);
    content.set_margin_end(20);
    content.set_margin_top(20);
    content.set_margin_bottom(20);
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&content)
        .build();
    window.set_child(Some(&scroller));
    (window, content)
}
