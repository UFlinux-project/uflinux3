use crate::{auth, menu::Menu};
use glib::variant::ToVariant;
use gtk::prelude::*;
use std::{cell::RefCell, rc::Rc, time::Instant};

pub struct App {
    pub application: gtk::Application,
    pub agent: RefCell<Option<auth::Agent>>,
    pub authorization_active: std::cell::Cell<bool>,
    menu: RefCell<Option<Rc<Menu>>>,
    activation: RefCell<Instant>,
}

pub fn run() {
    let application = gtk::Application::builder()
        .application_id("org.uflinux.StartMenu")
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    let controller = Rc::new(RefCell::new(None::<Rc<App>>));
    let slot = controller.clone();
    application.connect_startup(move |application| {
        let css = gtk::CssProvider::new();
        css.load_from_string(&format!(
            "{}\n{}",
            crate::popup::CSS,
            include_str!("style.css")
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
            activation: RefCell::new(Instant::now()),
        });
        let status = gio::SimpleAction::new_stateful(
            "status",
            None,
            &"ready=false visible=false".to_variant(),
        );
        status.connect_change_state(|_, _| {});
        application.add_action(&status);
        app.menu.replace(Some(Menu::new(&app, String::new())));
        let action = gio::SimpleAction::new("hide", None);
        let weak = Rc::downgrade(&app);
        action.connect_activate(move |_, _| {
            if let Some(app) = weak.upgrade() {
                app.hide();
            }
        });
        application.add_action(&action);
        let action = gio::SimpleAction::new("toggle", Some(glib::VariantTy::STRING));
        let weak = Rc::downgrade(&app);
        action.connect_activate(move |_, value| {
            if let Some(app) = weak.upgrade() {
                app.toggle(value.and_then(|v| v.str()).unwrap_or_default());
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
    // Closing/hiding the menu never releases the resident process.
    let resident = application.hold();
    application.run();
    if let Some(app) = controller.borrow_mut().take() {
        if let Some(menu) = app.menu.borrow_mut().take() {
            menu.shutdown();
        }
    }
    drop(resident);
}

impl App {
    pub fn catalog_changed(&self, _catalog: &crate::catalog::Catalog) {}
    fn toggle(&self, output: &str) {
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
        menu.set_output(output);
        menu.present();
    }
    pub fn hide(&self) {
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
            action.set_state(
                &format!(
                    "ready={} visible={}",
                    menu.ready(),
                    menu.window.is_visible()
                )
                .to_variant(),
            );
        }
    }
    pub fn frame_start(&self) -> Instant {
        *self.activation.borrow()
    }
    pub fn parent(&self) -> Option<gtk::Window> {
        self.menu
            .borrow()
            .as_ref()
            .map(|menu| menu.window.clone().upcast())
    }
    pub fn error(&self, message: &str) {
        if let Some(menu) = self.menu.borrow().as_ref() {
            menu.failure(message);
        }
    }
    pub fn ensure_agent(self: &Rc<Self>) {
        if self.agent.borrow().is_none() {
            self.agent
                .replace(Some(auth::Agent::register(Rc::downgrade(self))));
        }
    }
    pub fn release_agent_if_desktop(&self) {}
    pub fn release_agent(&self) {
        self.agent.borrow_mut().take();
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
