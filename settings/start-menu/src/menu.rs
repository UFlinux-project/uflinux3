//! ArcMenu-inspired launcher. All UI and subscriptions are scoped to this window.
use crate::{
    app::App,
    catalog::{self, Catalog, Entry},
    widgets::{self, tr},
};
use glib::variant::ToVariant;
use gtk::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::{Rc, Weak},
    time::{Duration, Instant},
};

const BAR_HEIGHT: i32 = 40;
const START_WIDTH: i32 = 56;

pub struct Menu {
    pub window: gtk::ApplicationWindow,
    output: RefCell<String>,
    app: Weak<App>,
    card: gtk::Box,
    root: gtk::Overlay,
    search: gtk::SearchEntry,
    store: gio::ListStore,
    selection: gtk::SingleSelection,
    view: gtk::ListView,
    heading: gtk::Label,
    browse: gtk::Button,
    error: gtk::Label,
    catalog: RefCell<Catalog>,
    all: Cell<bool>,
    loaded: Cell<bool>,
    closed: Cell<bool>,
    epoch: Cell<u64>,
    presentation: Cell<u64>,
    busy: Cell<bool>,
    confirm: RefCell<Option<gtk::Box>>,
    confirmation_host: gtk::Box,
    monitor: RefCell<Option<(gtk::gdk::Monitor, glib::SignalHandlerId)>>,
    app_monitor: gio::AppInfoMonitor,
    changed: RefCell<Option<glib::SignalHandlerId>>,
    reload_timer: RefCell<Option<glib::SourceId>>,
}
fn icon_button(name: &str, icon: &str) -> gtk::Button {
    let button = gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(tr(name))
        .build();
    button.update_property(&[gtk::accessible::Property::Label(&tr(name))]);
    button
}
fn shortcut(name: &str, icon: &str) -> gtk::Button {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.append(&gtk::Image::from_icon_name(icon));
    let label = gtk::Label::new(Some(&tr(name)));
    label.set_xalign(0.0);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_hexpand(true);
    row.append(&label);
    let button = gtk::Button::builder().child(&row).build();
    button.update_property(&[gtk::accessible::Property::Label(&tr(name))]);
    button.add_css_class("start-shortcut");
    button.add_css_class("flat");
    button
}
impl Menu {
    pub fn new(app: &Rc<App>, output: String) -> Rc<Self> {
        let window = gtk::ApplicationWindow::builder()
            .application(&app.application)
            .title("Start menu")
            .decorated(false)
            .resizable(false)
            .default_width(536)
            .default_height(624)
            .build();
        window.add_css_class("uflinux-start-menu");
        window.add_css_class("start-shell");
        if gtk4_layer_shell::is_supported() {
            window.set_resizable(true);
            window.set_default_size(0, 0);
            window.init_layer_shell();
            window.set_namespace(Some("uflinux-start-menu"));
            window.set_layer(Layer::Overlay);
            window.set_exclusive_zone(-1);
            window.set_keyboard_mode(KeyboardMode::None);
            for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
                window.set_anchor(edge, true);
            }
            crate::popup::select_monitor(&window, &output);
        }
        let root = gtk::Overlay::new();
        let empty = gtk::Box::new(gtk::Orientation::Vertical, 0);
        empty.set_hexpand(true);
        empty.set_vexpand(true);
        root.set_child(Some(&empty));
        let card = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        card.add_css_class("start-card");
        card.add_css_class("uflinux-popup-card");
        card.set_halign(gtk::Align::Start);
        card.set_valign(gtk::Align::End);
        card.set_margin_start(8);
        card.set_margin_end(8);
        card.set_margin_bottom(BAR_HEIGHT + 8);
        root.add_overlay(&card);
        window.set_child(Some(&root));
        let left = gtk::Box::new(gtk::Orientation::Vertical, 8);
        left.set_hexpand(true);
        let heading = gtk::Label::new(Some(&tr("Favorites")));
        heading.set_xalign(0.0);
        heading.add_css_class("start-heading");
        left.append(&heading);
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let selection = gtk::SingleSelection::new(Some(store.clone()));
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            row.add_css_class("start-app-row");
            let image = gtk::Image::new();
            image.set_pixel_size(28);
            row.append(&image);
            let label = gtk::Label::new(None);
            label.set_xalign(0.0);
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            label.set_hexpand(true);
            row.append(&label);
            item.set_child(Some(&row));
        });
        factory.connect_bind(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let object = item.item().and_downcast::<glib::BoxedAnyObject>().unwrap();
            let entry = object.borrow::<Entry>();
            let row = item.child().and_downcast::<gtk::Box>().unwrap();
            let image = row.first_child().and_downcast::<gtk::Image>().unwrap();
            let label = image.next_sibling().and_downcast::<gtk::Label>().unwrap();
            label.set_text(&entry.name);
            if let Ok(icon) = gio::Icon::for_string(&entry.icon) {
                image.set_from_gicon(&icon);
            } else {
                image.set_icon_name(Some("application-x-executable"));
            }
            row.set_tooltip_text(Some(&format!("{}\n{}", entry.name, entry.description)));
            row.update_property(&[gtk::accessible::Property::Label(&entry.name)]);
        });
        factory.connect_unbind(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            if let Some(row) = item.child().and_downcast::<gtk::Box>() {
                if let Some(image) = row.first_child().and_downcast::<gtk::Image>() {
                    image.clear();
                }
                row.set_tooltip_text(None);
            }
        });
        let view = gtk::ListView::new(Some(selection.clone()), Some(factory));
        view.set_single_click_activate(true);
        view.add_css_class("start-apps");
        view.update_property(&[gtk::accessible::Property::Label(&tr("Applications"))]);
        let scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .min_content_height(64)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&view)
            .build();
        left.append(&scroll);
        let error = widgets::description("");
        error.add_css_class("error");
        error.set_lines(3);
        error.set_ellipsize(gtk::pango::EllipsizeMode::End);
        error.set_visible(false);
        left.append(&error);
        let browse = shortcut("All Applications", "go-next-symbolic");
        left.append(&browse);
        let search = gtk::SearchEntry::builder()
            .placeholder_text(tr("Search applications…"))
            .hexpand(true)
            .build();
        search.set_width_chars(6);
        search.set_max_width_chars(24);
        search.update_property(&[gtk::accessible::Property::Label(&tr("Search applications"))]);
        left.append(&search);
        card.append(&left);
        let right = gtk::Box::new(gtk::Orientation::Vertical, 8);
        right.add_css_class("start-places");
        let places = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let account = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let avatar = gtk::Image::from_icon_name("avatar-default-symbolic");
        avatar.set_pixel_size(32);
        account.append(&avatar);
        let name = glib::real_name();
        let name = name.to_string_lossy();
        let login_os = glib::user_name();
        let login = login_os.to_string_lossy();
        let username = gtk::Label::new(Some(if name.is_empty() || name == "Unknown" {
            &login
        } else {
            &name
        }));
        username.set_ellipsize(gtk::pango::EllipsizeMode::End);
        username.set_xalign(0.0);
        username.set_hexpand(true);
        account.append(&username);
        places.append(&account);
        places.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        let right_scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .min_content_height(64)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&places)
            .build();
        right.append(&right_scroll);
        let power_row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        power_row.set_halign(gtk::Align::End);
        let actions = [
            ("Log out", "system-log-out-symbolic", "Logout"),
            ("Suspend", "media-playback-pause-symbolic", "Suspend"),
            ("Restart", "system-reboot-symbolic", "Reboot"),
            ("Shut down", "system-shutdown-symbolic", "PowerOff"),
        ];
        let power_buttons = actions
            .iter()
            .map(|(name, icon, _)| {
                let b = icon_button(name, icon);
                b.add_css_class("start-power");
                b.add_css_class("flat");
                power_row.append(&b);
                b
            })
            .collect::<Vec<_>>();
        right.append(&power_row);
        let right_overlay = gtk::Overlay::new();
        right_overlay.set_child(Some(&right));
        let confirmation_host = widgets::column();
        confirmation_host.set_visible(false);
        confirmation_host.set_valign(gtk::Align::End);
        confirmation_host.set_margin_bottom(44);
        confirmation_host.add_css_class("start-confirmation");
        right_overlay.add_overlay(&confirmation_host);
        card.append(&right_overlay);
        let menu = Rc::new(Self {
            window,
            output: RefCell::new(output),
            app: Rc::downgrade(app),
            card,
            root,
            search,
            store,
            selection,
            view,
            heading,
            browse,
            error,
            catalog: RefCell::new(Catalog::default()),
            all: Cell::new(false),
            loaded: Cell::new(false),
            closed: Cell::new(false),
            epoch: Cell::new(0),
            presentation: Cell::new(0),
            busy: Cell::new(false),
            confirm: RefCell::new(None),
            confirmation_host,
            monitor: RefCell::new(None),
            app_monitor: gio::AppInfoMonitor::get(),
            changed: RefCell::new(None),
            reload_timer: RefCell::new(None),
        });
        menu.places(&places);
        let weak = Rc::downgrade(&menu);
        menu.browse.connect_clicked(move |_| {
            if let Some(menu) = weak.upgrade() {
                menu.all.set(!menu.all.get());
                menu.rebuild();
                menu.search.grab_focus();
            }
        });
        let weak = Rc::downgrade(&menu);
        menu.search.connect_search_changed(move |_| {
            if let Some(menu) = weak.upgrade() {
                menu.failure("");
                menu.rebuild();
            }
        });
        let weak = Rc::downgrade(&menu);
        menu.search.connect_activate(move |_| {
            if let Some(menu) = weak.upgrade() {
                menu.launch_selected();
            }
        });
        let weak = Rc::downgrade(&menu);
        menu.view.connect_activate(move |_, position| {
            if let Some(menu) = weak.upgrade() {
                menu.selection.set_selected(position);
                menu.launch_selected();
            }
        });
        for (button, (label, _, action)) in power_buttons.iter().zip(actions) {
            let weak = Rc::downgrade(&menu);
            button.connect_clicked(move |_| {
                if let Some(menu) = weak.upgrade() {
                    if action == "Suspend" {
                        menu.session_action(action);
                    } else {
                        menu.confirm_action(label, action);
                    }
                }
            });
            if action != "Logout" {
                Self::capability(&menu, button, action);
            }
        }
        menu.wire();
        menu.reload();
        menu
    }
    pub fn matches_output(&self, output: &str) -> bool {
        output.is_empty()
            || *self.output.borrow() == output
            || self
                .monitor
                .borrow()
                .as_ref()
                .is_some_and(|(monitor, _)| monitor.connector().as_deref() == Some(output))
    }
    pub fn ready(&self) -> bool {
        self.loaded.get()
    }
    pub fn set_output(self: &Rc<Self>, output: &str) {
        if *self.output.borrow() != output {
            self.hide();
            self.output.replace(output.to_string());
        }
        if gtk4_layer_shell::is_supported() {
            let display = gtk::prelude::WidgetExt::display(&self.window);
            let monitor = display
                .monitors()
                .iter::<gtk::gdk::Monitor>()
                .flatten()
                .find(|monitor| monitor.connector().as_deref() == Some(output));
            self.window.set_monitor(monitor.as_ref());
            if let Some(monitor) = monitor {
                self.track_monitor(&monitor);
            }
        }
    }
    fn escape(&self) {
        let confirming = self.confirm.borrow().is_some();
        if confirming {
            self.dismiss_confirmation();
            self.search.grab_focus();
        } else if !self.search.text().is_empty() {
            self.search.set_text("");
            self.rebuild();
        } else if self.all.replace(false) {
            self.rebuild();
            self.search.grab_focus();
        } else {
            self.hide();
        }
    }
    fn dismiss_confirmation(&self) {
        let previous = self.confirm.borrow_mut().take();
        self.confirmation_host.set_visible(false);
        if let Some(panel) = previous {
            panel.set_visible(false);
            let host = self.confirmation_host.clone();
            // Release the event target after GTK finishes dispatching it.
            glib::idle_add_local_once(move || {
                if panel.parent().is_some() {
                    host.remove(&panel);
                }
            });
        }
    }
    pub fn hide(&self) {
        self.presentation
            .set(self.presentation.get().wrapping_add(1));
        self.window.set_visible(false);
        if gtk4_layer_shell::is_supported() {
            self.window.set_keyboard_mode(KeyboardMode::None);
        }
        self.dismiss_confirmation();
        self.all.set(false);
        self.search.set_text("");
        self.failure("");
        self.rebuild();
        if self.store.n_items() > 0 {
            self.selection.set_selected(0);
        }
        self.prepare();
        if let Some(app) = self.app.upgrade() {
            app.report_status();
        }
    }
    // Build and allocate the default screen without mapping a Wayland surface.
    // GTK/compositor buffers are created on presentation, not by flashing a window.
    fn prepare(&self) {
        if self.window.is_visible() || self.closed.get() {
            return;
        }
        let display = gtk::prelude::WidgetExt::display(&self.window);
        let monitor = display
            .monitors()
            .item(0)
            .and_downcast::<gtk::gdk::Monitor>();
        let (width, height) = monitor
            .map(|m| {
                let geometry = m.geometry();
                (geometry.width(), geometry.height())
            })
            .unwrap_or((1280, 720));
        self.layout(width, height);
        self.root.measure(gtk::Orientation::Horizontal, height);
        self.root.measure(gtk::Orientation::Vertical, width);
        self.root.allocate(width, height, -1, None);
    }
    pub fn shutdown(&self) {
        self.cleanup();
        self.window.destroy();
    }
    pub fn present(self: &Rc<Self>) {
        self.presentation
            .set(self.presentation.get().wrapping_add(1));
        let began = self
            .app
            .upgrade()
            .map(|app| app.frame_start())
            .unwrap_or_else(Instant::now);
        if gtk4_layer_shell::is_supported() {
            self.window.set_keyboard_mode(KeyboardMode::Exclusive);
        }
        self.window.present();
        if let Some(app) = self.app.upgrade() {
            app.report_status();
        }
        self.search.grab_focus();
        if std::env::var_os("UFLINUX_SETTINGS_BENCH").is_some() {
            let weak = Rc::downgrade(self);
            self.window.add_tick_callback(move |window, _| {
                if !weak.upgrade().is_some_and(|menu| menu.loaded.get()) {
                    return glib::ControlFlow::Continue;
                }
                if let Some(clock) = window.frame_clock() {
                    let token = Rc::new(RefCell::new(None));
                    let target = token.clone();
                    let id = clock.connect_after_paint(move |clock| {
                        eprintln!(
                            "UFLINUX_START_MENU_FIRST_FRAME_MS={:.1}",
                            began.elapsed().as_secs_f64() * 1000.0
                        );
                        if let Some(id) = target.borrow_mut().take() {
                            clock.disconnect(id);
                        }
                    });
                    token.replace(Some(id));
                }
                glib::ControlFlow::Break
            });
        }
    }
    fn wire(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.window.connect_realize(move |window| {
            if let Some(surface) = window.surface() {
                let target = weak.clone();
                surface.connect_layout(move |_, w, h| {
                    if let Some(menu) = target.upgrade() {
                        menu.layout(w, h);
                    }
                });
                let target = weak.clone();
                surface.connect_enter_monitor(move |_, monitor| {
                    if let Some(menu) = target.upgrade() {
                        menu.track_monitor(monitor);
                    }
                });
            }
        });
        let weak = Rc::downgrade(self);
        self.window.connect_map(move |window| {
            if let Some(menu) = weak.upgrade() {
                menu.layout(window.width(), window.height());
                menu.search.grab_focus();
                if let Some(monitor) = gtk4_layer_shell::is_supported()
                    .then(|| window.monitor())
                    .flatten()
                {
                    menu.track_monitor(&monitor);
                }
            }
        });
        let gesture = gtk::GestureClick::new();
        gesture.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        gesture.connect_pressed(move |gesture, _, x, y| {
            if let Some(menu) = weak.upgrade() {
                if menu.card.compute_bounds(&menu.root).is_some_and(|r| {
                    r.contains_point(&gtk::graphene::Point::new(x as f32, y as f32))
                }) {
                    gesture.set_state(gtk::EventSequenceState::Denied);
                } else {
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                    menu.hide();
                }
            }
        });
        self.root.add_controller(gesture);
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, _| {
            let Some(menu) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if key == gtk::gdk::Key::Escape {
                menu.escape();
                return glib::Propagation::Stop;
            }
            if (key == gtk::gdk::Key::Down || key == gtk::gdk::Key::Up)
                && gtk::prelude::RootExt::focus(&menu.window).is_some_and(|w| {
                    w == menu.search.clone().upcast::<gtk::Widget>() || w.is_ancestor(&menu.search)
                })
                && menu.store.n_items() > 0
            {
                let current = menu.selection.selected().min(menu.store.n_items() - 1);
                menu.selection.set_selected(current);
                menu.view.grab_focus();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.window.add_controller(keys);
        let seen = Rc::new(Cell::new(false));
        let weak = Rc::downgrade(self);
        self.window.connect_is_active_notify(move |window| {
            if window.is_active() {
                seen.set(true);
            } else if seen.get() {
                let weak = weak.clone();
                glib::idle_add_local_once(move || {
                    if let Some(menu) = weak.upgrade() {
                        let owned_modal = menu.app.upgrade().is_some_and(|app| {
                            app.application
                                .windows()
                                .iter()
                                .any(|w| w.is_visible() && w.is_modal())
                        });
                        if !menu.closed.get()
                            && !menu.busy.get()
                            && !menu.window.is_active()
                            && !owned_modal
                        {
                            menu.hide();
                        }
                    }
                });
            }
        });
        let weak = Rc::downgrade(self);
        self.window.connect_close_request(move |_| {
            if let Some(menu) = weak.upgrade() {
                menu.hide();
            }
            glib::Propagation::Stop
        });
        let weak = Rc::downgrade(self);
        let id = self.app_monitor.connect_changed(move |_| {
            if let Some(menu) = weak.upgrade() {
                if let Some(timer) = menu.reload_timer.borrow_mut().take() {
                    timer.remove();
                }
                let weak = Rc::downgrade(&menu);
                let timer = glib::timeout_add_local_once(Duration::from_millis(100), move || {
                    if let Some(menu) = weak.upgrade() {
                        menu.reload_timer.borrow_mut().take();
                        menu.reload();
                    }
                });
                menu.reload_timer.replace(Some(timer));
            }
        });
        self.changed.replace(Some(id));
    }
    fn layout(&self, width: i32, height: i32) {
        if width < 1 || height < 1 || self.closed.get() {
            return;
        }
        self.card.set_size_request(
            (width - 16).clamp(200, 520),
            (height - BAR_HEIGHT - 24).clamp(160, 560),
        );
        if let Some(surface) = self.window.surface() {
            crate::popup::set_input_region(&surface, width, height, START_WIDTH);
        }
    }
    fn track_monitor(self: &Rc<Self>, monitor: &gtk::gdk::Monitor) {
        if self
            .monitor
            .borrow()
            .as_ref()
            .is_some_and(|(m, _)| m == monitor)
        {
            return;
        }
        if let Some((old, id)) = self.monitor.borrow_mut().take() {
            old.disconnect(id);
        }
        let weak = Rc::downgrade(self);
        let id = monitor.connect_invalidate(move |_| {
            if let Some(menu) = weak.upgrade() {
                menu.hide();
            }
        });
        self.monitor.replace(Some((monitor.clone(), id)));
    }
    fn cleanup(&self) {
        if self.closed.replace(true) {
            return;
        }
        self.epoch.set(self.epoch.get() + 1);
        if let Some(id) = self.changed.borrow_mut().take() {
            self.app_monitor.disconnect(id);
        }
        if let Some(id) = self.reload_timer.borrow_mut().take() {
            id.remove();
        }
        if let Some((monitor, id)) = self.monitor.borrow_mut().take() {
            monitor.disconnect(id);
        }
        self.dismiss_confirmation();
        self.store.remove_all();
        self.catalog.replace(Catalog::default());
    }
    fn reload(self: &Rc<Self>) {
        if self.closed.get() {
            return;
        }
        let epoch = self.epoch.get() + 1;
        self.epoch.set(epoch);
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let result = gio::spawn_blocking(catalog::load).await;
            if let Some(menu) = weak.upgrade() {
                if !menu.closed.get() && menu.epoch.get() == epoch {
                    match result {
                        Ok(data) => {
                            menu.catalog.replace(data);
                            menu.loaded.set(true);
                            menu.rebuild();
                            menu.prepare();
                            if let Some(app) = menu.app.upgrade() {
                                app.report_status();
                            }
                        }
                        Err(_) => menu.failure("Applications could not be loaded"),
                    }
                }
            }
        });
    }
    fn rebuild(&self) {
        let query = self.search.text();
        let data = self.catalog.borrow();
        let rows = if !query.trim().is_empty() {
            catalog::search(&data.entries, &query)
        } else if self.all.get() {
            data.entries.clone()
        } else {
            data.favorites.clone()
        };
        self.heading.set_text(&tr(if !self.loaded.get() {
            "Loading applications…"
        } else if !query.trim().is_empty() {
            "Search results"
        } else if self.all.get() {
            "All Applications"
        } else {
            "Favorites"
        }));
        self.browse.set_tooltip_text(Some(&tr(if self.all.get() {
            "Back to favorites"
        } else {
            "All Applications"
        })));
        if let Some(row) = self.browse.child().and_downcast::<gtk::Box>() {
            if let Some(label) = row.last_child().and_downcast::<gtk::Label>() {
                label.set_text(&tr(if self.all.get() {
                    "Back to favorites"
                } else {
                    "All Applications"
                }));
            }
        }
        self.browse
            .update_property(&[gtk::accessible::Property::Label(&tr(if self.all.get() {
                "Back to favorites"
            } else {
                "All Applications"
            }))]);
        // Preserve bound rows and their icons when returning to the same screen.
        let unchanged = self.store.n_items() as usize == rows.len()
            && rows.iter().enumerate().all(|(index, entry)| {
                self.store
                    .item(index as u32)
                    .and_downcast::<glib::BoxedAnyObject>()
                    .is_some_and(|object| *object.borrow::<Entry>() == *entry)
            });
        if !unchanged {
            let objects: Vec<_> = rows.into_iter().map(glib::BoxedAnyObject::new).collect();
            self.store.splice(0, self.store.n_items(), &objects);
        }
        if self.loaded.get() && self.store.n_items() == 0 {
            self.heading.set_text(&tr(if query.is_empty() {
                "No applications available"
            } else {
                "No matching applications"
            }));
        }
    }
    pub fn failure(&self, message: &str) {
        self.error
            .set_tooltip_text((!message.is_empty()).then_some(message));
        self.error.set_text(message);
        self.error.set_visible(!message.is_empty());
    }
    fn settings(self: &Rc<Self>, page: &str) {
        if let Some(app) = self.app.upgrade() {
            app.open_page(page);
        }
    }
    fn launch_selected(self: &Rc<Self>) {
        if self.busy.get() {
            return;
        }
        let Some(object) = self
            .selection
            .selected_item()
            .and_downcast::<glib::BoxedAnyObject>()
        else {
            return;
        };
        let id = object.borrow::<Entry>().id.clone();
        if id == "org.uflinux.Settings.desktop" {
            self.settings("network");
            return;
        }
        let Some(info) = gio::DesktopAppInfo::new(&id).filter(|a| a.should_show()) else {
            self.failure(&tr("This application is no longer available"));
            self.reload();
            return;
        };
        self.busy.set(true);
        self.view.set_sensitive(false);
        self.failure("");
        let context = gtk::prelude::WidgetExt::display(&self.window).app_launch_context();
        let weak = Rc::downgrade(self);
        let presentation = self.presentation.get();
        glib::MainContext::default().spawn_local(async move {
            let result = info.launch_uris_future(&[], Some(&context)).await;
            if let Some(menu) = weak.upgrade() {
                menu.busy.set(false);
                menu.view.set_sensitive(true);
                if menu.presentation.get() != presentation || !menu.window.is_visible() {
                    return;
                }
                match result {
                    Ok(()) => menu.hide(),
                    Err(e) => {
                        menu.failure(&format!("{}: {}", tr("Could not launch application"), e));
                        if menu.window.is_visible() {
                            menu.search.grab_focus();
                        }
                    }
                }
            }
        });
    }
    fn places(self: &Rc<Self>, parent: &gtk::Box) {
        let home = glib::home_dir();
        self.place(parent, "Home", "user-home-symbolic", Some(home.clone()));
        for (name, icon, kind) in [
            (
                "Documents",
                "folder-documents-symbolic",
                glib::UserDirectory::Documents,
            ),
            (
                "Downloads",
                "folder-download-symbolic",
                glib::UserDirectory::Downloads,
            ),
            ("Music", "folder-music-symbolic", glib::UserDirectory::Music),
            (
                "Pictures",
                "folder-pictures-symbolic",
                glib::UserDirectory::Pictures,
            ),
            (
                "Videos",
                "folder-videos-symbolic",
                glib::UserDirectory::Videos,
            ),
        ] {
            let path = glib::user_special_dir(kind).filter(|p| p != &home);
            self.place(parent, name, icon, path);
        }
        parent.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        let settings = shortcut("Settings", "preferences-system-symbolic");
        let weak = Rc::downgrade(self);
        settings.connect_clicked(move |_| {
            if let Some(menu) = weak.upgrade() {
                menu.settings("network");
            }
        });
        parent.append(&settings);
        let terminal = shortcut("Terminal", "utilities-terminal-symbolic");
        let weak = Rc::downgrade(self);
        terminal.connect_clicked(move |_| {
            if let Some(menu) = weak.upgrade() {
                if let Some(entry) = menu
                    .catalog
                    .borrow()
                    .entries
                    .iter()
                    .find(|a| a.id == "foot.desktop")
                    .cloned()
                {
                    menu.store.remove_all();
                    menu.store.append(&glib::BoxedAnyObject::new(entry));
                    menu.launch_selected();
                } else {
                    menu.failure(&tr("Terminal is not installed"));
                }
            }
        });
        parent.append(&terminal);
    }
    fn place(self: &Rc<Self>, parent: &gtk::Box, name: &str, icon: &str, path: Option<PathBuf>) {
        let button = shortcut(name, icon);
        if let Some(path) = path.filter(|p| p.is_dir()) {
            let uri = gio::File::for_path(path).uri();
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(menu) = weak.upgrade() {
                    let uri = uri.clone();
                    let weak = Rc::downgrade(&menu);
                    let presentation = menu.presentation.get();
                    let context =
                        gtk::prelude::WidgetExt::display(&menu.window).app_launch_context();
                    glib::MainContext::default().spawn_local(async move {
                        let result =
                            gio::AppInfo::launch_default_for_uri_future(&uri, Some(&context)).await;
                        if let Some(menu) = weak.upgrade() {
                            if menu.presentation.get() != presentation || !menu.window.is_visible()
                            {
                                return;
                            }
                            match result {
                                Ok(()) => menu.hide(),
                                Err(e) => {
                                    menu.failure(&format!("{}: {}", tr("Could not open folder"), e))
                                }
                            }
                        }
                    });
                }
            });
        } else {
            button.set_sensitive(false);
            button.set_tooltip_text(Some(&tr("This folder is not configured or does not exist")));
        }
        parent.append(&button);
    }
    fn capability(menu: &Rc<Self>, button: &gtk::Button, action: &'static str) {
        button.set_sensitive(false);
        button.set_tooltip_text(Some(&tr("Checking whether this action is available…")));
        let target = button.downgrade();
        let weak = Rc::downgrade(menu);
        glib::MainContext::default().spawn_local(async move {
            let result = async {
                let connection = gio::bus_get_future(gio::BusType::System)
                    .await
                    .map_err(|e| e.to_string())?;
                let reply = connection
                    .call_future(
                        Some("org.freedesktop.login1"),
                        "/org/freedesktop/login1",
                        "org.freedesktop.login1.Manager",
                        &format!("Can{action}"),
                        None,
                        None,
                        gio::DBusCallFlags::NO_AUTO_START,
                        5000,
                    )
                    .await
                    .map_err(|e| e.to_string())?;
                Ok::<_, String>(reply.get::<(String,)>().map(|v| v.0).unwrap_or_default())
            }
            .await;
            if weak.upgrade().is_some_and(|m| !m.closed.get()) {
                if let Some(button) = target.upgrade() {
                    let allowed = matches!(&result,Ok(v) if v=="yes" || v=="challenge");
                    button.set_sensitive(allowed);
                    button.set_tooltip_text(Some(&match result {
                        Ok(v) if allowed => {
                            if v == "challenge" {
                                tr("Authorization may be required").to_string()
                            } else {
                                action.to_string()
                            }
                        }
                        Ok(_) => tr("This action is unavailable on this system").to_string(),
                        Err(e) => format!("{}: {e}", tr("Session manager unavailable")),
                    }));
                }
            }
        });
    }
    fn confirm_action(self: &Rc<Self>, label: &'static str, action: &'static str) {
        if self.busy.get() {
            return;
        }
        self.dismiss_confirmation();
        // Keep confirmation in the same native surface so keyboard focus and
        // rapid Escape events cannot get stranded in a closing Wayland popup.
        let content = widgets::column();
        content.append(&widgets::description(&format!("{}?", tr(label))));
        let buttons = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let cancel = gtk::Button::with_label(&tr("Cancel"));
        let confirm = gtk::Button::with_label(&tr(label));
        confirm.add_css_class("destructive-action");
        buttons.append(&cancel);
        buttons.append(&confirm);
        content.append(&buttons);
        let weak = Rc::downgrade(self);
        cancel.connect_clicked(move |_| {
            if let Some(menu) = weak.upgrade() {
                menu.dismiss_confirmation();
                menu.search.grab_focus();
            }
        });
        let weak = Rc::downgrade(self);
        confirm.connect_clicked(move |_| {
            if let Some(menu) = weak.upgrade() {
                menu.dismiss_confirmation();
                menu.session_action(action);
            }
        });
        self.confirmation_host.append(&content);
        self.confirm.replace(Some(content));
        self.confirmation_host.set_visible(true);
        cancel.grab_focus();
    }
    fn session_action(self: &Rc<Self>, action: &'static str) {
        if self.busy.replace(true) {
            return;
        }
        self.failure("");
        if action != "Logout" {
            if let Some(app) = self.app.upgrade() {
                app.ensure_agent();
            }
        }
        self.card.set_sensitive(false);
        let weak = Rc::downgrade(self);
        let presentation = self.presentation.get();
        glib::MainContext::default().spawn_local(async move {
            let result = if action == "Logout" {
                match gio::Subprocess::newv(
                    &[
                        std::ffi::OsStr::new("labwc"),
                        std::ffi::OsStr::new("--exit"),
                    ],
                    gio::SubprocessFlags::STDERR_PIPE,
                ) {
                    Ok(process) => process.wait_check_future().await.map_err(|e| e.to_string()),
                    Err(e) => Err(e.to_string()),
                }
            } else {
                match gio::bus_get_future(gio::BusType::System).await {
                    Ok(connection) => connection
                        .call_future(
                            Some("org.freedesktop.login1"),
                            "/org/freedesktop/login1",
                            "org.freedesktop.login1.Manager",
                            action,
                            Some(&(true,).to_variant()),
                            None,
                            gio::DBusCallFlags::ALLOW_INTERACTIVE_AUTHORIZATION,
                            120000,
                        )
                        .await
                        .map(|_| ())
                        .map_err(|e| e.to_string()),
                    Err(e) => Err(e.to_string()),
                }
            };
            if let Some(menu) = weak.upgrade() {
                menu.busy.set(false);
                menu.card.set_sensitive(true);
                if let Some(app) = menu.app.upgrade() {
                    app.agent.borrow_mut().take();
                }
                if menu.presentation.get() != presentation || !menu.window.is_visible() {
                    return;
                }
                match result {
                    Ok(()) => menu.hide(),
                    Err(e) => {
                        menu.failure(&format!("{}: {e}", tr("Action could not be completed")));
                        if menu.window.is_visible() {
                            menu.search.grab_focus();
                        }
                    }
                }
            }
        });
    }
}
