use super::*;
use std::{cell::Cell, cell::RefCell, rc::Rc};

/// The Screen Capture page edits one draft of the whole section and resubmits it with
/// every change, so the daemon stays the single writer of screen-capture.json. The
/// controls mirror the backend's own GTK frontend: quality presets are always shown,
/// the remaining recording knobs live behind an expander bound to `advanced_view`.
pub fn page(app: &Rc<App>, parent: &gtk::Box) {
    parent.append(&description(
        "Screenshots and screen recordings are taken by uflinux-screencapture, launched with the Print Screen key.",
    ));

    let draft = Rc::new(RefCell::new(app.state.borrow().screen_capture.clone()));
    let sender = Rc::downgrade(app);
    let send_draft = draft.clone();
    let send: Rc<dyn Fn()> = Rc::new(move || {
        if let Some(app) = sender.upgrade() {
            let screen_capture = send_draft.borrow().clone();
            app.execute(Action::SetScreenCapture { screen_capture }, None);
        }
    });

    screenshots(app, parent, &draft, &send);
    recording(app, parent, &draft, &send);
    backend(app, parent);
    shortcuts(parent);

    section(parent, "Restore defaults");
    parent.append(&description(
        "Return every option on this page, including the advanced view, to its original value.",
    ));
    parent.append(&button(app, "Restore defaults", || {
        Action::ResetScreenCapture
    }));
}

/// Screenshots group: format, folder, clipboard, flash, cursor, delay.
fn screenshots(
    app: &Rc<App>,
    parent: &gtk::Box,
    draft: &Rc<RefCell<ScreenCapture>>,
    send: &Rc<dyn Fn()>,
) {
    section(parent, "Screenshots");
    parent.append(&description(
        "Images are saved to a folder and copied to the clipboard, like GNOME Screenshot.",
    ));

    let formats = [
        (CaptureFormat::Webp, "WebP (smaller files)"),
        (CaptureFormat::Png, "PNG (lossless)"),
        (CaptureFormat::Jpeg, "JPEG"),
    ];
    let labels = formats
        .iter()
        .map(|(_, label)| tr(label).to_string())
        .collect::<Vec<_>>();
    let format = dropdown(parent, "Image format", &labels);
    bind_dropdown(app, &format, move |s| {
        formats
            .iter()
            .position(|(format, _)| *format == s.screen_capture.image_format)
            .unwrap_or(0) as u32
    });
    let send1 = send.clone();
    let draft1 = draft.clone();
    format.connect_selected_notify(move |drop| {
        if let Some((format, _)) = formats.get(drop.selected() as usize) {
            draft1.borrow_mut().image_format = *format;
            send1();
        }
    });

    let folder = entry(parent, "Screenshot folder", &draft.borrow().screenshot_dir);
    folder.set_placeholder_text(Some("Pictures/Screenshots"));
    let picker = gtk::Button::with_label(&tr("Choose…"));
    picker.set_valign(gtk::Align::Center);
    parent.append(&row("Change folder", &picker));
    let weak = Rc::downgrade(app);
    let folder_text = folder.clone();
    let send2 = send.clone();
    let draft2 = draft.clone();
    picker.connect_clicked(move |_| {
        let Some(app) = weak.upgrade() else {
            return;
        };
        let folder = folder_text.clone();
        let send = send2.clone();
        let draft = draft2.clone();
        glib::MainContext::default().spawn_local(async move {
            let chooser = gtk::FileDialog::builder()
                .title("Choose screenshot folder")
                .build();
            if let Ok(file) = chooser.select_folder_future(app.parent().as_ref()).await {
                if let Some(path) = file.path().map(|path| path.to_string_lossy().to_string()) {
                    folder.set_text(&path);
                    draft.borrow_mut().screenshot_dir = path;
                    send();
                }
            }
        });
    });
    let send4 = send.clone();
    let draft4 = draft.clone();
    folder.connect_changed(move |entry| {
        draft4.borrow_mut().screenshot_dir = entry.text().to_string();
        send4();
    });

    switch(
        app,
        parent,
        &draft,
        &send,
        "Copy to clipboard",
        |p| p.copy_to_clipboard,
        |p, value| p.copy_to_clipboard = value,
    );
    switch(
        app,
        parent,
        &draft,
        &send,
        "Flash screen when capturing",
        |p| p.flash,
        |p, value| p.flash = value,
    );
    switch(
        app,
        parent,
        &draft,
        &send,
        "Include mouse cursor",
        |p| p.include_cursor,
        |p, value| p.include_cursor = value,
    );

    let (delay, _) = spin(
        parent,
        "Delay before capture (seconds)",
        0.,
        MAX_DELAY_SECONDS as f64,
        draft.borrow().delay_seconds as f64,
    );
    let send5 = send.clone();
    let draft5 = draft.clone();
    delay.connect_value_changed(move |spin| {
        draft5.borrow_mut().delay_seconds = spin.value() as u8;
        send5();
    });
}

/// Recording group: target, quality preset, audio, frame rate, and the advanced expander.
fn recording(
    app: &Rc<App>,
    parent: &gtk::Box,
    draft: &Rc<RefCell<ScreenCapture>>,
    send: &Rc<dyn Fn()>,
) {
    section(parent, "Screen recording");
    parent.append(&description(
        "Recording uses the GPU screen recorder backend. Window capture goes through the desktop portal, where the frame rate follows the compositor rather than the value below.",
    ));

    let targets = [
        CaptureTarget::Screen,
        CaptureTarget::Window,
        CaptureTarget::Area,
    ];
    let labels = targets
        .iter()
        .map(|t| tr(t.label()).to_string())
        .collect::<Vec<_>>();
    let target = dropdown(parent, "Capture target", &labels);
    bind_dropdown(app, &target, move |s| {
        targets
            .iter()
            .position(|t| *t == s.screen_capture.target)
            .unwrap_or(0) as u32
    });
    let send1 = send.clone();
    let draft1 = draft.clone();
    target.connect_selected_notify(move |drop| {
        if let Some(target) = targets.get(drop.selected() as usize) {
            draft1.borrow_mut().target = *target;
            send1();
        }
    });

    let qualities = [
        CaptureQuality::Medium,
        CaptureQuality::High,
        CaptureQuality::VeryHigh,
        CaptureQuality::Ultra,
        CaptureQuality::ConstantBitrate,
    ];
    let labels = qualities
        .iter()
        .map(|quality| {
            let hint = quality.hint();
            if hint.is_empty() {
                tr(quality.label()).to_string()
            } else {
                format!("{} ({})", tr(quality.label()), tr(hint))
            }
        })
        .collect::<Vec<_>>();
    let quality = dropdown(parent, "Video quality", &labels);
    bind_dropdown(app, &quality, move |s| {
        qualities
            .iter()
            .position(|q| *q == s.screen_capture.quality)
            .unwrap_or(2) as u32
    });
    let send2 = send.clone();
    let draft2 = draft.clone();
    quality.connect_selected_notify(move |drop| {
        if let Some(quality) = qualities.get(drop.selected() as usize) {
            draft2.borrow_mut().quality = *quality;
            send2();
        }
    });

    let sources = [
        AudioSource::None,
        AudioSource::Output,
        AudioSource::Input,
        AudioSource::OutputAndInput,
    ];
    let labels = sources
        .iter()
        .map(|a| tr(a.label()).to_string())
        .collect::<Vec<_>>();
    let audio = dropdown(parent, "Audio", &labels);
    bind_dropdown(app, &audio, move |s| {
        sources
            .iter()
            .position(|a| *a == s.screen_capture.audio_source)
            .unwrap_or(0) as u32
    });
    let send3 = send.clone();
    let draft3 = draft.clone();
    audio.connect_selected_notify(move |drop| {
        if let Some(source) = sources.get(drop.selected() as usize) {
            draft3.borrow_mut().audio_source = *source;
            send3();
        }
    });

    let (fps, _) = spin(
        parent,
        "Frame rate (frames per second)",
        1.,
        MAX_FPS as f64,
        draft.borrow().fps as f64,
    );
    let send4 = send.clone();
    let draft4 = draft.clone();
    fps.connect_value_changed(move |spin| {
        draft4.borrow_mut().fps = spin.value() as u8;
        send4();
    });

    // Bitrate is only meaningful for the constant-bitrate preset, exactly like the
    // backend frontend, so its row is revealed only when that preset is selected.
    let (bitrate, bitrate_row) = spin(
        parent,
        "Bitrate (kilobits per second)",
        1.,
        MAX_BITRATE_KBPS as f64,
        draft.borrow().bitrate_kbps as f64,
    );
    let send5 = send.clone();
    let draft5 = draft.clone();
    bitrate.connect_value_changed(move |spin| {
        draft5.borrow_mut().bitrate_kbps = spin.value() as u32;
        send5();
    });
    let target = bitrate_row.downgrade();
    app.bind_for(Surface::Full, &bitrate_row, move |state| {
        if let Some(row) = target.upgrade() {
            row.set_visible(state.screen_capture.quality == CaptureQuality::ConstantBitrate);
        }
    });

    // The expander's open state is the persisted advanced_view flag. A guard swallows
    // the notify emitted when the flag is written back from state.
    let advanced = gtk::Expander::new(Some(&tr("Advanced")));
    parent.append(&advanced);
    let box_ = column();
    advanced.set_child(Some(&box_));
    let guard = Rc::new(Cell::new(false));
    let send6 = send.clone();
    let draft6 = draft.clone();
    let flag = guard.clone();
    advanced.connect_expanded_notify(move |expander| {
        if flag.get() {
            return;
        }
        draft6.borrow_mut().advanced_view = expander.is_expanded();
        send6();
    });
    let target = advanced.downgrade();
    let guard_for_bind = guard.clone();
    app.bind_for(Surface::Full, &advanced, move |state| {
        if let Some(expander) = target.upgrade() {
            guard_for_bind.set(true);
            expander.set_expanded(state.screen_capture.advanced_view);
            guard_for_bind.set(false);
        }
    });

    option(
        app,
        &box_,
        &draft,
        &send,
        "Video codec",
        &VIDEO_CODECS,
        |p| p.video_codec.clone(),
        |p, value| p.video_codec = value,
    );
    option(
        app,
        &box_,
        &draft,
        &send,
        "Container",
        &CONTAINERS,
        |p| p.container.clone(),
        |p, value| p.container = value,
    );

    let codecs = [AudioCodec::Opus, AudioCodec::Aac];
    let labels = std::iter::once(tr("Backend default").to_string())
        .chain(codecs.iter().map(|c| tr(c.label()).to_string()))
        .collect::<Vec<_>>();
    let codec = dropdown(&box_, "Audio codec", &labels);
    bind_dropdown(app, &codec, move |state| {
        match state.screen_capture.audio_codec {
            None => 0,
            Some(codec) => codecs
                .iter()
                .position(|candidate| *candidate == codec)
                .map(|index| index as u32 + 1)
                .unwrap_or(0),
        }
    });
    let send7 = send.clone();
    let draft7 = draft.clone();
    codec.connect_selected_notify(move |drop| {
        draft7.borrow_mut().audio_codec = drop
            .selected()
            .checked_sub(1)
            .and_then(|index| codecs.get(index as usize).copied());
        send7();
    });

    let modes = [
        FramerateMode::Auto,
        FramerateMode::Constant,
        FramerateMode::Variable,
        FramerateMode::Content,
    ];
    let labels = modes
        .iter()
        .map(|m| tr(m.label()).to_string())
        .collect::<Vec<_>>();
    let mode = dropdown(&box_, "Frame rate mode", &labels);
    bind_dropdown(app, &mode, move |state| {
        modes
            .iter()
            .position(|m| *m == state.screen_capture.framerate_mode)
            .unwrap_or(0) as u32
    });
    let send8 = send.clone();
    let draft8 = draft.clone();
    mode.connect_selected_notify(move |drop| {
        if let Some(mode) = modes.get(drop.selected() as usize) {
            draft8.borrow_mut().framerate_mode = *mode;
            send8();
        }
    });

    let ranges = [ColorRange::Limited, ColorRange::Full];
    let labels = ranges
        .iter()
        .map(|r| tr(r.label()).to_string())
        .collect::<Vec<_>>();
    let range = dropdown(&box_, "Colour range", &labels);
    bind_dropdown(app, &range, move |state| {
        ranges
            .iter()
            .position(|r| *r == state.screen_capture.color_range)
            .unwrap_or(0) as u32
    });
    let send9 = send.clone();
    let draft9 = draft.clone();
    range.connect_selected_notify(move |drop| {
        if let Some(range) = ranges.get(drop.selected() as usize) {
            draft9.borrow_mut().color_range = *range;
            send9();
        }
    });
}

/// Backend availability line plus a button that re-runs the capability probe.
fn backend(app: &Rc<App>, parent: &gtk::Box) {
    section(parent, "Backend");
    let status = gtk::Label::new(None);
    status.set_xalign(0.);
    status.set_wrap(true);
    parent.append(&row("GPU screen recorder", &status));
    let target = status.downgrade();
    app.bind_for(Surface::Full, &status, move |state| {
        if let Some(status) = target.upgrade() {
            let caps = &state.capture_capabilities;
            if caps.backend_available {
                status.set_text(&format!(
                    "{} — {} image formats, {} codecs, {} containers",
                    tr("Available"),
                    caps.image_formats.len(),
                    caps.video_codecs.len(),
                    caps.containers.len()
                ));
            } else {
                status.set_text(&tr(
                    "Not detected. Install gpu-screen-recorder to record video.",
                ));
            }
        }
    });
    let weak = Rc::downgrade(app);
    let refresh = gtk::Button::with_label(&tr("Refresh"));
    refresh.set_valign(gtk::Align::Center);
    refresh.connect_clicked(move |_| {
        if let Some(app) = weak.upgrade() {
            app.execute(Action::ScreenCaptureCapabilities, None);
        }
    });
    parent.append(&row("Re-detect backend", &refresh));
}

/// Read-only shortcut rows, mirroring the labwc keybinds installed by uflinux-defaults.
fn shortcuts(parent: &gtk::Box) {
    section(parent, "Keyboard shortcuts");
    for (keys, action) in [
        ("Print Screen", "Open the capture tool"),
        ("Shift + Print Screen", "Start a screen recording"),
        ("Ctrl + Print Screen", "Start a screen recording"),
    ] {
        let label = gtk::Label::new(Some(&tr(action)));
        label.set_xalign(0.);
        label.set_hexpand(true);
        label.set_wrap(true);
        let shortcut = gtk::Label::new(Some(keys));
        shortcut.add_css_class("dim-label");
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.add_css_class("settings-row");
        row.append(&label);
        row.append(&shortcut);
        parent.append(&row);
    }
}

/// A "Backend default" plus fixed string list option (video codec, container). Index 0
/// is the default (None); the remaining entries map to the given backend values.
fn option(
    app: &Rc<App>,
    parent: &gtk::Box,
    draft: &Rc<RefCell<ScreenCapture>>,
    send: &Rc<dyn Fn()>,
    label: &str,
    values: &'static [&'static str],
    current: fn(&ScreenCapture) -> Option<String>,
    write: fn(&mut ScreenCapture, Option<String>),
) {
    let labels = std::iter::once(tr("Backend default").to_string())
        .chain(values.iter().map(|value| (*value).to_string()))
        .collect::<Vec<_>>();
    let drop = dropdown(parent, label, &labels);
    bind_dropdown(app, &drop, move |state| {
        current(&state.screen_capture)
            .and_then(|value| values.iter().position(|candidate| *candidate == value))
            .map(|index| index as u32 + 1)
            .unwrap_or(0)
    });
    let send = send.clone();
    let draft = draft.clone();
    drop.connect_selected_notify(move |drop| {
        let value = drop
            .selected()
            .checked_sub(1)
            .and_then(|index| values.get(index as usize).map(|value| (*value).to_string()));
        write(&mut draft.borrow_mut(), value);
        send();
    });
}

/// A labelled DropDown over a fixed set of strings.
fn dropdown(parent: &gtk::Box, label: &str, strings: &[String]) -> gtk::DropDown {
    let refs = strings
        .iter()
        .map(|string| string.as_str())
        .collect::<Vec<_>>();
    let drop = gtk::DropDown::from_strings(&refs);
    parent.append(&row(label, &drop));
    drop
}

/// Refresh a DropDown's selection from state without echoing the change back.
fn bind_dropdown(app: &Rc<App>, drop: &gtk::DropDown, index: impl Fn(&Snapshot) -> u32 + 'static) {
    let target = drop.downgrade();
    app.bind_for(Surface::Full, drop, move |state| {
        if let Some(drop) = target.upgrade() {
            let want = index(state);
            if drop.selected() != want {
                drop.set_selected(want);
            }
        }
    });
}

/// A labelled switch that mutates one draft field and resubmits the whole section.
fn switch(
    app: &Rc<App>,
    parent: &gtk::Box,
    draft: &Rc<RefCell<ScreenCapture>>,
    send: &Rc<dyn Fn()>,
    name: &str,
    get: fn(&ScreenCapture) -> bool,
    set: fn(&mut ScreenCapture, bool),
) {
    let control = gtk::Switch::new();
    control.set_valign(gtk::Align::Center);
    control.update_property(&[gtk::accessible::Property::Label(name)]);
    control.set_active(get(&draft.borrow()));
    parent.append(&row(name, &control));
    let guard = Rc::new(Cell::new(false));
    let send = send.clone();
    let draft = draft.clone();
    let flag = guard.clone();
    control.connect_state_set(move |_, value| {
        if !flag.get() {
            set(&mut draft.borrow_mut(), value);
            send();
        }
        glib::Propagation::Proceed
    });
    let target = control.downgrade();
    app.bind_for(Surface::Full, &control, move |state| {
        if let Some(control) = target.upgrade() {
            guard.set(true);
            control.set_active(get(&state.screen_capture));
            guard.set(false);
        }
    });
}

fn spin(
    parent: &gtk::Box,
    name: &str,
    min: f64,
    max: f64,
    value: f64,
) -> (gtk::SpinButton, gtk::Box) {
    let spin = gtk::SpinButton::with_range(min, max, 1.);
    spin.set_value(value);
    spin.set_valign(gtk::Align::Center);
    spin.update_property(&[gtk::accessible::Property::Label(name)]);
    let row = row(name, &spin);
    parent.append(&row);
    (spin, row)
}
