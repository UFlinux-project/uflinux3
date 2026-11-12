//! The chooser shown on PrintScreen: what to capture and how.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use settings_core::{CaptureTarget, ScreenCapture};

use crate::backend::Target;
use crate::shell;

/// What the user asked for.
pub enum Choice {
    /// Take a single screenshot of this source.
    Still(Target),
    /// Record this source until the user says so.
    Video(Target),
    /// Ask for a rectangle first, then take a screenshot.
    Area,
    /// Nothing was captured.
    Cancel,
}

const TARGET_NAMES: [&str; 3] = ["Screen", "Window", "Area"];

type Done = Box<dyn FnOnce(Choice)>;

/// Show the chooser card centred on `monitor`. `on_choice` runs exactly once.
pub fn show(
    monitor: Option<&gdk::Monitor>,
    prefs: &ScreenCapture,
    recording_allowed: bool,
    on_choice: impl FnOnce(Choice) + 'static,
) {
    let Some(monitor) = monitor.cloned().or_else(shell::primary_monitor) else {
        on_choice(Choice::Cancel);
        return;
    };

    let window = gtk::Window::new();
    shell::popup(&window, Some(&monitor), shell::Keyboard::OnDemand);
    let window_for_escape = window.clone();
    shell::add_escape_key(&window, move || {
        shell::dismiss(&window_for_escape);
    });

    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .css_classes(["uflinux-capture-card"])
        .build();

    // Screenshot / Record. The screenshot button leads the group, so it is the
    // active one; recording is simply not offered when the backend cannot do it.
    let screenshot_toggle = gtk::ToggleButton::builder().label("Screenshot").build();
    let record_toggle = gtk::ToggleButton::builder().label("Record").build();
    record_toggle.set_group(Some(&screenshot_toggle));
    record_toggle.set_sensitive(recording_allowed);
    let mode_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .build();
    mode_row.add_css_class("linked");
    mode_row.append(&screenshot_toggle);
    mode_row.append(&record_toggle);
    card.append(&mode_row);

    // Screen / Window / Area.
    let targets = gtk::DropDown::from_strings(&TARGET_NAMES);
    targets.set_selected(target_index(prefs.target));
    let target_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .build();
    let target_label = gtk::Label::builder().label("Capture").build();
    target_label.set_halign(gtk::Align::Start);
    target_row.append(&target_label);
    target_row.append(&targets);
    card.append(&target_row);

    // Cancel / Capture.
    let cancel = gtk::Button::builder().label("Cancel").build();
    let capture = gtk::Button::builder().label("Capture").build();
    capture.add_css_class("suggested-action");
    window.set_default_widget(Some(&capture));
    capture.grab_focus();
    let action_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .halign(gtk::Align::End)
        .build();
    action_row.add_css_class("linked");
    action_row.append(&cancel);
    action_row.append(&capture);
    card.append(&action_row);

    card.update_property(&[gtk::accessible::Property::Label("Screenshot or record")]);
    let center = gtk::CenterBox::new();
    center.set_center_widget(Some(&card));
    window.set_child(Some(&center));

    let once: Rc<RefCell<Option<Done>>> = Rc::new(RefCell::new(Some(Box::new(on_choice))));

    let monitor_name = shell::monitor_name(&monitor);
    let window_for_capture = window.clone();
    let once_for_capture = once.clone();
    capture.connect_clicked(move |_| {
        let recording = record_toggle.is_active();
        let choice = match targets.selected() {
            // A window is captured through the portal, which hands over the
            // window the user picks.
            1 => Choice::Still(Target::portal()),
            2 => Choice::Area,
            _ => {
                let source = Target::monitor(&monitor_name);
                if recording {
                    Choice::Video(source)
                } else {
                    Choice::Still(source)
                }
            }
        };
        finish(&window_for_capture, &once_for_capture, choice);
    });

    let window_for_cancel = window.clone();
    let once_for_cancel = once;
    cancel.connect_clicked(move |_| {
        finish(&window_for_cancel, &once_for_cancel, Choice::Cancel);
    });

    window.present();
}

fn finish(window: &gtk::Window, once: &Rc<RefCell<Option<Done>>>, choice: Choice) {
    shell::dismiss(window);
    if let Some(done) = once.borrow_mut().take() {
        done(choice);
    }
}

fn target_index(target: CaptureTarget) -> u32 {
    match target {
        CaptureTarget::Screen => 0,
        CaptureTarget::Window => 1,
        CaptureTarget::Area => 2,
    }
}
