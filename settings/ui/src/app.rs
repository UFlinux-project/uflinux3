use crate::{auth, pages, widgets};
use glib::variant::ToVariant;
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
    pub authorization_active: Cell<bool>,
    pub submenu_open: Cell<bool>,
    pub submenu_epoch: Cell<u64>,
    full: RefCell<Option<gtk::ApplicationWindow>>,
    quick: RefCell<Option<gtk::ApplicationWindow>>,
    pub stack: RefCell<Option<gtk::Stack>>,
    full_bindings: RefCell<Vec<Binding>>,
    quick_bindings: RefCell<Vec<Binding>>,
    banner: RefCell<Vec<glib::WeakRef<gtk::Label>>>,
    quick_output: RefCell<String>,
    quick_panel: RefCell<String>,
    started: Instant,
    auth_id: Cell<u64>,
    pub quick_auth: RefCell<Option<gtk::Box>>,
    pub auth_window: RefCell<Option<gtk::ApplicationWindow>>,
    activation: Cell<Instant>,
    first_frame: Cell<bool>,
    loaded: Cell<bool>,
    desktop_mode: bool,
    desktop_quicks: RefCell<std::collections::HashMap<String, DesktopQuick>>,
    desktop_monitor: RefCell<Option<(gtk::gdk::Monitor, glib::SignalHandlerId)>>,
    desktop_ddc_discovered: Cell<bool>,
    /// Set by the host process that owns the authorization agent. In desktop
    /// mode that process registers the agent only once something needs it.
    pub agent_request: RefCell<Option<Rc<dyn Fn()>>>,
}
pub const PAGES: &[(&str, &str)] = &[
    ("network", "Network"),
    ("displays", "Displays"),
    ("audio", "Sound"),
    ("power", "Power"),
    ("default-apps", "Default Applications"),
    ("datetime", "Date & Time"),
    ("appearance", "Appearance"),
    ("screen-capture", "Screen Capture"),
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
        css.load_from_string(&format!(
            "{}\n{}",
            crate::popup::CSS,
            include_str!("../assets/style.css")
        ));
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
            authorization_active: Cell::new(false),
            submenu_open: Cell::new(false),
            submenu_epoch: Cell::new(0),
            full: RefCell::new(None),
            quick: RefCell::new(None),
            stack: RefCell::new(None),
            full_bindings: RefCell::new(Vec::new()),
            quick_bindings: RefCell::new(Vec::new()),
            banner: RefCell::new(Vec::new()),
            quick_output: RefCell::new(String::new()),
            quick_panel: RefCell::new(String::new()),
            started: began,
            auth_id: Cell::new(0),
            quick_auth: RefCell::new(None),
            auth_window: RefCell::new(None),
            activation: Cell::new(began),
            first_frame: Cell::new(true),
            loaded: Cell::new(false),
            desktop_mode: false,
            desktop_quicks: RefCell::new(std::collections::HashMap::new()),
            desktop_monitor: RefCell::new(None),
            desktop_ddc_discovered: Cell::new(false),
            agent_request: RefCell::new(None),
        });
        let action = gio::SimpleAction::new("close-quick", None);
        let weak = Rc::downgrade(&app);
        action.connect_activate(move |_, _| {
            if let Some(app) = weak.upgrade() {
                let window = app.quick.borrow_mut().take();
                if let Some(window) = window {
                    window.close();
                }
                app.quick_bindings.borrow_mut().clear();
            }
        });
        application.add_action(&action);
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
        if args.iter().any(|s| s == "--quick-settings") || value("--panel").is_some() {
            let panel = value("--panel").unwrap_or_else(|| "network".into());
            if !["network", "sound", "power"].contains(&panel.as_str()) {
                eprintln!("Unknown panel: {panel}");
                return glib::ExitCode::FAILURE;
            }
            app.open_quick(value("--output").unwrap_or_default(), &panel);
        } else if value("--network-auth").is_some() {
            if app.parent().is_none() {
                app.open_quick(String::new(), "network");
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
    /// Create quick controls inside the desktop's GTK process. Full Settings
    /// remains an independently launched application.
    pub fn for_desktop(application: &gtk::Application) -> Rc<Self> {
        let began = Instant::now();
        let css = gtk::CssProvider::new();
        css.load_from_string(include_str!("../assets/style.css"));
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &css,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
        let app = Rc::new(Self {
            application: application.clone(),
            state: RefCell::new(Snapshot::default()),
            agent: RefCell::new(None),
            authorization_active: Cell::new(false),
            submenu_open: Cell::new(false),
            submenu_epoch: Cell::new(0),
            full: RefCell::new(None),
            quick: RefCell::new(None),
            stack: RefCell::new(None),
            full_bindings: RefCell::new(Vec::new()),
            quick_bindings: RefCell::new(Vec::new()),
            banner: RefCell::new(Vec::new()),
            quick_output: RefCell::new(String::new()),
            quick_panel: RefCell::new(String::new()),
            started: began,
            auth_id: Cell::new(0),
            quick_auth: RefCell::new(None),
            auth_window: RefCell::new(None),
            activation: Cell::new(began),
            first_frame: Cell::new(true),
            loaded: Cell::new(false),
            desktop_mode: true,
            desktop_quicks: RefCell::new(std::collections::HashMap::new()),
            desktop_monitor: RefCell::new(None),
            desktop_ddc_discovered: Cell::new(false),
            agent_request: RefCell::new(None),
        });
        app.connect();
        app
    }

    /// Ask the host process to register an authorization agent. Standalone
    /// Settings activates the desktop service itself, so it has no hook.
    fn request_agent(&self) {
        if let Some(request) = self.agent_request.borrow().as_ref() {
            request();
        }
    }

    pub fn hide_quick(self: &Rc<Self>) {
        let window = self.quick.borrow_mut().take();
        if let Some(window) = window {
            if self.desktop_mode {
                window.set_visible(false);
                // Unmapping alone keeps the Wayland surface, the renderer and its
                // back buffer, so a hidden shell would hold a full-output
                // allocation for the rest of the session. Releasing the surface
                // drops all three; the shell realizes again when it is presented.
                crate::popup::release_window_surface(&window);
                if *self.quick_panel.borrow() == "network" {
                    let request = self
                        .state
                        .borrow()
                        .network
                        .authentication
                        .as_ref()
                        .map(|request| request.id);
                    if let Some(id) = request {
                        self.execute(Action::AuthCancel { id }, None);
                    }
                }
                if let Some(area) = self.quick_auth.borrow().as_ref() {
                    widgets::clear(area);
                }
                self.auth_id.set(0);
                self.submenu_open.set(false);
                self.submenu_epoch
                    .set(self.submenu_epoch.get().wrapping_add(1));
                if let Some((monitor, id)) = self.desktop_monitor.borrow_mut().take() {
                    monitor.disconnect(id);
                }
            } else {
                window.close();
            }
        }
    }

    pub fn quick_window(&self) -> Option<gtk::ApplicationWindow> {
        self.quick
            .borrow()
            .as_ref()
            .filter(|w| w.is_visible())
            .cloned()
    }

    pub fn quick_details(&self) -> Option<(String, String)> {
        self.quick_window().map(|_| {
            (
                self.quick_panel.borrow().clone(),
                self.quick_output.borrow().clone(),
            )
        })
    }

    pub fn release_agent_if_desktop(&self) {}
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
                let weak = Rc::downgrade(&app);
                connection.signal_subscribe(Some("org.freedesktop.DBus"), Some("org.freedesktop.DBus"), Some("NameOwnerChanged"), Some("/org/freedesktop/DBus"), Some(SESSION_NAME), gio::DBusSignalFlags::NONE, move |_, _, _, _, _, parameters| {
                    let Some((name, _, owner)) = parameters.get::<(String, String, String)>() else { return };
                    if name != SESSION_NAME { return; }
                    if let Some(app) = weak.upgrade() {
                        if owner.is_empty() {
                            app.desktop_ddc_discovered.set(false);
                            if let Ok(raw) = serde_json::to_string(&Snapshot::default()) { app.update(&raw); }
                            app.error("The settings service is unavailable.");
                        } else {
                            let weak = Rc::downgrade(&app);
                            glib::MainContext::default().spawn_local(async move {
                                if let (Some(app), Ok(raw)) = (weak.upgrade(), bus::session("GetState", None).await) {
                                    app.update(&raw);
                                }
                            });
                        }
                    }
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
            // Answering the request can require polkit, so the agent has to be
            // registered before the passphrase is submitted.
            if self.desktop_mode {
                self.request_agent();
                self.show_auth();
            }
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
        let desktop_mode = self.desktop_mode;
        if desktop_mode {
            self.request_agent();
        }
        glib::MainContext::default().spawn_local(async move {
            let result = async {
                if !desktop_mode {
                    let connection = gio::bus_get_future(gio::BusType::Session)
                        .await
                        .map_err(|error| error.to_string())?;
                    connection
                        .call_future(
                            Some("org.freedesktop.DBus"),
                            "/org/freedesktop/DBus",
                            "org.freedesktop.DBus",
                            "StartServiceByName",
                            Some(&("org.uflinux.Desktop", 0u32).to_variant()),
                            None,
                            gio::DBusCallFlags::NONE,
                            5000,
                        )
                        .await
                        .map_err(|error| {
                            format!("Could not start desktop authorization agent: {error}")
                        })?;
                }
                bus::execute(&action).await
            }
            .await;
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
        if self.desktop_mode {
            self.hide_quick();
            match std::process::Command::new("uflinux-settings")
                .args(["--page", page])
                .spawn()
            {
                Ok(mut child) => {
                    std::thread::spawn(move || {
                        let _ = child.wait();
                    });
                }
                Err(error) => self.error(&format!("Could not open Settings: {error}")),
            }
            return;
        }
        let quick = self.quick.borrow_mut().take();
        if let Some(window) = quick {
            window.close();
        }
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
        self.close_start();
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
                            "datetime" => "clock timezone calendar sntp synchronization",
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
    fn close_start(&self) {
        if self.desktop_mode {
            return;
        }
        remote_action("org.uflinux.Desktop", "/org/uflinux/Desktop", "hide");
    }
    pub fn open_quick(self: &Rc<Self>, output: String, panel: &str) {
        if self.desktop_mode {
            self.open_desktop_quick(output, panel);
            return;
        }
        let previous = self.quick.borrow_mut().take();
        if let Some(window) = previous {
            window.close();
            if *self.quick_output.borrow() == output && *self.quick_panel.borrow() == panel {
                return;
            }
        }
        self.quick_bindings.borrow_mut().clear();
        self.quick_output.replace(output.clone());
        self.quick_panel.replace(panel.to_string());
        let window = gtk::ApplicationWindow::builder()
            .application(&self.application)
            .title(match panel {
                "sound" => "Sound",
                "power" => "Power",
                _ => "Network",
            })
            .default_width(400)
            .resizable(false)
            .decorated(false)
            .build();
        self.close_start();
        window.add_css_class("uflinux-settings");
        window.add_css_class("status-shell");
        window.connect_realize(|window| {
            if window
                .renderer()
                .is_some_and(|renderer| renderer.is::<gtk::gsk::CairoRenderer>())
            {
                window.add_css_class("software-renderer");
            }
        });
        if gtk4_layer_shell::is_supported() {
            window.set_resizable(true);
            window.set_default_size(0, 0);
            window.init_layer_shell();
            window.set_namespace(Some("uflinux-status-menu"));
            window.set_layer(Layer::Overlay);
            window.set_exclusive_zone(-1);
            for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
                window.set_anchor(edge, true);
            }
            window.set_keyboard_mode(KeyboardMode::OnDemand);
        }
        let monitor = crate::popup::select_monitor(&window, &output);
        let geometry = monitor.as_ref().map(|m| m.geometry());
        let available_height = geometry.map(|g| (g.height() - 64).max(160)).unwrap_or(560);
        let available_width = geometry.map(|g| (g.width() - 16).max(200)).unwrap_or(400);
        if let Some(monitor) = monitor {
            let target = window.downgrade();
            let id = monitor.connect_invalidate(move |_| {
                if let Some(window) = target.upgrade() {
                    window.close();
                }
            });
            let id = RefCell::new(Some(id));
            window.connect_close_request(move |_| {
                if let Some(id) = id.borrow_mut().take() {
                    monitor.disconnect(id);
                }
                glib::Propagation::Proceed
            });
        }
        let root = gtk::Overlay::new();
        let empty = gtk::Box::new(gtk::Orientation::Vertical, 0);
        empty.set_hexpand(true);
        empty.set_vexpand(true);
        root.set_child(Some(&empty));
        let content = widgets::column();
        content.add_css_class("quick");
        let banner = widgets::description("");
        banner.set_visible(false);
        banner.add_css_class("error");
        content.append(&banner);
        self.banner.borrow_mut().push(banner.downgrade());
        pages::quick(self, &content, panel);
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .max_content_height(available_height - 24)
            .propagate_natural_height(true)
            .child(&content)
            .build();
        let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
        card.add_css_class("uflinux-popup-card");
        card.set_halign(gtk::Align::End);
        card.set_valign(gtk::Align::End);
        card.set_margin_end(8);
        card.set_margin_start(8);
        card.set_margin_bottom(48);
        card.set_size_request(available_width.min(400), -1);
        card.append(&scroll);
        root.add_overlay(&card);
        window.set_child(Some(&root));
        if gtk4_layer_shell::is_supported() {
            window.connect_realize(|window| {
                if let Some(surface) = window.surface() {
                    surface.connect_layout(|surface, width, height| {
                        crate::popup::set_input_region(surface, width, height, 0);
                    });
                }
            });
            let gesture = gtk::GestureClick::new();
            gesture.set_propagation_phase(gtk::PropagationPhase::Capture);
            let target = window.downgrade();
            let target_card = card.downgrade();
            let target_root = root.downgrade();
            gesture.connect_pressed(move |gesture, _, x, y| {
                if let (Some(card), Some(root)) = (target_card.upgrade(), target_root.upgrade()) {
                    if card.compute_bounds(&root).is_some_and(|r| {
                        r.contains_point(&gtk::graphene::Point::new(x as f32, y as f32))
                    }) {
                        gesture.set_state(gtk::EventSequenceState::Denied);
                    } else {
                        gesture.set_state(gtk::EventSequenceState::Claimed);
                        if let Some(window) = target.upgrade() {
                            window.close();
                        }
                    }
                }
            });
            root.add_controller(gesture);
        }
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
        let application = Rc::downgrade(self);
        let closing = Rc::new(Cell::new(false));
        let dismissing = closing.clone();
        let target = window.downgrade();
        window.connect_is_active_notify(move |window| {
            if window.is_active() {
                active.set(true);
            } else if active.get()
                && !dismissing.get()
                && !application
                    .upgrade()
                    .is_some_and(|app| app.submenu_open.get())
            {
                // GtkWindow destruction hides the window and can emit this
                // notification. Never recursively close it from its hide path.
                let target = target.clone();
                let closing = dismissing.clone();
                let application = application.clone();
                glib::idle_add_local_once(move || {
                    if let Some(window) = target.upgrade() {
                        if !closing.get()
                            && !application
                                .upgrade()
                                .is_some_and(|app| app.authorization_active.get())
                            && !window.is_active()
                            && window.is_visible()
                        {
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
                app.submenu_open.set(false);
                app.submenu_epoch
                    .set(app.submenu_epoch.get().wrapping_add(1));
            }
            glib::Propagation::Proceed
        });
        self.quick.replace(Some(window.clone()));
        if panel == "network" && self.state.borrow().network.authentication.is_some() {
            self.auth_id.set(0);
            self.show_auth();
        }
        window.present();
        self.report_first_frame(&window);
        if panel == "power" {
            self.execute(
                Action::System {
                    operation: SystemAction::DdcDiscover,
                },
                None,
            );
        }
    }
    pub fn frame_start(&self) -> Instant {
        if self.first_frame.replace(false) {
            self.started
        } else {
            self.activation.get()
        }
    }
    fn report_first_frame(self: &Rc<Self>, window: &gtk::ApplicationWindow) {
        let began = self.frame_start();
        let panel = self.quick_panel.borrow().clone();
        let app = Rc::downgrade(self);
        window.add_tick_callback(move |window, _| {
            if !app.upgrade().is_some_and(|app| app.loaded.get()) {
                return glib::ControlFlow::Continue;
            }
            if let Some(clock) = window.frame_clock() {
                let token = Rc::new(RefCell::new(None));
                let target = token.clone();
                let panel = panel.clone();
                let id = clock.connect_after_paint(move |clock| {
                    if std::env::var_os("UFLINUX_SETTINGS_BENCH").is_some() {
                        eprintln!("UFLINUX_FIRST_FRAME_MS={}", began.elapsed().as_millis());
                        eprintln!(
                            "UFLINUX_QUICK_PAINT_US={} panel={}",
                            glib::monotonic_time(),
                            panel
                        );
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
        if self.desktop_mode
            && (self.quick_window().is_none() || *self.quick_panel.borrow() != "network")
        {
            self.application
                .activate_action("panel", Some(&("network", "").to_variant()));
        }
        pages::network_auth(self, request);
    }
}

// NO_AUTO_START avoids starting the other application just to dismiss it.
fn remote_action(name: &'static str, path: &'static str, action: &'static str) {
    use glib::variant::ToVariant;
    glib::MainContext::default().spawn_local(async move {
        if let Ok(bus) = gio::bus_get_future(gio::BusType::Session).await {
            let parameters = (
                action,
                Vec::<glib::Variant>::new(),
                std::collections::HashMap::<String, glib::Variant>::new(),
            )
                .to_variant();
            let _ = bus
                .call_future(
                    Some(name),
                    path,
                    "org.gtk.Actions",
                    "Activate",
                    Some(&parameters),
                    None,
                    gio::DBusCallFlags::NO_AUTO_START,
                    1000,
                )
                .await;
        }
    });
}

include!("../../shared/desktop_quick.rs");
