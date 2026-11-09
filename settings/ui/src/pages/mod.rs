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
pub fn quick(app: &Rc<App>, parent: &gtk::Box, panel: &str) {
    let (title_text, settings_page) = match panel {
        "sound" => ("Sound", "audio"),
        "power" => ("Power", "power"),
        _ => ("Network", "network"),
    };
    parent.append(&title(title_text));
    match panel {
        "sound" => {
            audio::quick(app, parent);
        }
        "power" => {
            let status = description("");
            parent.append(&status);
            let target = status.downgrade();
            app.bind_for(Surface::Quick, &status, move |state| {
                if let Some(label) = target.upgrade() {
                    let text = if state.power.batteries.is_empty() {
                        "External power".into()
                    } else {
                        state
                            .power
                            .batteries
                            .iter()
                            .map(|battery| {
                                format!(
                                    "{}: {} · {}",
                                    battery.name,
                                    battery
                                        .percent
                                        .map(|p| format!("{p}%"))
                                        .unwrap_or_else(|| "Unknown charge".into()),
                                    battery.status
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    };
                    label.set_text(&text);
                }
            });
            displays::brightness(app, parent, Surface::Quick);
            let profiles = column();
            parent.append(&profiles);
            let target = profiles.downgrade();
            app.bind_for(Surface::Quick, &profiles, move |state| {
                if let Some(profiles) = target.upgrade() {
                    profiles.set_visible(state.power.available);
                }
            });
            power::profile(app, &profiles, Surface::Quick);
        }
        _ => {
            let status = description("");
            parent.append(&status);
            let target = status.downgrade();
            app.bind_for(Surface::Quick, &status, move |state| {
                if let Some(label) = target.upgrade() {
                    label.set_text(&settings_core::panel::status(state)[0].1);
                }
            });
            let networking = column();
            parent.append(&networking);
            let target = networking.downgrade();
            app.bind_for(Surface::Quick, &networking, move |state| {
                if let Some(networking) = target.upgrade() {
                    networking.set_visible(!state.network.technologies.is_empty());
                }
            });
            toggle(
                app,
                Surface::Quick,
                &networking,
                "Networking",
                |s| !s.network.offline,
                |enabled| Action::Networking { enabled },
            );
            let wifi = column();
            parent.append(&wifi);
            let target = wifi.downgrade();
            app.bind_for(Surface::Quick, &wifi, move |state| {
                if let Some(wifi) = target.upgrade() {
                    wifi.set_visible(
                        state.network.technologies.iter().any(|t| t.kind == "wifi")
                            || state.network.wifi_hardware_blocked
                            || state.network.wifi_software_blocked,
                    );
                }
            });
            toggle(
                app,
                Surface::Quick,
                &wifi,
                "Wi-Fi",
                |s| {
                    s.network
                        .technologies
                        .iter()
                        .any(|t| t.kind == "wifi" && t.powered)
                },
                |enabled| Action::Wifi { enabled },
            );
            wifi.append(&button(app, "Scan", || Action::Scan));
            network::list(app, parent, Surface::Quick, true);
            let auth = column();
            parent.append(&auth);
            app.quick_auth.replace(Some(auth));
        }
    }
    let settings = gtk::Button::with_label(&format!("{title_text} Settings…"));
    let weak = Rc::downgrade(app);
    settings.connect_clicked(move |_| {
        if let Some(app) = weak.upgrade() {
            app.open_page(settings_page);
        }
    });
    parent.append(&settings);
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
