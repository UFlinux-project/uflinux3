//! Experimental panel surface; the installed session remains on Ironbar.
use crate::{app::App, catalog, tray::Tray};
use gtk::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::{Rc, Weak},
};
use uflinux_desktop::{
    taskbar::{Activation, Application, Taskbar, WindowId},
    toplevel::{Client, Command},
};

struct TaskButton {
    button: gtk::Button,
    image: gtk::Image,
    icon: String,
    menu: gtk::Popover,
}
impl Drop for TaskButton {
    fn drop(&mut self) {
        self.menu.unparent();
    }
}
struct TrayButton {
    button: gtk::Button,
    image: gtk::Image,
    popover: gtk::Popover,
    content: gtk::Box,
    revision: u64,
}
struct Surface {
    tray: gtk::Box,
    tray_buttons: HashMap<String, TrayButton>,
    monitor: gtk::gdk::Monitor,
    window: gtk::ApplicationWindow,
    tasks: gtk::Box,
    buttons: HashMap<String, TaskButton>,
    clock: gtk::Popover,
    clock_label: gtk::Label,
}
pub struct Panel {
    app: Weak<App>,
    surfaces: RefCell<Vec<Surface>>,
    taskbar: RefCell<Taskbar>,
    tracker: RefCell<Option<Client>>,
    tray: RefCell<Option<Rc<Tray>>>,
    window_ids: RefCell<HashSet<WindowId>>,
    clock_timer: RefCell<Option<glib::SourceId>>,
    clock_watch: RefCell<Option<crate::clock_watch::ClockWatch>>,
}
impl Panel {
    pub fn new(app: &Rc<App>) -> Rc<Self> {
        let panel = Rc::new(Self {
            app: Rc::downgrade(app),
            surfaces: RefCell::new(Vec::new()),
            taskbar: RefCell::new(Taskbar::new(vec![
                "foot.desktop".into(),
                "nemo.desktop".into(),
            ])),
            tracker: RefCell::new(None),
            tray: RefCell::new(None),
            window_ids: RefCell::new(HashSet::new()),
            clock_timer: RefCell::new(None),
            clock_watch: RefCell::new(None),
        });
        if let Some(display) = gtk::gdk::Display::default() {
            let monitors = display.monitors();
            let weak = Rc::downgrade(&panel);
            monitors.connect_items_changed(move |monitors, _, _, _| {
                if let Some(panel) = weak.upgrade() {
                    panel.sync(monitors);
                    if let Some(app) = panel.app.upgrade() {
                        app.quick.desktop_outputs_changed();
                    }
                }
            });
            panel.sync(&monitors);
        }
        if let Ok((tracker, notifications)) = Client::start() {
            panel.tracker.replace(Some(tracker));
            let weak = Rc::downgrade(&panel);
            glib::MainContext::default().spawn_local(async move {
                while notifications.recv().await.is_ok() {
                    let Some(panel) = weak.upgrade() else { break };
                    panel.update_windows();
                }
            });
        }
        let weak = Rc::downgrade(&panel);
        glib::MainContext::default().spawn_local(async move {
            let changed = weak.clone();
            match Tray::start(move || {
                if let Some(panel) = changed.upgrade() {
                    panel.render_tray();
                }
            })
            .await
            {
                Ok(tray) => {
                    if let Some(panel) = weak.upgrade() {
                        panel.tray.replace(Some(tray));
                        panel.render_tray();
                    }
                }
                Err(error) => eprintln!("Desktop tray unavailable: {error}"),
            }
        });
        panel.schedule_clock();
        if let Ok((watch, notifications)) = crate::clock_watch::ClockWatch::start() {
            panel.clock_watch.replace(Some(watch));
            let weak = Rc::downgrade(&panel);
            glib::MainContext::default().spawn_local(async move {
                while notifications.recv().await.is_ok() {
                    let Some(panel) = weak.upgrade() else { break };
                    panel.schedule_clock();
                }
            });
        }
        let weak = Rc::downgrade(&panel);
        glib::MainContext::default().spawn_local(async move {
            if let Ok(bus) = gio::bus_get_future(gio::BusType::System).await {
                bus.signal_subscribe(
                    Some("org.freedesktop.login1"),
                    Some("org.freedesktop.login1.Manager"),
                    Some("PrepareForSleep"),
                    Some("/org/freedesktop/login1"),
                    None,
                    gio::DBusSignalFlags::NONE,
                    move |_, _, _, _, _, parameters| {
                        if parameters.get::<(bool,)>() == Some((false,)) {
                            if let Some(panel) = weak.upgrade() {
                                panel.schedule_clock();
                            }
                        }
                    },
                );
            }
        });
        panel
    }
    pub fn set_catalog(self: &Rc<Self>, catalog: &catalog::Catalog) {
        let applications = catalog
            .entries
            .iter()
            .map(|entry| Application {
                desktop_id: entry.id.clone(),
                name: entry.name.clone(),
                icon: entry.icon.clone(),
                startup_wm_class: entry.startup_wm_class.clone(),
            })
            .collect();
        self.taskbar.borrow_mut().set_applications(applications);
        self.render();
    }
    fn update_clock(&self) {
        if let Ok(now) = glib::DateTime::now_local() {
            if let Ok(text) = now.format("%H:%M\n%d/%m/%Y") {
                for surface in self.surfaces.borrow().iter() {
                    surface.clock_label.set_text(&text);
                }
            }
        }
    }
    fn schedule_clock(self: &Rc<Self>) {
        if let Some(timer) = self.clock_timer.borrow_mut().take() {
            timer.remove();
        }
        self.update_clock();
        let wait = glib::DateTime::now_local()
            .map(|n| 60_000_000 - i64::from(n.second()) * 1_000_000 - i64::from(n.microsecond()))
            .unwrap_or(60_000_000)
            .max(1);
        let weak = Rc::downgrade(self);
        let timer = glib::timeout_add_local_once(
            std::time::Duration::from_micros(wait as u64),
            move || {
                if let Some(panel) = weak.upgrade() {
                    panel.clock_timer.borrow_mut().take();
                    panel.schedule_clock();
                }
            },
        );
        self.clock_timer.replace(Some(timer));
    }
    pub fn hide_clocks(&self, except: Option<&gtk::Popover>) {
        let clocks: Vec<_> = self
            .surfaces
            .borrow()
            .iter()
            .map(|s| s.clock.clone())
            .collect();
        for clock in clocks {
            if except != Some(&clock) {
                clock.popdown();
            }
        }
    }
    pub fn hide_trays(&self, except: Option<&gtk::Popover>) {
        let popovers: Vec<_> = self
            .surfaces
            .borrow()
            .iter()
            .flat_map(|s| s.tray_buttons.values().map(|item| item.popover.clone()))
            .collect();
        for popover in popovers {
            if except != Some(&popover) {
                popover.popdown();
            }
        }
    }
    pub fn hide_task_menus(&self, except: Option<&gtk::Popover>) {
        let menus: Vec<_> = self
            .surfaces
            .borrow()
            .iter()
            .flat_map(|surface| surface.buttons.values().map(|item| item.menu.clone()))
            .collect();
        for menu in menus {
            if except != Some(&menu) {
                menu.popdown();
            }
        }
    }
    fn render_tray(self: &Rc<Self>) {
        let items = self
            .tray
            .borrow()
            .as_ref()
            .map(|tray| tray.items())
            .unwrap_or_default();
        let wanted: HashSet<_> = items.iter().map(|item| item.key.as_str()).collect();
        for surface in self.surfaces.borrow_mut().iter_mut() {
            surface.tray_buttons.retain(|id, item| {
                if wanted.contains(id.as_str()) {
                    true
                } else {
                    item.popover.unparent();
                    surface.tray.remove(&item.button);
                    false
                }
            });
            let mut previous: Option<gtk::Button> = None;
            for item in &items {
                let button = surface
                    .tray_buttons
                    .entry(item.key.clone())
                    .or_insert_with(|| {
                        let button = gtk::Button::new();
                        let image = gtk::Image::new();
                        image.set_pixel_size(18);
                        button.set_child(Some(&image));
                        let popover = gtk::Popover::new();
                        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
                        popover.set_child(Some(&content));
                        popover.set_parent(&button);
                        let weak = self.app.clone();
                        let output = surface
                            .monitor
                            .connector()
                            .map(|s| s.to_string())
                            .unwrap_or_default();
                        let target = popover.downgrade();
                        popover.connect_show(move |_| {
                            if let (Some(app), Some(popover)) = (weak.upgrade(), target.upgrade()) {
                                app.show_tray(&popover, &output);
                            }
                        });
                        let weak = self.app.clone();
                        popover.connect_hide(move |_| {
                            if let Some(app) = weak.upgrade() {
                                app.tray_closed();
                            }
                        });
                        let key = item.key.clone();
                        let weak = Rc::downgrade(self);
                        let menu = popover.clone();
                        let contents = content.clone();
                        button.connect_clicked(move |_| {
                            let Some(panel) = weak.upgrade() else { return };
                            if let Some(tray) = panel.tray.borrow().as_ref() {
                                if let Some(item) =
                                    tray.items().into_iter().find(|item| item.key == key)
                                {
                                    if item.item_is_menu {
                                        tray.show_menu(&item, &menu, &contents);
                                    } else {
                                        tray.activate(&item, "Activate", 0, 0);
                                    }
                                }
                            }
                        });
                        let key = item.key.clone();
                        let weak = Rc::downgrade(self);
                        let menu = popover.clone();
                        let contents = content.clone();
                        let click = gtk::GestureClick::new();
                        click.set_button(3);
                        click.connect_pressed(move |_, _, _, _| {
                            let Some(panel) = weak.upgrade() else { return };
                            if let Some(tray) = panel.tray.borrow().as_ref() {
                                if let Some(item) =
                                    tray.items().into_iter().find(|item| item.key == key)
                                {
                                    tray.show_menu(&item, &menu, &contents);
                                }
                            }
                        });
                        button.add_controller(click);
                        let key = item.key.clone();
                        let weak = Rc::downgrade(self);
                        let scroll = gtk::EventControllerScroll::new(
                            gtk::EventControllerScrollFlags::BOTH_AXES
                                | gtk::EventControllerScrollFlags::DISCRETE,
                        );
                        scroll.connect_scroll(move |_, dx, dy| {
                            let Some(panel) = weak.upgrade() else {
                                return glib::Propagation::Proceed;
                            };
                            if let Some(tray) = panel.tray.borrow().as_ref() {
                                if let Some(item) =
                                    tray.items().into_iter().find(|item| item.key == key)
                                {
                                    let horizontal = dx.abs() > dy.abs();
                                    let delta = if horizontal { dx } else { dy };
                                    tray.scroll(&item, (delta * 120.0) as i32, horizontal);
                                }
                            }
                            glib::Propagation::Stop
                        });
                        button.add_controller(scroll);
                        surface.tray.append(&button);
                        TrayButton {
                            button,
                            image,
                            popover,
                            content,
                            revision: item.menu_revision,
                        }
                    });
                if button.revision != item.menu_revision {
                    button.revision = item.menu_revision;
                    if button.popover.is_visible() {
                        if let Some(tray) = self.tray.borrow().as_ref() {
                            tray.show_menu(item, &button.popover, &button.content);
                        }
                    }
                }
                button.button.set_tooltip_text(Some(&item.tooltip));
                button
                    .button
                    .update_property(&[gtk::accessible::Property::Label(&item.title)]);
                if let Some(texture) = &item.pixmap {
                    button.image.set_paintable(Some(texture));
                } else {
                    button.image.set_icon_name(Some(&item.icon));
                }
                surface
                    .tray
                    .reorder_child_after(&button.button, previous.as_ref());
                previous = Some(button.button.clone());
            }
        }
    }
    fn update_windows(self: &Rc<Self>) {
        let snapshot = self.tracker.borrow().as_ref().map(Client::snapshot);
        let Some(snapshot) = snapshot else { return };
        let ids: HashSet<_> = snapshot.windows.iter().map(|w| w.id).collect();
        {
            let mut model = self.taskbar.borrow_mut();
            for id in self.window_ids.borrow().difference(&ids) {
                model.remove_window(*id);
            }
            for window in snapshot.windows {
                model.upsert_window(window);
            }
        }
        self.window_ids.replace(ids);
        self.render();
    }
    fn render(self: &Rc<Self>) {
        let groups = self.taskbar.borrow().groups();
        let wanted: HashSet<_> = groups.iter().map(|group| group.id.as_str()).collect();
        for surface in self.surfaces.borrow_mut().iter_mut() {
            surface.buttons.retain(|id, item| {
                if wanted.contains(id.as_str()) {
                    true
                } else {
                    surface.tasks.remove(&item.button);
                    false
                }
            });
            let mut previous: Option<gtk::Button> = None;
            for group in &groups {
                let item = surface.buttons.entry(group.id.clone()).or_insert_with(|| {
                    let button = gtk::Button::new();
                    let menu = gtk::Popover::new();
                    menu.set_parent(&button);
                    let output = surface
                        .monitor
                        .connector()
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                    let weak = Rc::downgrade(self);
                    menu.connect_hide(move |_| {
                        if let Some(panel) = weak.upgrade() {
                            if let Some(app) = panel.app.upgrade() {
                                app.task_closed();
                            }
                        }
                    });
                    let click = gtk::GestureClick::new();
                    click.set_button(3);
                    let context_id = group.id.clone();
                    let context_menu = menu.clone();
                    let weak = Rc::downgrade(self);
                    click.connect_pressed(move |_, _, _, _| {
                        let Some(panel) = weak.upgrade() else { return };
                        let Some(group) = panel
                            .taskbar
                            .borrow()
                            .groups()
                            .into_iter()
                            .find(|g| g.id == context_id)
                        else {
                            return;
                        };
                        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
                        for window in group.windows {
                            let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
                            let title = if window.title.is_empty() {
                                &group.name
                            } else {
                                &window.title
                            };
                            let activate = gtk::Button::with_label(title);
                            activate.set_hexpand(true);
                            let weak = Rc::downgrade(&panel);
                            let popup = context_menu.clone();
                            activate.connect_clicked(move |_| {
                                if let Some(panel) = weak.upgrade() {
                                    if let Some(tracker) = panel.tracker.borrow_mut().as_mut() {
                                        let _ = tracker.send(Command::Activate(window.id));
                                    }
                                }
                                popup.popdown();
                            });
                            row.append(&activate);
                            let close = gtk::Button::from_icon_name("window-close-symbolic");
                            close.set_tooltip_text(Some("Close window"));
                            let weak = Rc::downgrade(&panel);
                            let popup = context_menu.clone();
                            close.connect_clicked(move |_| {
                                if let Some(panel) = weak.upgrade() {
                                    if let Some(tracker) = panel.tracker.borrow_mut().as_mut() {
                                        let _ = tracker.send(Command::Close(window.id));
                                    }
                                }
                                popup.popdown();
                            });
                            row.append(&close);
                            column.append(&row);
                        }
                        if column.first_child().is_none() {
                            return;
                        }
                        if let Some(app) = panel.app.upgrade() {
                            app.hide();
                            app.task_opened(&output);
                        }
                        context_menu.set_child(Some(&column));
                        context_menu.popup();
                    });
                    button.add_controller(click);
                    let image = gtk::Image::new();
                    image.set_pixel_size(24);
                    button.set_child(Some(&image));
                    let group_id = group.id.clone();
                    let weak = Rc::downgrade(self);
                    button.connect_clicked(move |_| {
                        let Some(panel) = weak.upgrade() else { return };
                        let action = panel
                            .taskbar
                            .borrow()
                            .groups()
                            .into_iter()
                            .find(|g| g.id == group_id)
                            .and_then(|g| g.activate());
                        match action {
                            Some(Activation::Launch(id)) => {
                                if let Some(info) = gio::DesktopAppInfo::new(&id) {
                                    let _ = info.launch(&[], None::<&gio::AppLaunchContext>);
                                }
                            }
                            Some(Activation::Activate(id)) => {
                                if let Some(tracker) = panel.tracker.borrow_mut().as_mut() {
                                    let _ = tracker.send(Command::Activate(id));
                                }
                            }
                            Some(Activation::Minimize(id)) => {
                                if let Some(tracker) = panel.tracker.borrow_mut().as_mut() {
                                    let _ = tracker.send(Command::Minimize(id));
                                }
                            }
                            None => {}
                        }
                    });
                    surface.tasks.append(&button);
                    TaskButton {
                        button,
                        image,
                        icon: String::new(),
                        menu,
                    }
                });
                if item.icon != group.icon {
                    if let Ok(icon) = gio::Icon::for_string(&group.icon) {
                        item.image.set_from_gicon(&icon);
                    } else {
                        item.image.set_icon_name(Some("application-x-executable"));
                    }
                    item.icon.clone_from(&group.icon);
                }
                item.button.set_tooltip_text(Some(&group.name));
                if group.windows.is_empty() {
                    item.button.remove_css_class("open");
                } else {
                    item.button.add_css_class("open");
                }
                if group.windows.iter().any(|w| w.focused) {
                    item.button.add_css_class("focused");
                } else {
                    item.button.remove_css_class("focused");
                }
                surface
                    .tasks
                    .reorder_child_after(&item.button, previous.as_ref());
                previous = Some(item.button.clone());
            }
        }
    }
    fn sync(self: &Rc<Self>, monitors: &gio::ListModel) {
        let current: Vec<_> = (0..monitors.n_items())
            .filter_map(|i| monitors.item(i)?.downcast::<gtk::gdk::Monitor>().ok())
            .collect();
        self.surfaces.borrow_mut().retain(|surface| {
            let keep = current.contains(&surface.monitor);
            if !keep {
                for item in surface.tray_buttons.values() {
                    item.popover.unparent();
                }
                surface.window.close();
            }
            keep
        });
        for monitor in current {
            if self.surfaces.borrow().iter().any(|s| s.monitor == monitor) {
                continue;
            }
            if let Some(surface) = self.create(&monitor) {
                self.surfaces.borrow_mut().push(surface);
            }
        }
        self.render();
        self.render_tray();
    }
    fn create(self: &Rc<Self>, monitor: &gtk::gdk::Monitor) -> Option<Surface> {
        let app = self.app.upgrade()?;
        let window = gtk::ApplicationWindow::builder()
            .application(&app.application)
            .title("UFLinux panel")
            .decorated(false)
            .build();
        if gtk4_layer_shell::is_supported() {
            window.init_layer_shell();
            window.set_namespace(Some("uflinux-desktop-panel"));
            window.set_layer(Layer::Top);
            window.set_monitor(Some(monitor));
            window.set_anchor(Edge::Bottom, true);
            window.set_anchor(Edge::Left, true);
            window.set_anchor(Edge::Right, true);
            window.set_exclusive_zone(crate::popup::PANEL_HEIGHT);
            window.set_keyboard_mode(KeyboardMode::None);
        }
        window.add_css_class("uflinux-panel");
        window.set_height_request(crate::popup::PANEL_HEIGHT);
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let start = gtk::Button::new();
        let start_content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let logo = gtk::Image::from_file("/usr/share/icons/hicolor/scalable/apps/uflinux.svg");
        logo.set_pixel_size(28);
        start_content.append(&logo);
        start_content.append(&gtk::Label::new(Some("Menu")));
        start.set_child(Some(&start_content));
        start.set_tooltip_text(Some("Start"));
        let weak = self.app.clone();
        let output = monitor
            .connector()
            .map(|s| s.to_string())
            .unwrap_or_default();
        start.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.toggle(&output);
            }
        });
        root.append(&start);
        let tasks = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.append(&tasks);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        root.append(&spacer);
        let tray = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.append(&tray);
        for (index, name, fallback) in [
            (0, "network", "network-offline-symbolic"),
            (1, "sound", "audio-volume-muted-symbolic"),
            (2, "power", "thunderbolt-symbolic"),
        ] {
            let button = gtk::Button::from_icon_name(fallback);
            button.set_tooltip_text(Some(name));
            let target = button.downgrade();
            app.quick.bind_for(
                uflinux_settings::app::Surface::Full,
                &button,
                move |snapshot| {
                    if let Some(button) = target.upgrade() {
                        let values = settings_core::panel::status(snapshot);
                        button.set_icon_name(&values[index].0);
                        button.set_tooltip_text(Some(&values[index].1));
                        button
                            .update_property(&[gtk::accessible::Property::Label(&values[index].1)]);
                    }
                },
            );
            let weak = self.app.clone();
            let output = monitor
                .connector()
                .map(|s| s.to_string())
                .unwrap_or_default();
            button.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.open_quick(name, &output);
                }
            });
            root.append(&button);
        }
        let clock_label = gtk::Label::new(None);
        clock_label.add_css_class("desktop-clock");
        if let Ok(now) = glib::DateTime::now_local() {
            if let Ok(text) = now.format("%H:%M\n%d/%m/%Y") {
                clock_label.set_text(&text);
            }
        }
        let weak_panel = Rc::downgrade(self);
        let last_zone = RefCell::new(String::new());
        app.quick.bind_for(
            uflinux_settings::app::Surface::Full,
            &clock_label,
            move |snapshot| {
                if *last_zone.borrow() != snapshot.datetime.timezone {
                    last_zone.replace(snapshot.datetime.timezone.clone());
                    if let Some(panel) = weak_panel.upgrade() {
                        panel.update_clock();
                    }
                }
            },
        );
        let button = gtk::MenuButton::new();
        button.set_child(Some(&clock_label));
        button.set_tooltip_text(Some("Date and time"));
        let clock = gtk::Popover::new();
        let calendar = gtk::Calendar::new();
        clock.set_child(Some(&calendar));
        button.set_popover(Some(&clock));
        let weak = self.app.clone();
        let clock_weak = clock.downgrade();
        let output = monitor
            .connector()
            .map(|s| s.to_string())
            .unwrap_or_default();
        clock.connect_show(move |_| {
            if let Some(app) = weak.upgrade() {
                if let Some(clock) = clock_weak.upgrade() {
                    app.show_clock(&clock, &output);
                }
            }
        });
        let weak = self.app.clone();
        clock.connect_hide(move |_| {
            if let Some(app) = weak.upgrade() {
                app.clock_closed();
            }
        });
        root.append(&button);
        window.set_child(Some(&root));
        window.present();
        Some(Surface {
            tray,
            tray_buttons: HashMap::new(),
            monitor: monitor.clone(),
            window,
            tasks,
            buttons: HashMap::new(),
            clock,
            clock_label,
        })
    }
}
