mod app;
// Authorization is shared UI code, initialized only for an explicit power action.
#[path = "../../ui/src/auth.rs"]
mod auth;
mod catalog;
mod menu;
mod widgets;

fn main() {
    // A toggle client talks to the resident process before initializing GTK.
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
                    Some("org.uflinux.StartMenu"),
                    "/org/uflinux/StartMenu",
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
            .unwrap_or_else(|| "ready=false visible=false".to_string());
        println!("{status}");
        return true;
    }
    let action = if args.iter().any(|arg| arg == "--quit") {
        "quit"
    } else if args.iter().any(|arg| arg == "--hide") {
        "hide"
    } else {
        "toggle"
    };
    let output = args
        .iter()
        .position(|arg| arg == "--output")
        .and_then(|index| args.get(index + 1))
        .cloned()
        .unwrap_or_default();
    let arguments = if action == "toggle" {
        vec![output.to_variant()]
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
            Some("org.uflinux.StartMenu"),
            "/org/uflinux/StartMenu",
            "org.gtk.Actions",
            "Activate",
            Some(&parameters),
            None,
            gio::DBusCallFlags::NO_AUTO_START,
            5000,
            gio::Cancellable::NONE,
        )
    });
    result.is_ok() || action != "toggle"
}
