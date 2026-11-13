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

/// Panel logo size in device-independent pixels.
const LOGO_PIXELS: i32 = 28;
/// Layout cell one tray icon occupies in the overflow flyout, so the grid can be
/// sized from the output width instead of overflowing a small monitor.
const TRAY_CELL: i32 = 44;
/// The flyout keeps a usable shape on a tiny output and stops widening at eight
/// columns on a large one, trading a little width for a shorter popup.
const TRAY_MIN_COLUMNS: i32 = 4;
const TRAY_MAX_COLUMNS: i32 = 8;
/// Beyond this many icons deep the flyout scrolls rather than growing upwards
/// past the panel and off the screen.
const TRAY_FLYOUT_MAX_HEIGHT: i32 = 240;

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
    /// Grid cell the icon currently occupies, so a render only re-attaches a
    /// button whose position actually moved.
    cell: (i32, i32),
}
struct Surface {
    overflow: gtk::Button,
    overflow_popover: gtk::Popover,
    overflow_scroll: gtk::ScrolledWindow,
    overflow_grid: gtk::Grid,
    overflow_columns: i32,
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
            .flat_map(|s| {
                std::iter::once(s.overflow_popover.clone())
                    .chain(s.tray_buttons.values().map(|item| item.popover.clone()))
            })
            .collect();
        for popover in popovers {
            // An item menu lives inside the flyout, so the flyout is an
            // ancestor of it and has to stay open while that menu is up;
            // popping it would take the menu down with it.
            let keep =
                except.is_some_and(|except| except == &popover || except.is_ancestor(&popover));
            if !keep {
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
    /// Column count the flyout grid should use for an output of this width. The
    /// popup is clamped to the monitor so a handful of icons never stretch a
    /// small display's flyout past its edge.
    fn flyout_columns(monitor: &gtk::gdk::Monitor) -> i32 {
        let width = monitor.geometry().width();
        let available = (width / TRAY_CELL).min(TRAY_MAX_COLUMNS);
        available.max(TRAY_MIN_COLUMNS)
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
                    surface.overflow_grid.remove(&item.button);
                    false
                }
            });
            // Nothing to hide means no chevron at all, which keeps the panel at
            // the width it had before any tray app started.
            surface.overflow.set_visible(!items.is_empty());
            if items.is_empty() {
                surface.overflow_popover.popdown();
            }
            let label = match items.len() {
                1 => "Show 1 hidden icon".to_string(),
                count => format!("Show {count} hidden icons"),
            };
            surface.overflow.set_tooltip_text(Some(&label));
            surface
                .overflow
                .update_property(&[gtk::accessible::Property::Label(&label)]);
            let columns = Self::flyout_columns(&surface.monitor);
            if columns != surface.overflow_columns {
                surface.overflow_columns = columns;
            }
            let columns = surface.overflow_columns;
            // A popover sizes to its content's natural width, which a
            // homogeneous grid reports as only a few columns. Pin the grid to
            // the width its icons actually need so a flyout of several icons is
            // never squeezed into a single clipped column.
            let used = (items.len() as i32).clamp(1, columns);
            surface.overflow_grid.set_size_request(used * TRAY_CELL, -1);
            surface
                .overflow_scroll
                .set_max_content_width(columns * TRAY_CELL);
            for (index, item) in items.iter().enumerate() {
                let cell = ((index as i32) % columns, (index as i32) / columns);
                let button = surface
                    .tray_buttons
                    .entry(item.key.clone())
                    .or_insert_with(|| {
                        let button = gtk::Button::new();
                        let image = gtk::Image::new();
                        image.set_pixel_size(18);
                        button.set_child(Some(&image));
                        button.add_css_class("uflinux-tray-icon");
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
                        TrayButton {
                            button,
                            image,
                            popover,
                            content,
                            revision: item.menu_revision,
                            cell,
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
                // Attaching a button that is already in the grid moves it, so a
                // registration or an output change re-lays the icons out without
                // rebuilding their menus or losing an open one.
                if button.cell != cell || button.button.parent().is_none() {
                    surface
                        .overflow_grid
                        .attach(&button.button, cell.0, cell.1, 1, 1);
                    button.cell = cell;
                }
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
                surface.overflow_popover.unparent();
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
        let logo = gtk::Image::new();
        logo.set_pixel_size(LOGO_PIXELS);
        // The installed logo is a scalable document whose intrinsic size is far
        // larger than the panel. Decoding it at the pixel size actually shown
        // keeps the texture tiny; a file paintable would ignore pixel size.
        let scale = window.scale_factor().max(1);
        match gdk_pixbuf::Pixbuf::from_file_at_scale(
            "/usr/share/icons/hicolor/scalable/apps/uflinux.svg",
            LOGO_PIXELS * scale,
            LOGO_PIXELS * scale,
            true,
        ) {
            Ok(pixbuf) => logo.set_paintable(Some(&gtk::gdk::Texture::for_pixbuf(&pixbuf))),
            Err(error) => eprintln!("Could not load the panel logo: {error}"),
        }
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
        // StatusNotifier icons live behind a single chevron, as on Windows, so a
        // dozen background apps do not push the clock off a small panel. The
        // chevron stays hidden until an item registers.
        let grid = gtk::Grid::new();
        grid.set_column_homogeneous(true);
        grid.set_row_homogeneous(true);
        grid.set_column_spacing(2);
        grid.set_row_spacing(2);
        grid.add_css_class("uflinux-tray-flyout");
        let scroll = gtk::ScrolledWindow::builder()
            .child(&grid)
            // The grid is at most TRAY_MAX_COLUMNS wide, so it always fits
            // horizontally and only ever needs to scroll vertically. An
            // External/Always policy here would clip the last column instead.
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .propagate_natural_width(true)
            .propagate_natural_height(true)
            .max_content_height(TRAY_FLYOUT_MAX_HEIGHT)
            .build();
        scroll.add_css_class("uflinux-tray-scroll");
        let overflow_popover = gtk::Popover::builder()
            .child(&scroll)
            .position(gtk::PositionType::Top)
            .has_arrow(false)
            .build();
        overflow_popover.add_css_class("uflinux-tray-popover");
        // A plain button rather than a MenuButton: GTK exposes a MenuButton as
        // a combo box with no accessibility action, which would leave the
        // chevron unreachable to assistive tech and to the a11y test harness.
        let overflow = gtk::Button::from_icon_name("pan-up-symbolic");
        overflow.add_css_class("uflinux-tray-chevron");
        overflow.set_tooltip_text(Some("Hidden icons"));
        overflow_popover.set_parent(&overflow);
        overflow.set_visible(false);
        let weak = self.app.clone();
        let output = monitor
            .connector()
            .map(|s| s.to_string())
            .unwrap_or_default();
        let target = overflow_popover.downgrade();
        overflow_popover.connect_show(move |_| {
            if let (Some(app), Some(popover)) = (weak.upgrade(), target.upgrade()) {
                app.show_tray(&popover, &output);
            }
        });
        let weak = self.app.clone();
        overflow_popover.connect_hide(move |_| {
            if let Some(app) = weak.upgrade() {
                app.tray_closed();
            }
        });
        let open = overflow.clone();
        overflow_popover.connect_show(move |_| open.add_css_class("open"));
        let closed = overflow.clone();
        overflow_popover.connect_hide(move |_| closed.remove_css_class("open"));
        let toggle = overflow_popover.clone();
        overflow.connect_clicked(move |_| {
            if toggle.is_visible() {
                toggle.popdown();
            } else {
                toggle.popup();
            }
        });
        root.append(&overflow);
        let overflow_columns = Self::flyout_columns(monitor);
        scroll.set_max_content_width(overflow_columns * TRAY_CELL);
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
            overflow,
            overflow_popover,
            overflow_scroll: scroll,
            overflow_grid: grid,
            overflow_columns,
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
