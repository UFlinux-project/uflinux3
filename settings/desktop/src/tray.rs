//! Session StatusNotifier watcher and asynchronous item discovery.
use gio::prelude::*;
use glib::variant::ToVariant;
use gtk::{gdk, prelude::*};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    rc::Rc,
};

const PATH: &str = "/StatusNotifierWatcher";
const IFACES: [&str; 2] = [
    "org.kde.StatusNotifierWatcher",
    "org.freedesktop.StatusNotifierWatcher",
];
const ITEMS: [&str; 2] = [
    "org.kde.StatusNotifierItem",
    "org.freedesktop.StatusNotifierItem",
];
const XML: &str = r#"<node><interface name="INTERFACE">
<method name="RegisterStatusNotifierItem"><arg type="s" direction="in"/></method>
<method name="RegisterStatusNotifierHost"><arg type="s" direction="in"/></method>
<property name="RegisteredStatusNotifierItems" type="as" access="read"/>
<property name="IsStatusNotifierHostRegistered" type="b" access="read"/>
<property name="ProtocolVersion" type="i" access="read"/>
<signal name="StatusNotifierItemRegistered"><arg type="s"/></signal>
<signal name="StatusNotifierItemUnregistered"><arg type="s"/></signal>
<signal name="StatusNotifierHostRegistered"/>
</interface></node>"#;

#[derive(Clone)]
pub struct Item {
    pub key: String,
    pub service: String,
    pub path: String,
    pub owner: String,
    pub title: String,
    pub tooltip: String,
    pub icon: String,
    pub pixmap: Option<gdk::Texture>,
    pub status: String,
    pub item_is_menu: bool,
    pub menu: String,
    pub interface: String,
    pub menu_revision: u64,
}

pub struct Tray {
    bus: gio::DBusConnection,
    items: RefCell<HashMap<String, Item>>,
    pending: RefCell<HashSet<String>>,
    dirty: RefCell<HashSet<String>>,
    follow_pending: RefCell<HashMap<String, u64>>,
    follow_serial: Cell<u64>,
    watching: RefCell<HashSet<String>>,
    menu_pending: RefCell<HashSet<String>>,
    menu_dirty: RefCell<HashSet<String>>,
    scroll_pending: RefCell<HashSet<String>>,
    scroll_dirty: RefCell<HashMap<String, (i32, i32)>>,
    owners: RefCell<Vec<gio::OwnerId>>,
    registrations: RefCell<Vec<gio::RegistrationId>>,
    changed: Box<dyn Fn()>,
}
impl Tray {
    pub async fn start(changed: impl Fn() + 'static) -> Result<Rc<Self>, glib::Error> {
        let bus = gio::bus_get_future(gio::BusType::Session).await?;
        let tray = Rc::new(Self {
            bus: bus.clone(),
            items: RefCell::new(HashMap::new()),
            pending: RefCell::new(HashSet::new()),
            dirty: RefCell::new(HashSet::new()),
            follow_pending: RefCell::new(HashMap::new()),
            follow_serial: Cell::new(0),
            watching: RefCell::new(HashSet::new()),
            menu_pending: RefCell::new(HashSet::new()),
            menu_dirty: RefCell::new(HashSet::new()),
            scroll_pending: RefCell::new(HashSet::new()),
            scroll_dirty: RefCell::new(HashMap::new()),
            owners: RefCell::new(Vec::new()),
            registrations: RefCell::new(Vec::new()),
            changed: Box::new(changed),
        });
        for interface in IFACES {
            let info = gio::DBusNodeInfo::for_xml(&XML.replace("INTERFACE", interface))?
                .interfaces()[0]
                .clone();
            let weak = Rc::downgrade(&tray);
            let property_weak = weak.clone();
            let registration = bus
                .register_object(PATH, &info)
                .method_call(move |_, sender, _, _, method, params, invocation| {
                    let Some(tray) = weak.upgrade() else {
                        invocation
                            .return_dbus_error("org.freedesktop.DBus.Error.Failed", "Tray stopped");
                        return;
                    };
                    match method {
                        "RegisterStatusNotifierItem" => {
                            if let (Some(sender), Some((service,))) =
                                (sender, params.get::<(String,)>())
                            {
                                match tray.register(sender, &service) {
                                    Ok(()) => invocation.return_value(None),
                                    Err(error) => invocation.return_dbus_error(
                                        "org.freedesktop.DBus.Error.InvalidArgs",
                                        &error,
                                    ),
                                }
                            } else {
                                invocation.return_dbus_error(
                                    "org.freedesktop.DBus.Error.InvalidArgs",
                                    "Missing item",
                                );
                            }
                        }
                        "RegisterStatusNotifierHost" => invocation.return_value(None),
                        _ => invocation.return_dbus_error(
                            "org.freedesktop.DBus.Error.UnknownMethod",
                            "Unknown method",
                        ),
                    }
                })
                .property(move |_, _, _, _, property| match property {
                    "RegisteredStatusNotifierItems" => property_weak
                        .upgrade()
                        .map(|t| t.keys())
                        .unwrap_or_default()
                        .to_variant(),
                    "IsStatusNotifierHostRegistered" => true.to_variant(),
                    "ProtocolVersion" => 0i32.to_variant(),
                    _ => ().to_variant(),
                })
                .build()?;
            tray.registrations.borrow_mut().push(registration);
            let weak = Rc::downgrade(&tray);
            let owner = gio::bus_own_name_on_connection(
                &bus,
                interface,
                gio::BusNameOwnerFlags::NONE,
                |_, _| {},
                move |_, name| {
                    if let Some(tray) = weak.upgrade() {
                        tray.follow_watcher(name);
                    }
                },
            );
            tray.owners.borrow_mut().push(owner);
        }
        let weak = Rc::downgrade(&tray);
        bus.signal_subscribe(
            Some("org.freedesktop.DBus"),
            Some("org.freedesktop.DBus"),
            Some("NameOwnerChanged"),
            Some("/org/freedesktop/DBus"),
            None,
            gio::DBusSignalFlags::NONE,
            move |_, _, _, _, _, params| {
                let Some((name, old, _new)) = params.get::<(String, String, String)>() else {
                    return;
                };
                if old.is_empty() {
                    return;
                }
                if let Some(tray) = weak.upgrade() {
                    tray.remove_owner(&name);
                }
            },
        );
        for interface in ITEMS {
            let weak = Rc::downgrade(&tray);
            bus.signal_subscribe(
                None,
                Some(interface),
                None,
                None,
                None,
                gio::DBusSignalFlags::NONE,
                move |_, sender, path, _, _, _| {
                    if let Some(tray) = weak.upgrade() {
                        let keys: Vec<_> = tray
                            .items
                            .borrow()
                            .values()
                            .filter(|item| item.owner == sender && item.path == path)
                            .map(|item| item.key.clone())
                            .collect();
                        for key in keys {
                            tray.refresh(&key);
                        }
                    }
                },
            );
        }
        let weak = Rc::downgrade(&tray);
        bus.signal_subscribe(
            None,
            Some("com.canonical.dbusmenu"),
            None,
            None,
            None,
            gio::DBusSignalFlags::NONE,
            move |_, sender, path, _, signal, _| {
                if signal != "LayoutUpdated" && signal != "ItemsPropertiesUpdated" {
                    return;
                }
                if let Some(tray) = weak.upgrade() {
                    let mut changed = false;
                    for item in tray.items.borrow_mut().values_mut() {
                        if item.owner == sender && item.menu == path {
                            item.menu_revision = item.menu_revision.wrapping_add(1);
                            changed = true;
                        }
                    }
                    if changed {
                        (tray.changed)();
                    }
                }
            },
        );
        Ok(tray)
    }
    pub fn items(&self) -> Vec<Item> {
        let mut items: Vec<_> = self
            .items
            .borrow()
            .values()
            .filter(|i| i.status != "Passive")
            .cloned()
            .collect();
        items.sort_by(|a, b| a.key.cmp(&b.key));
        items
    }
    fn keys(&self) -> Vec<String> {
        let mut keys: Vec<_> = self.items.borrow().keys().cloned().collect();
        keys.sort();
        keys
    }
    fn register(self: &Rc<Self>, sender: &str, requested: &str) -> Result<(), String> {
        let (service, path) = if requested.starts_with('/') {
            (sender, requested)
        } else {
            (requested, "/StatusNotifierItem")
        };
        self.insert_item(sender, service, path)
    }
    fn insert_item(self: &Rc<Self>, sender: &str, service: &str, path: &str) -> Result<(), String> {
        if !gio::dbus_is_name(service) || !glib::Variant::is_object_path(path) {
            return Err("Invalid item service or path".into());
        }
        let key = format!("{service}{path}");
        if self.items.borrow().contains_key(&key) {
            self.refresh(&key);
            return Ok(());
        }
        // Bound pixmap storage to 16 MiB even if every item supplies the maximum.
        if self.items.borrow().len() >= 256 {
            return Err("Too many tray items".into());
        }
        self.items.borrow_mut().insert(
            key.clone(),
            Item {
                key: key.clone(),
                service: service.into(),
                path: path.into(),
                owner: sender.into(),
                title: service.into(),
                tooltip: service.into(),
                icon: "application-x-executable".into(),
                pixmap: None,
                status: "Active".into(),
                item_is_menu: false,
                menu: String::new(),
                interface: ITEMS[0].into(),
                menu_revision: 0,
            },
        );
        self.emit("StatusNotifierItemRegistered", &key);
        (self.changed)();
        self.refresh(&key);
        Ok(())
    }
    fn follow_watcher(self: &Rc<Self>, name: &str) {
        if !self.watching.borrow_mut().insert(name.to_string()) {
            return;
        }
        let name = name.to_string();
        let weak = Rc::downgrade(self);
        self.bus.signal_subscribe(
            Some(&name),
            Some(&name),
            None,
            Some(PATH),
            None,
            gio::DBusSignalFlags::NONE,
            move |_, _, _, _, signal, params| {
                let Some((key,)) = params.get::<(String,)>() else {
                    return;
                };
                if let Some(tray) = weak.upgrade() {
                    match signal {
                        "StatusNotifierItemRegistered" => tray.follow_item(&key),
                        "StatusNotifierItemUnregistered" => tray.remove_item(&key),
                        _ => {}
                    }
                }
            },
        );
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let Some(tray) = weak.upgrade() else { return };
            if let Some(host) = tray.bus.unique_name() {
                let _ = tray
                    .bus
                    .call_future(
                        Some(&name),
                        PATH,
                        &name,
                        "RegisterStatusNotifierHost",
                        Some(&(host.as_str(),).to_variant()),
                        None,
                        gio::DBusCallFlags::NO_AUTO_START,
                        1500,
                    )
                    .await;
            }
            let result = tray
                .bus
                .call_future(
                    Some(&name),
                    PATH,
                    "org.freedesktop.DBus.Properties",
                    "Get",
                    Some(&(name.as_str(), "RegisteredStatusNotifierItems").to_variant()),
                    None,
                    gio::DBusCallFlags::NO_AUTO_START,
                    1500,
                )
                .await;
            if let Ok(reply) = result {
                if let Some(items) = reply
                    .child_value(0)
                    .as_variant()
                    .and_then(|v| v.get::<Vec<String>>())
                {
                    for item in items {
                        tray.follow_item(&item);
                    }
                }
            }
        });
    }
    fn follow_item(self: &Rc<Self>, key: &str) {
        let Some(index) = key.find('/') else { return };
        let service = key[..index].to_string();
        let path = key[index..].to_string();
        if !gio::dbus_is_name(&service) || !glib::Variant::is_object_path(&path) {
            return;
        }
        if !self.follow_pending.borrow().contains_key(key)
            && self.follow_pending.borrow().len() >= 256
        {
            return;
        }
        let serial = self.follow_serial.get().wrapping_add(1);
        self.follow_serial.set(serial);
        self.follow_pending
            .borrow_mut()
            .insert(key.to_string(), serial);
        let key = key.to_string();
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let Some(tray) = weak.upgrade() else { return };
            let result = tray
                .bus
                .call_future(
                    Some("org.freedesktop.DBus"),
                    "/org/freedesktop/DBus",
                    "org.freedesktop.DBus",
                    "GetNameOwner",
                    Some(&(service.as_str(),).to_variant()),
                    None,
                    gio::DBusCallFlags::NO_AUTO_START,
                    1500,
                )
                .await;
            if tray.follow_pending.borrow().get(&key) != Some(&serial) {
                return;
            }
            tray.follow_pending.borrow_mut().remove(&key);
            if let Ok(reply) = result {
                if let Some((owner,)) = reply.get::<(String,)>() {
                    let _ = tray.insert_item(&owner, &service, &path);
                }
            }
        });
    }
    fn remove_item(&self, key: &str) {
        self.follow_pending.borrow_mut().remove(key);
        self.dirty.borrow_mut().remove(key);
        self.menu_dirty.borrow_mut().remove(key);
        self.scroll_dirty.borrow_mut().remove(key);
        if self.items.borrow_mut().remove(key).is_some() {
            self.emit("StatusNotifierItemUnregistered", key);
            (self.changed)();
        }
    }
    fn emit(&self, signal: &str, key: &str) {
        for interface in IFACES {
            let _ = self
                .bus
                .emit_signal(None, PATH, interface, signal, Some(&(key,).to_variant()));
        }
    }
    fn remove_owner(&self, owner: &str) {
        self.follow_pending.borrow_mut().retain(|key, _| {
            key.split_once('/')
                .is_none_or(|(service, _)| service != owner)
        });
        let keys: Vec<_> = self
            .items
            .borrow()
            .values()
            .filter(|item| item.owner == owner || item.service == owner)
            .map(|item| item.key.clone())
            .collect();
        if keys.is_empty() {
            return;
        }
        for key in keys {
            self.items.borrow_mut().remove(&key);
            self.dirty.borrow_mut().remove(&key);
            self.menu_dirty.borrow_mut().remove(&key);
            self.scroll_dirty.borrow_mut().remove(&key);
            self.emit("StatusNotifierItemUnregistered", &key);
        }
        (self.changed)();
    }
    fn refresh(self: &Rc<Self>, key: &str) {
        if !self.pending.borrow_mut().insert(key.to_string()) {
            self.dirty.borrow_mut().insert(key.to_string());
            return;
        }
        let Some(item) = self.items.borrow().get(key).cloned() else {
            self.pending.borrow_mut().remove(key);
            return;
        };
        let weak = Rc::downgrade(self);
        let key = key.to_string();
        glib::MainContext::default().spawn_local(async move {
            let Some(tray) = weak.upgrade() else { return };
            for interface in ITEMS {
                let result = tray
                    .bus
                    .call_future(
                        Some(&item.service),
                        &item.path,
                        "org.freedesktop.DBus.Properties",
                        "GetAll",
                        Some(&(interface,).to_variant()),
                        None,
                        gio::DBusCallFlags::NO_AUTO_START,
                        1500,
                    )
                    .await;
                if let Ok(reply) = result {
                    if let Some((props,)) = reply.get::<(HashMap<String, glib::Variant>,)>() {
                        if let Some(current) = tray
                            .items
                            .borrow_mut()
                            .get_mut(&key)
                            .filter(|current| current.owner == item.owner)
                        {
                            current.interface = interface.into();
                            let text = |name: &str| {
                                props
                                    .get(name)
                                    .and_then(|v| v.get::<String>())
                                    .unwrap_or_default()
                            };
                            current.title = text("Title");
                            current.tooltip = props
                                .get("ToolTip")
                                .filter(|value| value.is_container() && value.n_children() == 4)
                                .map(|value| {
                                    let heading =
                                        value.child_value(2).str().unwrap_or_default().to_string();
                                    let body =
                                        value.child_value(3).str().unwrap_or_default().to_string();
                                    match (heading.is_empty(), body.is_empty()) {
                                        (true, true) => current.title.clone(),
                                        (false, true) => heading,
                                        (true, false) => body,
                                        (false, false) => format!("{heading}\n{body}"),
                                    }
                                })
                                .unwrap_or_else(|| current.title.clone());
                            current.status = text("Status");
                            let icon = if current.status == "NeedsAttention" {
                                text("AttentionIconName")
                            } else {
                                text("IconName")
                            };
                            current.icon = if icon.is_empty() {
                                "application-x-executable".into()
                            } else {
                                icon
                            };
                            current.item_is_menu = props
                                .get("ItemIsMenu")
                                .and_then(|v| v.get::<bool>())
                                .unwrap_or(false);
                            current.menu = props
                                .get("Menu")
                                .and_then(|v| v.get::<glib::variant::ObjectPath>())
                                .map(|v| v.to_string())
                                .unwrap_or_default();
                            let pixmap_name = if current.status == "NeedsAttention" {
                                "AttentionIconPixmap"
                            } else {
                                "IconPixmap"
                            };
                            current.pixmap = props.get(pixmap_name).and_then(pixmap);
                        }
                        (tray.changed)();
                        break;
                    }
                }
            }
            tray.pending.borrow_mut().remove(&key);
            if tray.dirty.borrow_mut().remove(&key) {
                tray.refresh(&key);
            }
        });
    }
    pub fn show_menu(self: &Rc<Self>, item: &Item, popover: &gtk::Popover, content: &gtk::Box) {
        if item.menu.is_empty() || item.menu == "/" {
            self.activate(item, "ContextMenu", 0, 0);
            return;
        }
        if !popover.is_visible() {
            while let Some(child) = content.first_child() {
                content.remove(&child);
            }
            content.append(&gtk::Label::new(Some("Loading…")));
            popover.popup();
        }
        if !self.menu_pending.borrow_mut().insert(item.key.clone()) {
            self.menu_dirty.borrow_mut().insert(item.key.clone());
            return;
        }
        let weak = Rc::downgrade(self);
        let bus = self.bus.clone();
        let item = item.clone();
        let popover = popover.clone();
        let content = content.clone();
        glib::MainContext::default().spawn_local(async move {
            crate::tray_menu::load(bus, item.clone(), popover.clone(), content.clone()).await;
            if let Some(tray) = weak.upgrade() {
                tray.menu_pending.borrow_mut().remove(&item.key);
                let dirty = tray.menu_dirty.borrow_mut().remove(&item.key);
                if dirty && popover.is_visible() {
                    let current = tray.items.borrow().get(&item.key).cloned();
                    if let Some(current) = current {
                        tray.show_menu(&current, &popover, &content);
                    }
                }
            }
        });
    }
    pub fn scroll(self: &Rc<Self>, item: &Item, delta: i32, horizontal: bool) {
        let key = item.key.clone();
        let mut dirty = self.scroll_dirty.borrow_mut();
        let amounts = dirty.entry(key.clone()).or_default();
        let amount = if horizontal {
            &mut amounts.0
        } else {
            &mut amounts.1
        };
        *amount = amount.saturating_add(delta);
        drop(dirty);
        if !self.scroll_pending.borrow_mut().insert(key.clone()) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let Some(tray) = weak.upgrade() else { return };
            loop {
                let Some((x, y)) = tray.scroll_dirty.borrow_mut().remove(&key) else {
                    break;
                };
                let Some(item) = tray.items.borrow().get(&key).cloned() else {
                    break;
                };
                for (delta, orientation) in [(x, "horizontal"), (y, "vertical")] {
                    if delta == 0 {
                        continue;
                    }
                    let _ = tray
                        .bus
                        .call_future(
                            Some(&item.service),
                            &item.path,
                            &item.interface,
                            "Scroll",
                            Some(&(delta, orientation).to_variant()),
                            None,
                            gio::DBusCallFlags::NO_AUTO_START,
                            1500,
                        )
                        .await;
                    if !tray
                        .items
                        .borrow()
                        .get(&key)
                        .is_some_and(|current| current.owner == item.owner)
                    {
                        break;
                    }
                }
            }
            tray.scroll_pending.borrow_mut().remove(&key);
        });
    }
    pub fn activate(&self, item: &Item, method: &str, x: i32, y: i32) {
        let bus = self.bus.clone();
        let item = item.clone();
        let method = method.to_string();
        glib::MainContext::default().spawn_local(async move {
            let _ = bus
                .call_future(
                    Some(&item.service),
                    &item.path,
                    &item.interface,
                    &method,
                    Some(&(x, y).to_variant()),
                    None,
                    gio::DBusCallFlags::NO_AUTO_START,
                    1500,
                )
                .await;
        });
    }
}
impl Drop for Tray {
    fn drop(&mut self) {
        for owner in self.owners.get_mut().drain(..) {
            gio::bus_unown_name(owner);
        }
        for id in self.registrations.get_mut().drain(..) {
            let _ = self.bus.unregister_object(id);
        }
    }
}
fn pixmap(value: &glib::Variant) -> Option<gdk::Texture> {
    let values = value.get::<Vec<(i32, i32, Vec<u8>)>>()?;
    let (width, height, bytes) = values
        .into_iter()
        .filter(|(w, h, p)| {
            *w > 0 && *h > 0 && *w <= 128 && *h <= 128 && p.len() == *w as usize * *h as usize * 4
        })
        .min_by_key(|(w, h, _)| (w - 18).abs() + (h - 18).abs())?;
    let mut rgba = Vec::with_capacity(bytes.len());
    for pixel in bytes.chunks_exact(4) {
        rgba.extend_from_slice(&[pixel[1], pixel[2], pixel[3], pixel[0]]);
    }
    let bytes = glib::Bytes::from_owned(rgba);
    Some(
        gdk::MemoryTexture::new(
            width,
            height,
            gdk::MemoryFormat::R8g8b8a8,
            &bytes,
            width as usize * 4,
        )
        .upcast(),
    )
}
