use crate::{auth, menu::Menu, panel::Panel};
use glib::variant::ToVariant;
use gtk::prelude::*;
use std::{cell::RefCell, rc::Rc, time::Instant};

pub struct App {
    pub application: gtk::Application,
    pub agent: RefCell<Option<auth::Agent>>,
    pub authorization_active: std::cell::Cell<bool>,
    menu: RefCell<Option<Rc<Menu>>>,
    panel: RefCell<Option<Rc<Panel>>>,
    pub quick: Rc<uflinux_settings::app::App>,
    activation: RefCell<Instant>,
    output: RefCell<String>,
    clock_output: RefCell<Option<String>>,
    tray_output: RefCell<Option<String>>,
    task_output: RefCell<Option<String>>,
    quick_settings_ready: std::cell::Cell<bool>,
    quick_prepared: RefCell<std::collections::HashSet<String>>,
}

pub fn run() {
    let application = gtk::Application::builder()
        .application_id("org.uflinux.Desktop")
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    let controller = Rc::new(RefCell::new(None::<Rc<App>>));
    let resident = Rc::new(RefCell::new(None));
    let hold = resident.clone();
    let slot = controller.clone();
    application.connect_startup(move |application| {
        hold.replace(Some(application.hold()));
        let css = gtk::CssProvider::new();
        css.load_from_string(&format!(
            "{}\n{}\n{}",
            crate::popup::CSS,
            include_str!("../../start-menu/src/style.css"),
            include_str!("panel.css")
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
            agent: RefCell::new(None),
            authorization_active: std::cell::Cell::new(false),
            menu: RefCell::new(None),
            panel: RefCell::new(None),
            quick: uflinux_settings::app::App::for_desktop(application),
            activation: RefCell::new(Instant::now()),
            output: RefCell::new(String::new()),
            clock_output: RefCell::new(None),
            tray_output: RefCell::new(None),
            task_output: RefCell::new(None),
            quick_settings_ready: std::cell::Cell::new(false),
            quick_prepared: RefCell::new(std::collections::HashSet::new()),
        });
        let status = gio::SimpleAction::new_stateful(
            "status",
            None,
            &"{\"ready\":false,\"popup\":null,\"output\":null}".to_variant(),
        );
        status.connect_change_state(|_, _| {});
        application.add_action(&status);
        app.menu.replace(Some(Menu::new(&app, String::new())));
        app.panel.replace(Some(Panel::new(&app)));
        app.ensure_agent();
        let action = gio::SimpleAction::new("hide", None);
        let weak = Rc::downgrade(&app);
        action.connect_activate(move |_, _| {
            if let Some(app) = weak.upgrade() {
                app.hide();
            }
        });
        application.add_action(&action);
        let action = gio::SimpleAction::new("start-menu", Some(glib::VariantTy::STRING));
        let weak = Rc::downgrade(&app);
        action.connect_activate(move |_, value| {
            if let Some(app) = weak.upgrade() {
                app.toggle(value.and_then(|v| v.str()).unwrap_or_default());
            }
        });
        application.add_action(&action);
        let action = gio::SimpleAction::new("panel", Some(glib::VariantTy::new("(ss)").unwrap()));
        let weak = Rc::downgrade(&app);
        action.connect_activate(move |_, value| {
            if let (Some(app), Some((panel, output))) = (
                weak.upgrade(),
                value.and_then(|v| v.get::<(String, String)>()),
            ) {
                app.open_quick(&panel, &output);
            }
        });
        application.add_action(&action);
        let action = gio::SimpleAction::new("quit", None);
        let weak = application.downgrade();
        action.connect_activate(move |_, _| {
            if let Some(application) = weak.upgrade() {
                application.quit();
            }
        });
        application.add_action(&action);
        let weak = Rc::downgrade(&app);
        let mut index = 0;
        glib::idle_add_local(move || {
            let Some(app) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let panels = ["network", "sound", "power"];
            app.prepare_quick(panels[index]);
            index += 1;
            if index == panels.len() {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
        slot.replace(Some(app));
    });
    let slot = controller.clone();
    application.connect_command_line(move |application, line| {
        let Some(app) = slot.borrow().clone() else {
            return glib::ExitCode::FAILURE;
        };
        let args: Vec<_> = line
            .arguments()
            .iter()
            .map(|v| v.to_string_lossy().into_owned())
            .collect();
        if args.iter().any(|arg| arg == "--quit") {
            application.quit();
        } else if args.iter().any(|arg| arg == "--hide") {
            app.hide();
        } else if let Some(panel) = args
            .iter()
            .position(|arg| arg == "--panel")
            .and_then(|i| args.get(i + 1))
        {
            let output = args
                .iter()
                .position(|arg| arg == "--output")
                .and_then(|i| args.get(i + 1))
                .map(String::as_str)
                .unwrap_or("");
            if !["network", "sound", "power"].contains(&panel.as_str()) {
                return glib::ExitCode::FAILURE;
            }
            app.open_quick(panel, output);
        } else if !args.iter().any(|arg| arg == "--daemon") {
            let output = args
                .iter()
                .position(|arg| arg == "--output")
                .and_then(|index| args.get(index + 1))
                .cloned()
                .unwrap_or_default();
            app.toggle(&output);
        }
        glib::ExitCode::SUCCESS
    });
    let slot = controller.clone();
    application.connect_activate(move |_| {
        if let Some(app) = slot.borrow().as_ref() {
            app.toggle("");
        }
    });
    // Only the primary process holds the application; forwarding clients exit.
    application.run();
    if let Some(app) = controller.borrow_mut().take() {
        if let Some(menu) = app.menu.borrow_mut().take() {
            menu.shutdown();
        }
    }
    resident.borrow_mut().take();
}

impl App {
    pub fn catalog_changed(&self, catalog: &crate::catalog::Catalog) {
        if let Some(panel) = self.panel.borrow().as_ref() {
            panel.set_catalog(catalog);
        }
    }
    pub fn toggle(&self, output: &str) {
        if let Some(panel) = self.panel.borrow().as_ref() {
            panel.hide_task_menus(None);
            panel.hide_clocks(None);
            panel.hide_trays(None);
        }
        self.quick.hide_quick();
        let slot = self.menu.borrow();
        let Some(menu) = slot.as_ref() else { return };
        if menu.window.is_visible() && menu.matches_output(output) {
            menu.hide();
            return;
        }
        *self.activation.borrow_mut() = Instant::now();
        remote_action(
            "org.uflinux.Settings",
            "/org/uflinux/Settings",
            "close-quick",
        );
        self.output.replace(output.to_string());
        menu.set_output(output);
        menu.present();
    }
    fn prepare_quick(self: &Rc<Self>, panel: &str) {
        if self.quick_prepared.borrow().contains(panel) {
            return;
        }
        if let Some(window) = self.quick.prepare_desktop_quick(panel) {
            self.quick_prepared.borrow_mut().insert(panel.to_string());
            let weak = Rc::downgrade(self);
            window.connect_visible_notify(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.report_status();
                }
            });
            self.quick_settings_ready
                .set(self.quick_prepared.borrow().len() == 3);
            self.report_status();
        }
    }
    pub fn open_quick(self: &Rc<Self>, panel: &str, output: &str) {
        if !["network", "sound", "power"].contains(&panel) {
            return;
        }
        self.prepare_quick(panel);
        if let Some(panel) = self.panel.borrow().as_ref() {
            panel.hide_task_menus(None);
            panel.hide_clocks(None);
            panel.hide_trays(None);
        }
        if let Some(menu) = self.menu.borrow().as_ref() {
            menu.hide();
        }
        self.quick.open_quick(output.to_string(), panel);
        self.report_status();
    }
    pub fn show_clock(&self, clock: &gtk::Popover, output: &str) {
        if let Some(panel) = self.panel.borrow().as_ref() {
            panel.hide_task_menus(None);
            panel.hide_clocks(Some(clock));
            panel.hide_trays(None);
        }
        self.quick.hide_quick();
        if let Some(menu) = self.menu.borrow().as_ref() {
            menu.hide();
        }
        self.clock_output.replace(Some(output.to_string()));
        self.report_status();
    }
    pub fn show_tray(&self, tray: &gtk::Popover, output: &str) {
        if let Some(panel) = self.panel.borrow().as_ref() {
            panel.hide_task_menus(None);
            panel.hide_clocks(None);
            panel.hide_trays(Some(tray));
        }
        self.quick.hide_quick();
        if let Some(menu) = self.menu.borrow().as_ref() {
            menu.hide();
        }
        self.tray_output.replace(Some(output.to_string()));
        self.report_status();
    }
    pub fn tray_closed(&self) {
        self.tray_output.replace(None);
        self.report_status();
    }
    pub fn task_opened(&self, output: &str) {
        self.task_output.replace(Some(output.to_string()));
        self.report_status();
    }
    pub fn task_closed(&self) {
        self.task_output.replace(None);
        self.report_status();
    }
    pub fn clock_closed(&self) {
        self.clock_output.replace(None);
        self.report_status();
    }
    pub fn hide(&self) {
        if let Some(panel) = self.panel.borrow().as_ref() {
            panel.hide_task_menus(None);
            panel.hide_clocks(None);
            panel.hide_trays(None);
        }
        self.quick.hide_quick();
        if let Some(menu) = self.menu.borrow().as_ref() {
            menu.hide();
        }
    }
    pub fn report_status(&self) {
        if let (Some(menu), Some(action)) = (
            self.menu.borrow().as_ref(),
            self.application
                .lookup_action("status")
                .and_downcast::<gio::SimpleAction>(),
        ) {
            let visible = menu.window.is_visible();
            let quick = self.quick.quick_details();
            let clock = self.clock_output.borrow().clone();
            let tray = self.tray_output.borrow().clone();
            let task = self.task_output.borrow().clone();
            let popup = if visible {
                Some("start".to_string())
            } else if let Some(quick) = quick.as_ref() {
                Some(quick.0.clone())
            } else {
                clock
                    .as_ref()
                    .map(|_| "clock".to_string())
                    .or_else(|| tray.as_ref().map(|_| "tray".to_string()))
                    .or_else(|| task.as_ref().map(|_| "windows".to_string()))
            };
            let output = if visible {
                self.output.borrow().clone()
            } else {
                quick
                    .map(|v| v.1)
                    .or(clock)
                    .or(tray)
                    .or(task)
                    .unwrap_or_default()
            };
            let output = if output.is_empty() {
                None
            } else {
                Some(output)
            };
            action.set_state(
                &serde_json::json!({
                "ready": menu.ready() && self.quick_settings_ready.get(),
                    "popup": popup,
                    "output": output,
                })
                .to_string()
                .to_variant(),
            );
        }
    }
    pub fn frame_start(&self) -> Instant {
        *self.activation.borrow()
    }
    pub fn parent(&self) -> Option<gtk::Window> {
        self.quick
            .quick_window()
            .map(|window| window.upcast())
            .or_else(|| {
                self.menu
                    .borrow()
                    .as_ref()
                    .filter(|menu| menu.window.is_visible())
                    .map(|menu| menu.window.clone().upcast())
            })
    }
    pub fn error(&self, message: &str) {
        if self.quick.quick_window().is_some() {
            self.quick.error(message);
        } else if let Some(menu) = self.menu.borrow().as_ref() {
            menu.failure(message);
        }
    }
    // The desktop owns the agent for the whole session.
    pub fn release_agent(&self) {}
    pub fn ensure_agent(self: &Rc<Self>) {
        if self.agent.borrow().is_none() {
            self.agent
                .replace(Some(auth::Agent::register(Rc::downgrade(self))));
        }
    }
    pub fn open_page(&self, page: &str) {
        match std::process::Command::new("uflinux-settings")
            .args(["--page", page])
            .spawn()
        {
            Ok(mut child) => {
                self.hide();
                // Reap the GApplication forwarding process without blocking GTK.
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
            }
            Err(error) => self.error(&format!("Could not open Settings: {error}")),
        }
    }
}

fn remote_action(name: &'static str, path: &'static str, action: &'static str) {
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
