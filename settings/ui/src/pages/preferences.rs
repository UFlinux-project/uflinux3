use super::*;
use std::{cell::RefCell, fs};

pub fn defaults(app: &Rc<App>, parent: &gtk::Box) {
    parent.append(&description("Choose from installed applications. Associations are saved for your account using the XDG standard."));
    for (name, types) in [
        (
            "Web browser",
            vec!["x-scheme-handler/http", "x-scheme-handler/https"],
        ),
        ("Email", vec!["x-scheme-handler/mailto"]),
        ("File manager", vec!["inode/directory"]),
        ("Text", vec!["text/plain"]),
        ("PDF", vec!["application/pdf"]),
        ("Images", vec!["image/png", "image/jpeg", "image/webp"]),
        ("Music", vec!["audio/mpeg", "audio/ogg", "audio/flac"]),
        ("Video", vec!["video/mp4", "video/x-matroska"]),
    ] {
        let mut choices = Vec::<gio::AppInfo>::new();
        for mime in &types {
            for candidate in gio::AppInfo::all_for_type(mime) {
                if !choices.iter().any(|a| a.equal(&candidate)) {
                    choices.push(candidate);
                }
            }
        }
        if choices.is_empty() {
            parent.append(&row(name, &description("No application installed")));
            continue;
        }
        let names = choices
            .iter()
            .map(|a| a.display_name().to_string())
            .collect::<Vec<_>>();
        let refs = names.iter().map(String::as_str).collect::<Vec<_>>();
        let drop = gtk::DropDown::from_strings(&refs);
        let current = gio::AppInfo::default_for_type(types[0], false);
        drop.set_selected(
            current
                .as_ref()
                .and_then(|c| choices.iter().position(|a| a.equal(c)))
                .map(|p| p as u32)
                .unwrap_or(gtk::INVALID_LIST_POSITION),
        );
        let weak = Rc::downgrade(app);
        drop.connect_selected_notify(move |drop| {
            if let Some(info) = choices.get(drop.selected() as usize) {
                for mime in &types {
                    if let Err(error) = info.set_as_default_for_type(mime) {
                        if let Some(app) = weak.upgrade() {
                            app.error(error.message());
                        }
                        break;
                    }
                }
            }
        });
        parent.append(&row(name, &drop));
    }
    let mime = entry(parent, "Another MIME type", "");
    let choose = gtk::Button::with_label("Choose application…");
    let weak = Rc::downgrade(app);
    choose.connect_clicked(move |_| {
        let Some(app) = weak.upgrade() else {
            return;
        };
        let content = mime.text();
        if content.is_empty() || !content.contains('/') || content.chars().any(char::is_whitespace)
        {
            app.error("Enter a MIME type such as text/markdown");
            return;
        }
        let (window, parent) = dialog(&app, "Choose default application");
        let choices = gio::AppInfo::all()
            .into_iter()
            .filter(|a| a.should_show())
            .collect::<Vec<_>>();
        let names = choices
            .iter()
            .map(|a| a.display_name().to_string())
            .collect::<Vec<_>>();
        let refs = names.iter().map(String::as_str).collect::<Vec<_>>();
        let select = gtk::DropDown::from_strings(&refs);
        select.set_enable_search(true);
        parent.append(&select);
        let save = gtk::Button::with_label("Use this application");
        let weak = Rc::downgrade(&app);
        let mime = content.to_string();
        save.connect_clicked(move |_| {
            if let Some(info) = choices.get(select.selected() as usize) {
                if let Err(error) = info.set_as_default_for_type(&mime) {
                    if let Some(app) = weak.upgrade() {
                        app.error(error.message());
                    }
                }
            }
        });
        parent.append(&save);
        window.present();
    });
    parent.append(&choose);
}
pub fn datetime(app: &Rc<App>, parent: &gtk::Box) {
    let summary = description("");
    parent.append(&summary);
    let target = summary.downgrade();
    app.bind_for(Surface::Full, &summary, move |s| {
        if let Some(label) = target.upgrade() {
            label.set_text(&format!(
                "Timezone: {}\nAutomatic time: {} · Service: {} · Synchronization: {}",
                s.datetime.timezone,
                if s.datetime.time_sync_enabled {
                    "enabled"
                } else {
                    "disabled"
                },
                if s.datetime.sntp_running {
                    "running"
                } else {
                    "stopped"
                },
                match s.datetime.synchronized {
                    Some(true) => "synchronized",
                    Some(false) => "not synchronized",
                    None => "unknown / unavailable",
                }
            ));
        }
    });
    section(parent, "Time servers");
    let servers = description("");
    servers.set_wrap(true);
    servers.set_xalign(0.);
    parent.append(&servers);
    let target = servers.downgrade();
    app.bind_for(Surface::Full, &servers, move |s| {
        if let Some(label) = target.upgrade() {
            label.set_text(&format!(
                "{}\nConnMan uses servers announced by the connected network first, then its configured fallback list.",
                if s.datetime.timeservers.is_empty() {
                    "No time server is currently configured.".to_string()
                } else {
                    s.datetime.timeservers.join(", ")
                }
            ));
        }
    });
    toggle(
        app,
        Surface::Full,
        parent,
        "Automatic date and time (ConnMan SNTP)",
        |s| s.datetime.time_sync_enabled,
        |enabled| Action::System {
            operation: SystemAction::SetTimeSync { enabled },
        },
    );
    section(parent, "Timezone");
    let mut zones = vec!["UTC".to_string()];
    for line in fs::read_to_string("/usr/share/zoneinfo/zone1970.tab")
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.starts_with('#'))
    {
        if let Some(zone) = line.split('\t').nth(2) {
            zones.push(zone.to_string());
        }
    }
    zones.sort();
    zones.dedup();
    let names = zones.iter().map(String::as_str).collect::<Vec<_>>();
    let timezone = gtk::DropDown::from_strings(&names);
    timezone.set_enable_search(true);
    timezone.set_selected(
        zones
            .iter()
            .position(|z| z == &app.state.borrow().datetime.timezone)
            .unwrap_or_else(|| zones.iter().position(|z| z == "UTC").unwrap_or(0)) as u32,
    );
    parent.append(&row("Region / city", &timezone));
    let choices = zones.clone();
    parent.append(&button(app, "Apply timezone", move || Action::System {
        operation: SystemAction::SetTimezone {
            timezone: choices
                .get(timezone.selected() as usize)
                .cloned()
                .unwrap_or_else(|| "UTC".into()),
        },
    }));
    section(parent, "Manual date and time");
    parent.append(&description("Disable automatic time before changing the clock. The selected date and time use the system timezone. The hardware clock remains in UTC."));
    let calendar = gtk::Calendar::new();
    parent.append(&calendar);
    let time = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let hour = gtk::SpinButton::with_range(0., 23., 1.);
    let minute = gtk::SpinButton::with_range(0., 59., 1.);
    let second = gtk::SpinButton::with_range(0., 59., 1.);
    if let Ok(now) = glib::DateTime::now_local() {
        hour.set_value(now.hour() as f64);
        minute.set_value(now.minute() as f64);
        second.set_value(now.second() as f64);
    }
    for (name, control) in [("Hour", &hour), ("Minute", &minute), ("Second", &second)] {
        time.append(&row(name, control));
    }
    parent.append(&time);
    let save = gtk::Button::with_label("Set date and time");
    let weak = Rc::downgrade(app);
    save.connect_clicked(move |button| {
        let Some(app) = weak.upgrade() else {
            return;
        };
        let date = calendar.date();
        let zone = glib::TimeZone::from_identifier(Some(&app.state.borrow().datetime.timezone));
        let result = zone.ok_or("Invalid timezone").and_then(|zone| {
            glib::DateTime::new(
                &zone,
                date.year(),
                date.month(),
                date.day_of_month(),
                hour.value_as_int(),
                minute.value_as_int(),
                second.value(),
            )
            .map_err(|_| "Invalid date or local time")
        });
        match result {
            Ok(date) => app.execute(
                Action::System {
                    operation: SystemAction::SetTime {
                        utc_seconds: date.to_unix(),
                    },
                },
                Some(button.clone().upcast()),
            ),
            Err(error) => app.error(error),
        }
    });
    let target = save.downgrade();
    app.bind_for(Surface::Full, &save, move |s| {
        if let Some(button) = target.upgrade() {
            button.set_sensitive(!s.datetime.time_sync_enabled);
        }
    });
    parent.append(&save);
    parent.append(&button(app, "Refresh clock status", || Action::Refresh));
}
pub fn appearance(app: &Rc<App>, parent: &gtk::Box) {
    parent.append(&description("Choose a local wallpaper. Images are limited to 16 megapixels and 32 MiB to keep memory use predictable."));
    let mut outputs = vec![("*".to_string(), "All displays".to_string())];
    outputs.extend(
        app.state
            .borrow()
            .displays
            .outputs
            .iter()
            .map(|o| (o.name.clone(), o.name.clone())),
    );
    let pairs = outputs
        .iter()
        .map(|p| (p.0.as_str(), p.1.as_str()))
        .collect::<Vec<_>>();
    let output = choose(parent, "Display", &pairs, "*");
    let fit = choose(
        parent,
        "Fit",
        &[
            ("fill", "Fill (crop)"),
            ("fit", "Fit"),
            ("stretch", "Stretch"),
            ("center", "Center"),
            ("tile", "Tile"),
        ],
        "fill",
    );
    let appearance = app.state.borrow().appearance.clone();
    let initial = appearance
        .wallpapers
        .first()
        .map(|w| w.file.clone())
        .unwrap_or_else(|| "/usr/share/backgrounds/uflinux/Dark_Ivy.jpg".into());
    let selected = Rc::new(RefCell::new(initial.clone()));
    let name = description(&initial);
    parent.append(&name);
    let preview = gtk::Picture::new();
    preview.set_size_request(192, 108);
    preview.set_can_shrink(true);
    preview.set_content_fit(gtk::ContentFit::Contain);
    parent.append(&preview);
    let choose_image = gtk::Button::with_label("Choose wallpaper…");
    parent.append(&choose_image);
    let weak = Rc::downgrade(app);
    let path = selected.clone();
    let image = preview.downgrade();
    let label = name.downgrade();
    choose_image.connect_clicked(move |_| {
        let weak = weak.clone();
        let path = path.clone();
        let image = image.clone();
        let label = label.clone();
        glib::MainContext::default().spawn_local(async move {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let chooser = gtk::FileDialog::builder().title("Choose wallpaper").build();
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("Images"));
            for mime in ["image/png", "image/jpeg", "image/webp", "image/svg+xml"] {
                filter.add_mime_type(mime);
            }
            chooser.set_default_filter(Some(&filter));
            chooser
                .set_initial_folder(Some(&gio::File::for_path("/usr/share/backgrounds/uflinux")));
            if let Ok(file) = chooser.open_future(app.parent().as_ref()).await {
                if let Some(chosen) = file.path() {
                    let source = chosen.clone();
                    let result = gio::spawn_blocking(move || {
                        let size = fs::metadata(&source)
                            .map_err(|_| "Cannot read image")?
                            .len();
                        let (_, w, h) =
                            gdk_pixbuf::Pixbuf::file_info(&source).ok_or("Unsupported image")?;
                        if size > 32 * 1024 * 1024
                            || w <= 0
                            || h <= 0
                            || w as i64 * h as i64 > 16_777_216
                        {
                            return Err("Image exceeds the memory limit");
                        }
                        // Return pixel bytes, never a non-Send GTK object, to the main context.
                        let pixbuf =
                            gdk_pixbuf::Pixbuf::from_file_at_scale(&source, 192, 108, true)
                                .map_err(|_| "Cannot preview image")?;
                        let bytes = pixbuf
                            .save_to_bufferv("png", &[])
                            .map_err(|_| "Cannot encode preview")?;
                        Ok(bytes)
                    })
                    .await;
                    match result {
                        Ok(Ok(bytes)) => {
                            path.replace(chosen.to_string_lossy().to_string());
                            if let Some(label) = label.upgrade() {
                                label.set_text(&chosen.to_string_lossy());
                            }
                            if let Some(image) = image.upgrade() {
                                if let Ok(texture) =
                                    gtk::gdk::Texture::from_bytes(&glib::Bytes::from_owned(bytes))
                                {
                                    image.set_paintable(Some(&texture));
                                }
                            }
                        }
                        _ => app.error("Cannot preview this image, or it exceeds the memory limit"),
                    }
                }
            }
        });
    });
    let color = entry(parent, "Background color (#RRGGBB)", &appearance.color);
    let weak = Rc::downgrade(app);
    parent.append(&button(app, "Apply wallpaper", move || {
        let id = outputs
            .get(output.selected() as usize)
            .map(|p| p.0.clone())
            .unwrap_or_else(|| "*".into());
        let mut appearance = weak
            .upgrade()
            .map(|a| a.state.borrow().appearance.clone())
            .unwrap_or_default();
        appearance.color = color.text().to_string();
        appearance.wallpapers.retain(|w| w.output != id);
        appearance.wallpapers.push(Wallpaper {
            output: id,
            file: selected.borrow().clone(),
            fit: ["fill", "fit", "stretch", "center", "tile"][fit.selected().min(4) as usize]
                .into(),
        });
        // Defaults precede overrides in swaybg's per-output command sequence.
        appearance
            .wallpapers
            .sort_by_key(|w| if w.output == "*" { 0 } else { 1 });
        Action::SetAppearance { appearance }
    }));
    parent.append(&button(app, "Restore default wallpaper", || {
        Action::SetAppearance {
            appearance: Appearance::default(),
        }
    }));
}
pub fn about(app: &Rc<App>, parent: &gtk::Box) {
    let logo = gtk::Image::from_icon_name("org.uflinux.Settings");
    logo.set_pixel_size(48);
    logo.set_halign(gtk::Align::Start);
    logo.update_property(&[gtk::accessible::Property::Label("UFLinux leaf logo")]);
    parent.append(&logo);
    let summary = gtk::Label::new(None);
    summary.set_xalign(0.);
    summary.set_yalign(0.);
    summary.set_selectable(true);
    summary.set_wrap(true);
    parent.append(&summary);
    let target = summary.downgrade();
    app.bind_for(Surface::Full, &summary, move |s| {
        if let Some(label) = target.upgrade() {
            let mut rows = s
                .about
                .iter()
                .map(|(name, value)| {
                    let name = match name.as_str() {
                        "PRETTY_NAME" => "Distribution",
                        "VERSION_ID" => "Distribution version",
                        "MemTotal" => "Installed memory",
                        "MemAvailable" => "Available memory",
                        name => name,
                    };
                    let value = if name.ends_with("memory") {
                        value
                            .split_whitespace()
                            .next()
                            .and_then(|s| s.parse::<f64>().ok())
                            .map(|kib| format!("{:.0} MiB", kib / 1024.))
                            .unwrap_or_else(|| value.clone())
                    } else {
                        value.clone()
                    };
                    format!("{}: {value}", tr(name))
                })
                .collect::<Vec<_>>();
            rows.push(format!("Renderer: {}", s.displays.renderer));
            rows.push(format!(
                "Displays: {}",
                s.displays
                    .outputs
                    .iter()
                    .map(|o| format!("{} ({} {})", o.name, o.make, o.model))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
            label.set_text(&rows.join("\n\n"));
        }
    });
    let copy = gtk::Button::with_label("Copy system information");
    let text = summary.clone();
    copy.connect_clicked(move |_| {
        if let Some(display) = gtk::gdk::Display::default() {
            display.clipboard().set_text(&text.text());
        }
    });
    parent.append(&copy);
    parent.append(&button(app, "Refresh system information", || {
        Action::Refresh
    }));
}
