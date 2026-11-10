//! Bounded GTK representation of a com.canonical.dbusmenu layout.
use gio::prelude::*;
use glib::variant::ToVariant;
use gtk::prelude::*;
use std::collections::HashMap;

struct Node {
    id: i32,
    properties: HashMap<String, glib::Variant>,
    children: Vec<Node>,
}
fn parse(value: &glib::Variant, depth: usize, budget: &mut usize) -> Option<Node> {
    if depth > 8 || *budget == 0 || value.n_children() != 3 {
        return None;
    }
    *budget -= 1;
    let id = value.child_value(0).get::<i32>()?;
    let properties = value
        .child_value(1)
        .get::<HashMap<String, glib::Variant>>()?;
    let children = value.child_value(2);
    let mut parsed = Vec::new();
    for i in 0..children.n_children() {
        let child = children.child_value(i).as_variant()?;
        if let Some(child) = parse(&child, depth + 1, budget) {
            parsed.push(child);
        }
    }
    Some(Node {
        id,
        properties,
        children: parsed,
    })
}
fn string(node: &Node, name: &str) -> String {
    node.properties
        .get(name)
        .and_then(|v| v.get::<String>())
        .unwrap_or_default()
}
fn boolean(node: &Node, name: &str, default: bool) -> bool {
    node.properties
        .get(name)
        .and_then(|v| v.get::<bool>())
        .unwrap_or(default)
}
fn event(bus: &gio::DBusConnection, item: &crate::tray::Item, id: i32) {
    let bus = bus.clone();
    let item = item.clone();
    glib::MainContext::default().spawn_local(async move {
        let _ = bus
            .call_future(
                Some(&item.service),
                &item.menu,
                "com.canonical.dbusmenu",
                "Event",
                Some(&(id, "clicked", 0i32.to_variant(), 0u32).to_variant()),
                None,
                gio::DBusCallFlags::NO_AUTO_START,
                1500,
            )
            .await;
    });
}
fn fill(
    container: &gtk::Box,
    nodes: &[Node],
    bus: &gio::DBusConnection,
    item: &crate::tray::Item,
    root: &gtk::Popover,
) {
    let mut radio_group: Option<gtk::CheckButton> = None;
    for node in nodes {
        if !boolean(node, "visible", true) {
            continue;
        }
        if string(node, "type") == "separator" {
            container.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
            radio_group = None;
            continue;
        }
        let label = string(node, "label");
        let enabled = boolean(node, "enabled", true);
        if !node.children.is_empty() || string(node, "children-display") == "submenu" {
            let button = gtk::MenuButton::new();
            button.set_label(&label);
            button.set_sensitive(enabled);
            let popover = gtk::Popover::new();
            let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
            fill(&content, &node.children, bus, item, root);
            popover.set_child(Some(&content));
            button.set_popover(Some(&popover));
            container.append(&button);
            continue;
        }
        let toggle_type = string(node, "toggle-type");
        if toggle_type == "checkmark" || toggle_type == "radio" {
            let check = gtk::CheckButton::with_label(&label);
            check.set_sensitive(enabled);
            if toggle_type == "radio" {
                if let Some(group) = radio_group.as_ref() {
                    check.set_group(Some(group));
                } else {
                    radio_group = Some(check.clone());
                }
            }
            let state = node
                .properties
                .get("toggle-state")
                .and_then(|v| v.get::<i32>())
                .unwrap_or(0);
            check.set_active(state > 0);
            let bus = bus.clone();
            let item = item.clone();
            let id = node.id;
            let root = root.downgrade();
            check.connect_toggled(move |check| {
                if toggle_type == "radio" && !check.is_active() {
                    return;
                }
                event(&bus, &item, id);
                if let Some(root) = root.upgrade() {
                    root.popdown();
                }
            });
            container.append(&check);
        } else {
            let button = gtk::Button::with_label(&label);
            button.set_sensitive(enabled);
            let icon = string(node, "icon-name");
            if !icon.is_empty() {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                row.append(&gtk::Image::from_icon_name(&icon));
                row.append(&gtk::Label::new(Some(&label)));
                button.set_child(Some(&row));
            }
            let bus = bus.clone();
            let item = item.clone();
            let id = node.id;
            let root = root.downgrade();
            button.connect_clicked(move |_| {
                event(&bus, &item, id);
                if let Some(root) = root.upgrade() {
                    root.popdown();
                }
            });
            container.append(&button);
        }
    }
}
/// Refresh a menu asynchronously; callers may invoke this again on layout updates.
pub async fn load(
    bus: gio::DBusConnection,
    item: crate::tray::Item,
    popover: gtk::Popover,
    content: gtk::Box,
) {
    let _ = bus
        .call_future(
            Some(&item.service),
            &item.menu,
            "com.canonical.dbusmenu",
            "AboutToShow",
            Some(&(0i32,).to_variant()),
            None,
            gio::DBusCallFlags::NO_AUTO_START,
            1000,
        )
        .await;
    let properties = [
        "label",
        "enabled",
        "visible",
        "type",
        "children-display",
        "toggle-type",
        "toggle-state",
        "icon-name",
    ];
    let Ok(reply) = bus
        .call_future(
            Some(&item.service),
            &item.menu,
            "com.canonical.dbusmenu",
            "GetLayout",
            Some(&(0i32, -1i32, properties.as_slice()).to_variant()),
            None,
            gio::DBusCallFlags::NO_AUTO_START,
            1500,
        )
        .await
    else {
        return;
    };
    if !popover.is_visible() || reply.n_children() != 2 {
        return;
    }
    let Some(root) = parse(&reply.child_value(1), 0, &mut 256) else {
        return;
    };
    while let Some(child) = content.first_child() {
        content.remove(&child);
    }
    fill(&content, &root.children, &bus, &item, &popover);
}
