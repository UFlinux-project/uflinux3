use super::*;
use std::cell::{Cell, RefCell};

/// Compact controls for the status menu. Full Settings retains its detailed selectors.
pub fn quick(app: &Rc<App>, parent: &gtk::Box) {
    let unavailable = description("");
    parent.append(&unavailable);
    let target = unavailable.downgrade();
    app.bind_for(Surface::Quick, &unavailable, move |state| {
        if let Some(label) = target.upgrade() {
            label.set_visible(!state.audio.available);
            label.set_text("Audio is unavailable or still starting.");
        }
    });
    quick_device(app, parent, false);
    quick_device(app, parent, true);
    let streams = column();
    parent.append(&streams);
    let target = streams.downgrade();
    app.bind_for(Surface::Quick, &streams, move |state| {
        if let Some(streams) = target.upgrade() {
            streams.set_visible(!state.audio.streams.is_empty());
        }
    });
    applications(app, &streams, Surface::Quick);
}

fn quick_device(app: &Rc<App>, parent: &gtk::Box, input: bool) {
    let group = column();
    parent.append(&group);
    let target = group.downgrade();
    app.bind_for(Surface::Quick, &group, move |state| {
        if let Some(group) = target.upgrade() {
            group.set_visible(if input {
                !state.audio.inputs.is_empty()
            } else {
                !state.audio.outputs.is_empty()
            });
        }
    });
    let label = if input {
        "Input volume"
    } else {
        "Output volume"
    };
    section(&group, if input { "Input" } else { "Output" });
    let control = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    control.set_hexpand(true);
    let scale = slider_control(
        app,
        Surface::Quick,
        label,
        move |state| {
            let (devices, current) = if input {
                (&state.audio.inputs, &state.audio.default_input)
            } else {
                (&state.audio.outputs, &state.audio.default_output)
            };
            devices
                .iter()
                .find(|device| &device.name == current)
                .map(|device| device.volume)
                .unwrap_or(0.)
        },
        move |percent| Action::SetVolume {
            input,
            device: String::new(),
            percent,
        },
    );
    control.append(&scale);
    let popover = gtk::Popover::new();
    let weak = Rc::downgrade(app);
    popover.connect_show(move |_| {
        if let Some(app) = weak.upgrade() {
            app.submenu_epoch
                .set(app.submenu_epoch.get().wrapping_add(1));
            app.submenu_open.set(true);
        }
    });
    let weak = Rc::downgrade(app);
    popover.connect_closed(move |_| {
        // The compositor can report focus loss while the popup is being dismissed.
        let epoch = weak.upgrade().map(|app| app.submenu_epoch.get());
        let weak = weak.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(250), move || {
            if let Some(app) = weak.upgrade() {
                if Some(app.submenu_epoch.get()) == epoch {
                    app.submenu_open.set(false);
                }
            }
        });
    });
    let list = column();
    list.add_css_class("audio-devices");
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .max_content_height(300)
        .propagate_natural_height(true)
        .child(&list)
        .build();
    popover.set_child(Some(&scroll));
    let menu = gtk::MenuButton::builder()
        .icon_name("go-next-symbolic")
        .popover(&popover)
        .build();
    let accessible = if input {
        "Choose input device"
    } else {
        "Choose output device"
    };
    menu.set_tooltip_text(Some(accessible));
    menu.update_property(&[gtk::accessible::Property::Label(accessible)]);
    control.append(&menu);
    group.append(&row(label, &control));
    let target = list.downgrade();
    let popover = popover.downgrade();
    let weak = Rc::downgrade(app);
    let last = RefCell::new(Vec::<(String, String, bool)>::new());
    app.bind_for(Surface::Quick, &list, move |state| {
        let (devices, current) = if input {
            (&state.audio.inputs, &state.audio.default_input)
        } else {
            (&state.audio.outputs, &state.audio.default_output)
        };
        let next = devices
            .iter()
            .map(|device| {
                (
                    device.name.clone(),
                    device.description.clone(),
                    &device.name == current,
                )
            })
            .collect::<Vec<_>>();
        if *last.borrow() == next {
            return;
        }
        last.replace(next.clone());
        let Some(list) = target.upgrade() else {
            return;
        };
        clear(&list);
        for (name, description, selected) in next {
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let label = gtk::Label::new(Some(if description.is_empty() {
                &name
            } else {
                &description
            }));
            label.set_xalign(0.);
            label.set_hexpand(true);
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            content.append(&label);
            if selected {
                content.append(&gtk::Image::from_icon_name("object-select-symbolic"));
            }
            let button = gtk::Button::builder().child(&content).build();
            button.add_css_class("flat");
            let accessible = format!(
                "{}{}",
                if description.is_empty() {
                    &name
                } else {
                    &description
                },
                if selected { " (selected)" } else { "" }
            );
            button.update_property(&[gtk::accessible::Property::Label(&accessible)]);
            let weak = weak.clone();
            let popover = popover.clone();
            button.connect_clicked(move |_| {
                // Dismiss before dispatching: a state update can rebuild this list
                // while the selected button's click handler is still running.
                if let Some(popover) = popover.upgrade() {
                    popover.popdown();
                }
                if let Some(app) = weak.upgrade() {
                    app.execute(
                        Action::SetDefault {
                            input,
                            device: name.clone(),
                        },
                        None,
                    );
                }
            });
            list.append(&button);
        }
    });
    toggle(
        app,
        Surface::Quick,
        &group,
        if input {
            "Mute microphone"
        } else {
            "Mute output"
        },
        move |state| {
            let (devices, current) = if input {
                (&state.audio.inputs, &state.audio.default_input)
            } else {
                (&state.audio.outputs, &state.audio.default_output)
            };
            devices
                .iter()
                .find(|device| &device.name == current)
                .is_some_and(|device| device.muted)
        },
        move |muted| Action::SetMute {
            input,
            device: String::new(),
            muted,
        },
    );
}

pub fn page(app: &Rc<App>, parent: &gtk::Box) {
    parent.append(&description(
        "PipeWire audio with WirePlumber session management.",
    ));
    basic(app, parent, Surface::Full);
    devices(app, parent, Surface::Full, false);
    devices(app, parent, Surface::Full, true);
    section(parent, "Device profiles");
    let group = column();
    parent.append(&group);
    let target = group.downgrade();
    let weak = Rc::downgrade(app);
    let last = RefCell::new(String::new());
    app.bind_for(Surface::Full, &group, move |state| {
        let signature = state
            .audio
            .cards
            .iter()
            .map(|c| format!("{}:{}:{:?}", c.index, c.description, c.profiles))
            .collect::<String>();
        if *last.borrow() == signature {
            return;
        }
        last.replace(signature);
        let (Some(app), Some(group)) = (weak.upgrade(), target.upgrade()) else {
            return;
        };
        clear(&group);
        for card in &state.audio.cards {
            let labels = card
                .profiles
                .iter()
                .map(|p| p.1.as_str())
                .collect::<Vec<_>>();
            let dropdown = gtk::DropDown::from_strings(&labels);
            let profiles = card.profiles.clone();
            let id = card.index;
            let guard = Rc::new(Cell::new(false));
            let flag = guard.clone();
            let weak = Rc::downgrade(&app);
            dropdown.connect_selected_notify(move |d| {
                if !flag.get() {
                    if let (Some(app), Some(profile)) =
                        (weak.upgrade(), profiles.get(d.selected() as usize))
                    {
                        app.execute(
                            Action::SetProfile {
                                card: id,
                                profile: profile.0.clone(),
                            },
                            None,
                        );
                    }
                }
            });
            let target = dropdown.downgrade();
            let profiles = card.profiles.clone();
            app.bind_for(Surface::Full, &dropdown, move |s| {
                if let Some(d) = target.upgrade() {
                    if let Some(c) = s.audio.cards.iter().find(|c| c.index == id) {
                        guard.set(true);
                        d.set_selected(
                            profiles.iter().position(|p| p.0 == c.profile).unwrap_or(0) as u32
                        );
                        guard.set(false);
                    }
                }
            });
            group.append(&row(&card.description, &dropdown));
        }
    });
    applications(app, parent, Surface::Full);
}

pub fn applications(app: &Rc<App>, parent: &gtk::Box, surface: Surface) {
    section(parent, "Applications");
    let group = column();
    parent.append(&group);
    let target = group.downgrade();
    let weak = Rc::downgrade(app);
    let last = RefCell::new(String::new());
    app.bind_for(surface, &group, move |s| {
        let signature = s
            .audio
            .streams
            .iter()
            .map(|v| format!("{}:{}:{}", v.index, v.input, v.name))
            .collect::<String>();
        if *last.borrow() == signature {
            return;
        }
        last.replace(signature);
        let (Some(app), Some(group)) = (weak.upgrade(), target.upgrade()) else {
            return;
        };
        clear(&group);
        for stream in &s.audio.streams {
            let id = stream.index;
            let input = stream.input;
            let name = stream.name.clone();
            let weak = Rc::downgrade(&app);
            slider(
                &app,
                surface,
                &group,
                &name,
                move |s| {
                    s.audio
                        .streams
                        .iter()
                        .find(|v| v.index == id && v.input == input)
                        .map(|v| v.volume)
                        .unwrap_or(0.)
                },
                move |percent| {
                    let muted = weak
                        .upgrade()
                        .and_then(|a| {
                            a.state
                                .borrow()
                                .audio
                                .streams
                                .iter()
                                .find(|v| v.index == id && v.input == input)
                                .map(|v| v.muted)
                        })
                        .unwrap_or(false);
                    Action::SetStream {
                        input,
                        index: id,
                        percent,
                        muted,
                    }
                },
            );
            let weak = Rc::downgrade(&app);
            toggle(
                &app,
                surface,
                &group,
                "Mute application",
                move |s| {
                    s.audio
                        .streams
                        .iter()
                        .find(|v| v.index == id && v.input == input)
                        .is_some_and(|v| v.muted)
                },
                move |muted| {
                    let percent = weak
                        .upgrade()
                        .and_then(|a| {
                            a.state
                                .borrow()
                                .audio
                                .streams
                                .iter()
                                .find(|v| v.index == id && v.input == input)
                                .map(|v| v.volume)
                        })
                        .unwrap_or(0.);
                    Action::SetStream {
                        input,
                        index: id,
                        percent,
                        muted,
                    }
                },
            );
        }
    });
}

pub fn basic(app: &Rc<App>, parent: &gtk::Box, surface: Surface) {
    section(parent, "Sound");
    let unavailable = description("");
    parent.append(&unavailable);
    let target = unavailable.downgrade();
    app.bind_for(surface, &unavailable, move |s| {
        if let Some(label) = target.upgrade() {
            label.set_text(if s.audio.available {
                ""
            } else {
                "Audio is unavailable or still starting."
            });
        }
    });
    selector(app, parent, surface, false);
    selector(app, parent, surface, true);
    slider(
        app,
        surface,
        parent,
        "Output volume",
        |s| {
            s.audio
                .outputs
                .iter()
                .find(|d| d.name == s.audio.default_output)
                .map(|d| d.volume)
                .unwrap_or(0.)
        },
        |percent| Action::SetVolume {
            input: false,
            device: String::new(),
            percent,
        },
    );
    toggle(
        app,
        surface,
        parent,
        "Mute output",
        |s| {
            s.audio
                .outputs
                .iter()
                .find(|d| d.name == s.audio.default_output)
                .is_some_and(|d| d.muted)
        },
        |muted| Action::SetMute {
            input: false,
            device: String::new(),
            muted,
        },
    );
    slider(
        app,
        surface,
        parent,
        "Input volume",
        |s| {
            s.audio
                .inputs
                .iter()
                .find(|d| d.name == s.audio.default_input)
                .map(|d| d.volume)
                .unwrap_or(0.)
        },
        |percent| Action::SetVolume {
            input: true,
            device: String::new(),
            percent,
        },
    );
    toggle(
        app,
        surface,
        parent,
        "Mute microphone",
        |s| {
            s.audio
                .inputs
                .iter()
                .find(|d| d.name == s.audio.default_input)
                .is_some_and(|d| d.muted)
        },
        |muted| Action::SetMute {
            input: true,
            device: String::new(),
            muted,
        },
    );
}
fn selector(app: &Rc<App>, parent: &gtk::Box, surface: Surface, input: bool) {
    let model = gtk::StringList::new(&[]);
    let dropdown = gtk::DropDown::new(Some(model.clone()), gtk::Expression::NONE);
    let guard = Rc::new(Cell::new(false));
    let ids = Rc::new(RefCell::new(Vec::<String>::new()));
    let weak = Rc::downgrade(app);
    let names = ids.clone();
    let flag = guard.clone();
    dropdown.connect_selected_notify(move |d| {
        if !flag.get() {
            if let (Some(app), Some(name)) =
                (weak.upgrade(), names.borrow().get(d.selected() as usize))
            {
                app.execute(
                    Action::SetDefault {
                        input,
                        device: name.clone(),
                    },
                    None,
                );
            }
        }
    });
    let target = dropdown.downgrade();
    let weak_model = model.downgrade();
    app.bind_for(surface, &dropdown, move |s| {
        let devices = if input {
            &s.audio.inputs
        } else {
            &s.audio.outputs
        };
        let current = if input {
            &s.audio.default_input
        } else {
            &s.audio.default_output
        };
        if let (Some(drop), Some(model)) = (target.upgrade(), weak_model.upgrade()) {
            guard.set(true);
            let next = devices.iter().map(|d| d.name.clone()).collect::<Vec<_>>();
            if *ids.borrow() != next {
                ids.replace(next);
                let values = devices
                    .iter()
                    .map(|d| d.description.as_str())
                    .collect::<Vec<_>>();
                model.splice(0, model.n_items(), &values);
            }
            drop.set_sensitive(!devices.is_empty());
            drop.set_selected(
                devices
                    .iter()
                    .position(|d| &d.name == current)
                    .map(|p| p as u32)
                    .unwrap_or(gtk::INVALID_LIST_POSITION),
            );
            guard.set(false);
        }
    });
    parent.append(&row(
        if input {
            "Input device"
        } else {
            "Output device"
        },
        &dropdown,
    ));
}
fn devices(app: &Rc<App>, parent: &gtk::Box, surface: Surface, input: bool) {
    section(
        parent,
        if input {
            "Input devices"
        } else {
            "Output devices"
        },
    );
    let group = column();
    parent.append(&group);
    let target = group.downgrade();
    let weak = Rc::downgrade(app);
    let last = RefCell::new(String::new());
    app.bind_for(surface, &group, move |s| {
        let devices = if input {
            &s.audio.inputs
        } else {
            &s.audio.outputs
        };
        let signature = devices
            .iter()
            .map(|d| format!("{}:{:?}", d.name, d.ports))
            .collect::<String>();
        if *last.borrow() == signature {
            return;
        }
        last.replace(signature);
        let (Some(app), Some(group)) = (weak.upgrade(), target.upgrade()) else {
            return;
        };
        clear(&group);
        for device in devices {
            let controls = column();
            controls.add_css_class("settings-group");
            controls.append(&description(&device.description));
            let name = device.name.clone();
            let id = name.clone();
            slider(
                &app,
                surface,
                &controls,
                "Volume",
                move |s| {
                    let devices = if input {
                        &s.audio.inputs
                    } else {
                        &s.audio.outputs
                    };
                    devices
                        .iter()
                        .find(|d| d.name == id)
                        .map(|d| d.volume)
                        .unwrap_or(0.)
                },
                move |percent| Action::SetVolume {
                    input,
                    device: name.clone(),
                    percent,
                },
            );
            let name = device.name.clone();
            let id = name.clone();
            toggle(
                &app,
                surface,
                &controls,
                "Mute",
                move |s| {
                    let devices = if input {
                        &s.audio.inputs
                    } else {
                        &s.audio.outputs
                    };
                    devices
                        .iter()
                        .find(|d| d.name == id)
                        .is_some_and(|d| d.muted)
                },
                move |muted| Action::SetMute {
                    input,
                    device: name.clone(),
                    muted,
                },
            );
            let name = device.name.clone();
            let id = name.clone();
            slider(
                &app,
                surface,
                &controls,
                "Balance (left → right)",
                move |s| {
                    let devices = if input {
                        &s.audio.inputs
                    } else {
                        &s.audio.outputs
                    };
                    (devices
                        .iter()
                        .find(|d| d.name == id)
                        .map(|d| d.balance)
                        .unwrap_or(0.)
                        + 1.)
                        * 50.
                },
                move |percent| Action::SetBalance {
                    input,
                    device: name.clone(),
                    balance: percent / 50. - 1.,
                },
            );
            if !device.ports.is_empty() {
                let pairs = device
                    .ports
                    .iter()
                    .map(|(id, label)| (id.as_str(), label.as_str()))
                    .collect::<Vec<_>>();
                let drop = choose(&controls, "Port", &pairs, &device.port);
                let ports = device.ports.clone();
                let name = device.name.clone();
                let weak = Rc::downgrade(&app);
                let guard = Rc::new(Cell::new(false));
                let flag = guard.clone();
                drop.connect_selected_notify(move |drop| {
                    if flag.get() {
                        return;
                    }
                    if let (Some(app), Some(port)) =
                        (weak.upgrade(), ports.get(drop.selected() as usize))
                    {
                        app.execute(
                            Action::SetPort {
                                input,
                                device: name.clone(),
                                port: port.0.clone(),
                            },
                            None,
                        );
                    }
                });
                let target = drop.downgrade();
                let ports = device.ports.clone();
                let name = device.name.clone();
                app.bind_for(surface, &drop, move |s| {
                    let devices = if input {
                        &s.audio.inputs
                    } else {
                        &s.audio.outputs
                    };
                    if let (Some(drop), Some(device)) =
                        (target.upgrade(), devices.iter().find(|d| d.name == name))
                    {
                        guard.set(true);
                        drop.set_selected(
                            ports
                                .iter()
                                .position(|p| p.0 == device.port)
                                .map(|n| n as u32)
                                .unwrap_or(gtk::INVALID_LIST_POSITION),
                        );
                        guard.set(false);
                    }
                });
            }
            group.append(&controls);
        }
    });
}
