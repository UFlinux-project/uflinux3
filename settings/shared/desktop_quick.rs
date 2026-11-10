// Included in app.rs so the reusable quick controls can share its state model.
struct DesktopQuick {
    window: gtk::ApplicationWindow,
    card: gtk::Box,
    scroll: gtk::ScrolledWindow,
}

impl App {
    /// Re-discover external brightness controls only when outputs change.
    pub fn desktop_outputs_changed(self: &Rc<Self>) {
        if !self.desktop_mode {
            return;
        }
        self.desktop_ddc_discovered.set(false);
        if self.quick_window().is_some() && *self.quick_panel.borrow() == "power" {
            self.desktop_ddc_discovered.set(true);
            self.execute(
                Action::System {
                    operation: SystemAction::DdcDiscover,
                },
                None,
            );
        }
    }

    /// Build one hidden shell without waiting for the Settings service.
    pub fn prepare_desktop_quick(self: &Rc<Self>, panel: &str) -> Option<gtk::ApplicationWindow> {
        if !self.desktop_mode || !["network", "sound", "power"].contains(&panel) {
            return None;
        }
        if let Some(existing) = self.desktop_quicks.borrow().get(panel) {
            return Some(existing.window.clone());
        }
        let window = gtk::ApplicationWindow::builder()
            .application(&self.application)
            .title(match panel {
                "sound" => "Sound",
                "power" => "Power",
                _ => "Network",
            })
            .resizable(false)
            .decorated(false)
            .build();
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
        card.append(&scroll);
        root.add_overlay(&card);
        window.set_child(Some(&root));
        if gtk4_layer_shell::is_supported() {
            window.connect_realize(|window| {
                if let Some(surface) = window.surface() {
                    surface.connect_layout(|surface, width, height| {
                        crate::popup::set_input_region(surface, width, height, 0)
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
        let weak = Rc::downgrade(self);
        window.connect_close_request(move |_| {
            if let Some(app) = weak.upgrade() {
                app.hide_quick();
            }
            glib::Propagation::Stop
        });
        let weak = Rc::downgrade(self);
        let active = Rc::new(Cell::new(false));
        window.connect_is_active_notify(move |window| {
            if window.is_active() {
                active.set(true);
                return;
            }
            if !active.get() || !window.is_visible() {
                return;
            }
            let weak = weak.clone();
            let target = window.downgrade();
            glib::idle_add_local_once(move || {
                if let (Some(app), Some(window)) = (weak.upgrade(), target.upgrade()) {
                    if window.is_visible()
                        && !window.is_active()
                        && !app.submenu_open.get()
                        && !app.authorization_active.get()
                    {
                        window.close();
                    }
                }
            });
        });
        self.desktop_quicks.borrow_mut().insert(
            panel.to_string(),
            DesktopQuick {
                window: window.clone(),
                card,
                scroll,
            },
        );
        Some(window)
    }

    fn open_desktop_quick(self: &Rc<Self>, output: String, panel: &str) {
        let same = self.quick_window().is_some()
            && *self.quick_output.borrow() == output
            && *self.quick_panel.borrow() == panel;
        self.hide_quick();
        if same {
            return;
        }
        let Some(window) = self.prepare_desktop_quick(panel) else {
            return;
        };
        self.quick_output.replace(output.clone());
        self.quick_panel.replace(panel.to_string());
        if let Some(monitor) = crate::popup::select_monitor(&window, &output) {
            let geometry = monitor.geometry();
            if let Some(quick) = self.desktop_quicks.borrow().get(panel) {
                quick
                    .card
                    .set_size_request((geometry.width() - 16).max(200).min(400), -1);
                quick
                    .scroll
                    .set_max_content_height((geometry.height() - 88).max(136));
            }
            let weak = Rc::downgrade(self);
            let id = monitor.connect_invalidate(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.hide_quick();
                }
            });
            self.desktop_monitor.replace(Some((monitor, id)));
        }
        self.quick.replace(Some(window.clone()));
        window.present();
        self.report_first_frame(&window);
        if panel == "network" && self.state.borrow().network.authentication.is_some() {
            self.show_auth();
        }
        if panel == "power" && !self.desktop_ddc_discovered.replace(true) {
            self.execute(
                Action::System {
                    operation: SystemAction::DdcDiscover,
                },
                None,
            );
        }
    }
}
