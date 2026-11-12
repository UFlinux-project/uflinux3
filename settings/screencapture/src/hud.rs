//! The bar shown while recording: elapsed time, pause and stop.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{gdk, glib};

use crate::backend::Recorder;
use crate::shell;

const TICK: Duration = Duration::from_millis(500);

pub struct Hud {
    window: gtk::Window,
    tick: RefCell<Option<glib::SourceId>>,
}

impl Hud {
    /// Show the bar over `monitor` for a running recording.
    pub fn show(recorder: &Recorder, monitor: Option<&gdk::Monitor>) -> Self {
        let window = gtk::Window::new();
        shell::top_bar(&window, monitor);

        let elapsed = gtk::Label::builder()
            .label("0:00")
            .css_classes(["title-2"])
            .build();
        let pause = gtk::ToggleButton::builder().label("Pause").build();
        let stop = gtk::Button::builder().label("Stop Recording").build();
        stop.add_css_class("destructive-action");

        let row = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .build();
        row.add_css_class("linked");
        row.append(&elapsed);
        row.append(&pause);
        row.append(&stop);

        let bar = gtk::CenterBox::new();
        bar.set_margin_top(6);
        bar.set_margin_bottom(6);
        bar.set_margin_start(6);
        bar.set_margin_end(6);
        bar.set_center_widget(Some(&row));
        window.set_child(Some(&bar));

        let started = Instant::now();
        let elapsed_for_tick = elapsed.clone();
        let tick = glib::timeout_add_local(TICK, move || {
            let seconds = started.elapsed().as_secs();
            elapsed_for_tick.set_text(&format!("{}:{:02}", seconds / 60, seconds % 60));
            glib::ControlFlow::Continue
        });

        let recorder_for_pause = recorder.clone();
        pause.connect_toggled(move |button| {
            let paused = button.is_active();
            if recorder_for_pause.set_paused(paused).is_err() {
                // The recorder refused (it may already be gone); do not lie.
                button.set_active(!paused);
            }
        });
        let recorder_for_stop = recorder.clone();
        stop.connect_clicked(move |_| recorder_for_stop.stop());

        // The bar can also go away on its own, when its output is disconnected.
        // Then there is nothing left to press, so stop the recording.
        let recorder_for_close = recorder.clone();
        window.connect_close_request(move |_| {
            recorder_for_close.stop();
            glib::Propagation::Proceed
        });

        window.present();
        Self {
            window,
            tick: RefCell::new(Some(tick)),
        }
    }

    /// Take the bar away without asking the recorder to stop again.
    pub fn close(&self) {
        if let Some(tick) = self.tick.borrow_mut().take() {
            tick.remove();
        }
        self.window.set_visible(false);
    }
}

impl Drop for Hud {
    fn drop(&mut self) {
        // Without this the clock keeps running for the rest of the process.
        if let Some(tick) = self.tick.borrow_mut().take() {
            tick.remove();
        }
    }
}
