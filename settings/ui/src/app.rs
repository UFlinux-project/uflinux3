use crate::{auth, pages, widgets};
use gtk::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use settings_core::{bus, *};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Instant,
};

#[derive(Clone, Copy)]
pub enum Surface {
    Full,
    Quick,
}
type Binding = Box<dyn Fn(&Snapshot) -> bool>;
pub struct App {
    pub application: gtk::Application,
    pub state: RefCell<Snapshot>,
    pub agent: RefCell<Option<auth::Agent>>,
    full: RefCell<Option<gtk::ApplicationWindow>>,
    quick: RefCell<Option<gtk::ApplicationWindow>>,
    pub stack: RefCell<Option<gtk::Stack>>,
    full_bindings: RefCell<Vec<Binding>>,
    quick_bindings: RefCell<Vec<Binding>>,
    banner: RefCell<Vec<glib::WeakRef<gtk::Label>>>,
    quick_output: RefCell<String>,
    started: Instant,
    auth_id: Cell<u64>,
    pub quick_auth: RefCell<Option<gtk::Box>>,
    pub auth_window: RefCell<Option<gtk::ApplicationWindow>>,
    activation: Cell<Instant>,
    first_frame: Cell<bool>,
    loaded: Cell<bool>,
}
pub const PAGES: &[(&str, &str)] = &[
    ("network", "Network"),
    ("displays", "Displays"),
    ("audio", "Sound"),
    ("power", "Power"),
    ("default-apps", "Default Applications"),
    ("datetime", "Date & Time"),
    ("appearance", "Appearance"),
    ("about", "About"),
];
pub fn run() {
    let began = Instant::now();
    let application = gtk::Application::builder()
        .application_id("org.uflinux.Settings")
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    let controller: Rc<RefCell<Option<Rc<App>>>> = Rc::new(RefCell::new(None));
    let slot = controller.clone();
    application.connect_startup(move |application| {
        let css = gtk::CssProvider::new();
        css.load_from_string(include_str!("../assets/style.css"));
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &css,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
        let app = Rc::new(App {
            application: application.clone(),
            state: RefCell::new(Snapshot::default()),
            agent: RefCell::new(None),
            full: RefCell::new(None),
            quick: RefCell::new(None),
            stack: RefCell::new(None),
            full_bindings: RefCell::new(Vec::new()),
            quick_bindings: RefCell::new(Vec::new()),
            banner: RefCell::new(Vec::new()),
            quick_output: RefCell::new(String::new()),
            started: began,
            auth_id: Cell::new(0),
            quick_auth: RefCell::new(None),
            auth_window: RefCell::new(None),
            activation: Cell::new(began),
            first_frame: Cell::new(true),
            loaded: Cell::new(false),
        });
        app.agent
            .replace(Some(auth::Agent::register(Rc::downgrade(&app))));
        app.connect();
        slot.replace(Some(app));
    });
    let slot = controller.clone();
    application.connect_command_line(move |_, line| {
        let Some(app) = slot.borrow().clone() else {
            return glib::ExitCode::FAILURE;
        };
        app.activation.set(Instant::now());
        let args = line
            .arguments()
            .iter()
            .map(|s| s.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        let value = |key: &str| {
            args.iter()
                .position(|s| s == key)
                .and_then(|i| args.get(i + 1))
                .cloned()
        };
        if args.iter().any(|s| s == "--quick-settings") {
            app.open_quick(value("--output").unwrap_or_default());
        } else if value("--network-auth").is_some() {
            if app.parent().is_none() {
                app.open_quick(String::new());
            }
            app.show_auth();
        } else {
            app.open_page(&value("--page").unwrap_or_else(|| "network".into()));
        }
        glib::ExitCode::SUCCESS
    });
    let slot = controller.clone();
    application.connect_activate(move |_| {
        if let Some(app) = slot.borrow().clone() {
            app.open_page("network");
        }
    });
    application.run();
    controller.borrow_mut().take();
}
impl App {
    pub fn parent(&self) -> Option<gtk::Window> {
        self.quick
            .borrow()
            .as_ref()
            .map(|w| w.clone().upcast())
            .or_else(|| self.full.borrow().as_ref().map(|w| w.clone().upcast()))
    }
    fn connect(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move{
        let Some(app)=weak.upgrade()else{return;};
        match gio::bus_get_future(gio::BusType::Session).await{
            Ok(connection)=>{
                let weak=Rc::downgrade(&app);connection.signal_subscribe(Some(SESSION_NAME),Some(SESSION_NAME),Some("Changed"),Some(SESSION_PATH),None,gio::DBusSignalFlags::NONE,move|_,_,_,_,_,parameters|{
                    if let Some(app)=weak.upgrade(){if let Some((raw,))=parameters.get::<(String,)>(){app.update(&raw);}}
                });
                match bus::session("GetState",None).await{Ok(raw)=>{app.update(&raw);app.execute(Action::Refresh,None);},Err(_)=>app.error("The settings service could not start. Check the session D-Bus and service package.")}
            }
            Err(_)=>app.error("No user D-Bus session is available"),
        }
    });
    }
    fn update(self: &Rc<Self>, raw: &str) {
        let Ok(state) = serde_json::from_str::<Snapshot>(raw) else {
            return;
        };
        self.state.replace(state.clone());
        let first = !self.loaded.replace(true);
        if first {
            let page = self
                .stack
                .borrow()
                .as_ref()
                .and_then(|s| s.visible_child_name());
            if let Some(page) = page {
                self.build_page(&page);
            }
        }
        for bindings in [&self.full_bindings, &self.quick_bindings] {
            let mut previous = std::mem::take(&mut *bindings.borrow_mut());
            previous.retain(|callback| callback(&state));
            bindings.borrow_mut().extend(previous);
        }
        self.error(state.error.as_deref().unwrap_or_default());
        if state.network.authentication.is_some() {
            self.show_auth();
        } else {
            self.auth_id.set(0);
            if let Some(area) = &*self.quick_auth.borrow() {
                widgets::clear(area);
            }
            if let Some(window) = self.auth_window.borrow_mut().take() {
                window.destroy();
            }
        }
    }
    pub fn bind_for(
        self: &Rc<Self>,
        surface: Surface,
        widget: &impl IsA<gtk::Widget>,
        callback: impl Fn(&Snapshot) + 'static,
    ) {
        callback(&self.state.borrow());
        let target = widget.as_ref().downgrade();
        let callback: Binding = Box::new(move |s| {
            if target.upgrade().is_none() {
                return false;
            }
            callback(s);
            true
        });
        match surface {
            Surface::Full => self.full_bindings.borrow_mut().push(callback),
            Surface::Quick => self.quick_bindings.borrow_mut().push(callback),
        };
    }
    pub fn error(&self, message: &str) {
        let mut message = message;
        while message.starts_with("GDBus.Error:") {
            if let Some((_, text)) = message.split_once(": ") {
                message = text;
            } else {
                break;
            }
        }
        self.banner.borrow_mut().retain(|target| {
            if let Some(label) = target.upgrade() {
                label.set_text(message);
                label.set_visible(!message.is_empty());
                true
            } else {
                false
            }
        });
    }
    pub fn execute(self: &Rc<Self>, action: Action, button: Option<gtk::Widget>) {
        if let Some(widget) = &button {
            widget.set_sensitive(false);
        }
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let result = bus::execute(&action).await;
            if let Some(widget) = button {
                widget.set_sensitive(true);
            }
            if let Some(app) = weak.upgrade() {
                match result {
                    Err(error) => app.error(&error),
                    Ok(value) => {
                        if matches!(action, Action::ApplyDisplays { .. }) {
                            if let Some(id) = value.get("id").and_then(|v| v.as_u64()) {
                                pages::display_confirmation(&app, id);
                            }
                        }
                    }
                }
            }
        });
    }
    pub fn open_page(self: &Rc<Self>, page: &str) {
        let page = if PAGES.iter().any(|p| p.0 == page) {
            page
        } else {
            "network"
        };
        if self.full.borrow().is_none() {
            self.create_full();
        }
        if let Some(stack) = &*self.stack.borrow() {
            stack.set_visible_child_name(page);
        }
        if let Some(window) = &*self.full.borrow() {
            window.present();
        }
    }
    fn create_full(self: &Rc<Self>) {
        let window = gtk::ApplicationWindow::builder()
            .application(&self.application)
            .title("Settings")
            .default_width(880)
            .default_height(640)
            .icon_name("org.uflinux.Settings")
            .build();
        window.add_css_class("uflinux-settings");
        let root = widgets::column();
        root.set_spacing(0);
        let header = gtk::HeaderBar::new();
        let nav = gtk::ToggleButton::builder()
            .icon_name("sidebar-show-symbolic")
            .active(true)
            .tooltip_text("Show or hide settings navigation")
            .build();
        header.pack_start(&nav);
        window.set_titlebar(Some(&header));
        let banner = widgets::description("");
        banner.add_css_class("error");
        banner.set_visible(false);
        root.append(&banner);
        self.banner.borrow_mut().push(banner.downgrade());
        let body = gtk::Paned::new(gtk::Orientation::Horizontal);
        body.set_vexpand(true);
        body.set_resize_start_child(false);
        body.set_shrink_start_child(false);
        let sidebar = widgets::column();
        sidebar.set_width_request(180);
        sidebar.add_css_class("sidebar");
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Search settings"));
        sidebar.append(&search);
        let stack = gtk::Stack::new();
        stack.set_hexpand(true);
        stack.set_vexpand(true);
        stack.set_transition_type(gtk::StackTransitionType::None);
        for (id, name) in PAGES {
            let placeholder = widgets::column();
            placeholder.set_visible(true);
            stack.add_titled(&placeholder, Some(id), name);
            stack.page(&placeholder).set_visible(true);
        }
        let navigation = gtk::StackSidebar::new();
        navigation.set_stack(&stack);
        navigation.set_vexpand(true);
        sidebar.append(&navigation);
        let target = sidebar.downgrade();
        nav.connect_toggled(move |button| {
            if let Some(sidebar) = target.upgrade() {
                sidebar.set_visible(button.is_active());
            }
        });
        body.set_start_child(Some(&sidebar));
        body.set_end_child(Some(&stack));
        body.set_position(200);
        root.append(&body);
        window.set_child(Some(&root));
        self.stack.replace(Some(stack.clone()));
        self.full.replace(Some(window.clone()));
        let weak = Rc::downgrade(self);
        stack.connect_visible_child_name_notify(move |stack| {
            if let Some(app) = weak.upgrade() {
                if let Some(page) = stack.visible_child_name() {
                    app.build_page(&page);
                }
            }
        });
        let weak = Rc::downgrade(self);
        search.connect_search_changed(move |search| {
            if let Some(app) = weak.upgrade() {
                let query = search.text().to_lowercase();
                if !query.is_empty() {
                    if let Some((id, _)) = PAGES.iter().find(|(id, name)| {
                        let keywords = match *id {
                            "network" => "wifi wi-fi ethernet internet dns proxy routes addressing",
                            "displays" => "screen resolution refresh scale rotation brightness hdr vrr monitors",
                            "audio" => "sound volume microphone speakers headphones mute",
                            "power" => "battery energy power saver balanced performance suspend sleep button shutdown tlp",
                            "default-apps" => "browser mail email files documents music video associations",
                            "datetime" => "clock timezone calendar ntp synchronization",
                            "appearance" => "wallpaper background picture color",
                            "about" => "system information memory processor cpu kernel version storage graphics",
                            _ => "",
                        };
                        name.to_lowercase().contains(&query) || id.contains(&query) || keywords.contains(&query)
                    }) {
                        app.open_page(id);
                    }
                }
            }
        });
        let weak = Rc::downgrade(self);
        window.connect_close_request(move |_| {
            if let Some(app) = weak.upgrade() {
                app.full.borrow_mut().take();
                app.stack.borrow_mut().take();
                app.full_bindings.borrow_mut().clear();
            }
            glib::Propagation::Proceed
        });
        self.build_page("network");
        self.report_first_frame(&window);
    }
    fn build_page(self: &Rc<Self>, page: &str) {
        self.full_bindings.borrow_mut().clear();
        let Some(stack) = self.stack.borrow().clone() else {
            return;
        };
        let Some(container) = stack.child_by_name(page).and_downcast::<gtk::Box>() else {
            return;
        };
        // Destroy inactive page widgets and signal bindings rather than retaining hidden pages.
        for (id, _) in PAGES {
            if let Some(old) = stack.child_by_name(id).and_downcast::<gtk::Box>() {
                widgets::clear(&old);
            }
        }
        let content = widgets::column();
        content.set_margin_start(24);
        content.set_margin_end(24);
        content.set_margin_top(20);
        content.set_margin_bottom(20);
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .hexpand(true)
            .child(&content)
            .build();
        container.append(&scroller);
        if !self.loaded.get() {
            content.append(&widgets::description("Loading settings…"));
            return;
        }
        pages::build(self, page, &content);
    }
    pub fn open_quick(self: &Rc<Self>, output: String) {
        let previous = self.quick.borrow_mut().take();
        if let Some(window) = previous {
            window.close();
            if *self.quick_output.borrow() == output {
                return;
            }
        }
        self.quick_bindings.borrow_mut().clear();
        self.quick_output.replace(output.clone());
        let window = gtk::ApplicationWindow::builder()
            .application(&self.application)
            .title("Quick settings")
            .default_width(400)
            .resizable(false)
            .decorated(false)
            .build();
        window.add_css_class("uflinux-settings");
        if gtk4_layer_shell::is_supported() {
            window.init_layer_shell();
            window.set_layer(Layer::Overlay);
            window.set_anchor(Edge::Bottom, true);
            window.set_anchor(Edge::Right, true);
            window.set_margin(Edge::Bottom, 48);
            window.set_margin(Edge::Right, 8);
            window.set_keyboard_mode(KeyboardMode::OnDemand);
            if let Some(display) = gtk::gdk::Display::default() {
                let monitors = display.monitors();
                for index in 0..monitors.n_items() {
                    if let Some(monitor) = monitors.item(index).and_downcast::<gtk::gdk::Monitor>()
                    {
                        if monitor.connector().as_deref() == Some(output.as_str()) {
                            window.set_monitor(Some(&monitor));
                            break;
                        }
                    }
                }
            }
        }
        let content = widgets::column();
        content.add_css_class("quick");
        let banner = widgets::description("");
        banner.set_visible(false);
        banner.add_css_class("error");
        content.append(&banner);
        self.banner.borrow_mut().push(banner.downgrade());
        pages::quick(self, &content);
        let selected_monitor = if gtk4_layer_shell::is_supported() {
            window.monitor()
        } else {
            None
        };
        let available_height = selected_monitor
            .or_else(|| {
                gtk::gdk::Display::default()
                    .and_then(|d| d.monitors().item(0))
                    .and_downcast::<gtk::gdk::Monitor>()
            })
            .map(|m| m.geometry().height() - 80)
            .unwrap_or(560)
            .max(240);
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .min_content_height(240)
            .max_content_height(available_height)
            .propagate_natural_height(true)
            .child(&content)
            .build();
        window.set_child(Some(&scroll));
        let keys = gtk::EventControllerKey::new();
        let target = window.downgrade();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                if let Some(window) = target.upgrade() {
                    window.close();
                }
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        window.add_controller(keys);
        let active = Rc::new(Cell::new(false));
        let closing = Rc::new(Cell::new(false));
        let dismissing = closing.clone();
        let target = window.downgrade();
        window.connect_is_active_notify(move |window| {
            if window.is_active() {
                active.set(true);
            } else if active.get() && !dismissing.get() {
                // GtkWindow destruction hides the window and can emit this
                // notification. Never recursively close it from its hide path.
                let target = target.clone();
                let closing = dismissing.clone();
                glib::idle_add_local_once(move || {
                    if let Some(window) = target.upgrade() {
                        if !closing.get() && !window.is_active() && window.is_visible() {
                            window.close();
                        }
                    }
                });
            }
        });
        let weak = Rc::downgrade(self);
        window.connect_close_request(move |_| {
            closing.set(true);
            if let Some(app) = weak.upgrade() {
                app.quick.borrow_mut().take();
                app.quick_auth.borrow_mut().take();
                app.quick_bindings.borrow_mut().clear();
            }
            glib::Propagation::Proceed
        });
        self.quick.replace(Some(window.clone()));
        window.present();
        self.report_first_frame(&window);
        self.execute(
            Action::System {
                operation: SystemAction::DdcDiscover,
            },
            None,
        );
    }
    fn report_first_frame(self: &Rc<Self>, window: &gtk::ApplicationWindow) {
        let began = if self.first_frame.replace(false) {
            self.started
        } else {
            self.activation.get()
        };
        let app = Rc::downgrade(self);
        window.add_tick_callback(move |window, _| {
            if !app.upgrade().is_some_and(|app| app.loaded.get()) {
                return glib::ControlFlow::Continue;
            }
            if let Some(clock) = window.frame_clock() {
                let token = Rc::new(RefCell::new(None));
                let target = token.clone();
                let id = clock.connect_after_paint(move |clock| {
                    if std::env::var_os("UFLINUX_SETTINGS_BENCH").is_some() {
                        eprintln!("UFLINUX_FIRST_FRAME_MS={}", began.elapsed().as_millis());
                    }
                    if let Some(id) = target.borrow_mut().take() {
                        clock.disconnect(id);
                    }
                });
                token.replace(Some(id));
            }
            glib::ControlFlow::Break
        });
    }
    pub fn show_auth(self: &Rc<Self>) {
        let Some(request) = self.state.borrow().network.authentication.clone() else {
            return;
        };
        if self.auth_id.replace(request.id) == request.id {
            return;
        }
        pages::network_auth(self, request);
    }
}
