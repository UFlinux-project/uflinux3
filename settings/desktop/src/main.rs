mod app;
mod auth;
#[path = "../../start-menu/src/catalog.rs"]
mod catalog;
mod clock_watch;
#[path = "../../start-menu/src/menu.rs"]
mod menu;
mod panel;
#[path = "../../shared/popup.rs"]
mod popup;
mod supervisor;
mod tray;
mod tray_menu;
#[path = "../../start-menu/src/widgets.rs"]
mod widgets;
fn main() {
    // Supervision has no use for a display connection, so it leaves before GTK
    // or the taskbar integration can pull either into the process.
    if std::env::args().any(|argument| argument == "--supervise") {
        std::process::exit(supervisor::run());
    }
    let _taskbar = uflinux_desktop::taskbar::Taskbar::default();
    if !forward() {
        app::run();
    }
}

fn forward() -> bool {
    use glib::variant::ToVariant;
    let args: Vec<_> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--daemon") {
        return false;
    }
    if args.iter().any(|arg| arg == "--status") {
        let status = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)
            .and_then(|bus| {
                bus.call_sync(
                    Some("org.uflinux.Desktop"),
                    "/org/uflinux/Desktop",
                    "org.gtk.Actions",
                    "Describe",
                    Some(&("status",).to_variant()),
                    None,
                    gio::DBusCallFlags::NO_AUTO_START,
                    1000,
                    gio::Cancellable::NONE,
                )
            })
            .ok()
            .and_then(|reply| {
                reply
                    .child_value(0)
                    .child_value(2)
                    .child_value(0)
                    .as_variant()
            })
            .and_then(|state| state.str().map(str::to_string))
            .unwrap_or_else(|| "{\"ready\":false,\"popup\":null,\"output\":null}".to_string());
        println!("{status}");
        return true;
    }
    let action = if args.iter().any(|arg| arg == "--quit") {
        "quit"
    } else if args.iter().any(|arg| arg == "--hide") {
        "hide"
    } else if args.iter().any(|arg| arg == "--panel") {
        "panel"
    } else {
        "start-menu"
    };
    let output = args
        .iter()
        .position(|arg| arg == "--output")
        .and_then(|index| args.get(index + 1))
        .cloned()
        .unwrap_or_default();
    let arguments = if action == "start-menu" {
        vec![output.to_variant()]
    } else if action == "panel" {
        let Some(panel) = args
            .iter()
            .position(|arg| arg == "--panel")
            .and_then(|i| args.get(i + 1))
        else {
            eprintln!("Missing panel name");
            std::process::exit(2)
        };
        if !["network", "sound", "power"].contains(&panel.as_str()) {
            eprintln!("Unknown panel: {panel}");
            std::process::exit(2);
        }
        vec![(panel.as_str(), output.as_str()).to_variant()]
    } else {
        Vec::new()
    };
    let parameters = (
        action,
        arguments,
        std::collections::HashMap::<String, glib::Variant>::new(),
    )
        .to_variant();
    let result = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).and_then(|bus| {
        bus.call_sync(
            Some("org.uflinux.Desktop"),
            "/org/uflinux/Desktop",
            "org.gtk.Actions",
            "Activate",
            Some(&parameters),
            None,
            gio::DBusCallFlags::NO_AUTO_START,
            5000,
            gio::Cancellable::NONE,
        )
    });
    result.is_ok() || (action != "start-menu" && action != "panel")
}
