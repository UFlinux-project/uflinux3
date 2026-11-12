// uflinux-screencapture: GNOME-style screenshot and recording entry point.
// SPDX-License-Identifier: GPL-3.0-only

mod backend;
mod clipboard;
mod flash;
mod hud;
mod output;
mod overlay;
mod prefs;
mod selection;
mod shell;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use backend::{Job, Outcome, Recorder, Target};
use gtk::gdk;
use gtk::glib;
use settings_core::{CONTAINERS, CaptureFormat, CaptureTarget, MAX_DELAY_SECONDS, ScreenCapture};

const APP_ID: &str = "org.uflinux.ScreenCapture";

const USAGE: &str = "\
uflinux-screencapture - screenshot and screen recording

Usage:
  uflinux-screencapture [options]                Show the capture chooser
  uflinux-screencapture --screenshot [options]   Capture without asking
  uflinux-screencapture --record [options]       Record without asking

Options:
  --target screen|window|area   What to capture (default: from settings)
  --delay SECONDS               Extra delay before capturing (max 30)
  --silent                      Do not show a notification on failure
  -h, --help                    Show this help

Screenshots are written to the folder configured in Settings and copied to the
clipboard. A recording is controlled from the bar shown on screen.";

/// What the process was asked to do.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Screenshot,
    Record,
}

/// Command line state.
struct Options {
    mode: Option<Mode>,
    target: Option<CaptureTarget>,
    delay: u8,
    silent: bool,
}

thread_local! {
    /// The recorder of this process, so that a shutdown signal can stop it.
    static RECORDER: RefCell<Option<Recorder>> = const { RefCell::new(None) };
    /// The control bar belonging to that recorder.
    static HUD: RefCell<Option<hud::Hud>> = const { RefCell::new(None) };
}

fn main() -> ExitCode {
    let options = match parse_args(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(message) => {
            if !message.is_empty() {
                eprintln!("uflinux-screencapture: {message}");
                eprintln!();
                eprintln!("{USAGE}");
            }
            return ExitCode::from(2);
        }
    };

    if gtk::init().is_err() {
        eprintln!("uflinux-screencapture: cannot connect to the display server");
        return ExitCode::from(1);
    }

    let monitor = shell::pointed_monitor();
    let mut prefs = prefs::load();
    if let Some(target) = options.target {
        prefs.target = target;
    }
    if let Err(error) = refresh().map(|_| ()) {
        eprintln!("uflinux-screencapture: {error}");
        return ExitCode::from(1);
    }
    shell::add_css();
    watch_shutdown_signals();

    let main_loop = glib::MainLoop::new(None, false);
    // A direct capture honours --target, so `--screenshot --target area` still asks
    // for a region. Recording a region is not offered; it falls back to the screen.
    let direct = options.mode.map(|mode| {
        let video = mode == Mode::Record;
        match (video, prefs.target) {
            (false, CaptureTarget::Window) => overlay::Choice::Still(Target::portal()),
            (true, CaptureTarget::Window) => overlay::Choice::Video(Target::portal()),
            (false, CaptureTarget::Area) => overlay::Choice::Area,
            _ => {
                let source = screen_target(monitor.as_ref());
                if video {
                    overlay::Choice::Video(source)
                } else {
                    overlay::Choice::Still(source)
                }
            }
        }
    });

    let loop_ref = main_loop.clone();
    let delay = options.delay;
    let silent = options.silent;
    let monitor_for_choice = monitor.clone();
    let on_choice = move |choice: overlay::Choice| {
        let state = match refresh() {
            Ok(state) => state,
            Err(error) => {
                finish(&loop_ref, Outcome::Failed(error), None, silent);
                return;
            }
        };
        let (prefs, directory) = state;
        let monitor = monitor_for_choice.as_ref();
        match choice {
            overlay::Choice::Cancel => loop_ref.quit(),
            overlay::Choice::Still(source) => {
                run_still(
                    &loop_ref, &prefs, &directory, delay, source, monitor, silent,
                );
            }
            overlay::Choice::Video(source) => {
                run_recording(&loop_ref, &prefs, &directory, source, monitor, silent);
            }
            overlay::Choice::Area => {
                let pointed = monitor.cloned();
                selection::pick(monitor, move |source| match source {
                    Some(source) => run_still(
                        &loop_ref,
                        &prefs,
                        &directory,
                        delay,
                        source,
                        pointed.as_ref(),
                        silent,
                    ),
                    None => loop_ref.quit(),
                });
            }
        }
    };

    if let Some(choice) = direct {
        on_choice(choice);
    } else {
        overlay::show(
            monitor.as_ref(),
            &prefs,
            backend::backend_available(),
            on_choice,
        );
    }

    main_loop.run();
    ExitCode::SUCCESS
}

/// Re-reads the settings and makes sure the output folder exists. The chooser
/// can stay open for a while, so a change made in the meantime is honoured.
fn refresh() -> Result<(ScreenCapture, PathBuf), String> {
    let prefs = prefs::load();
    let directory = output::screenshot_dir(&prefs.screenshot_dir);
    output::ensure_dir(&directory)?;
    Ok((prefs, directory))
}

/// The capture source for a whole-screen capture.
fn screen_target(monitor: Option<&gdk::Monitor>) -> Target {
    match monitor {
        Some(monitor) => Target::monitor(&shell::monitor_name(monitor)),
        None => Target::monitor(&shell::fallback_monitor_name()),
    }
}

/// Takes one screenshot, after a countdown when one is configured. The loop is
/// stopped when the backend is done, whatever the result.
fn run_still(
    main_loop: &glib::MainLoop,
    prefs: &ScreenCapture,
    directory: &Path,
    extra_delay: u8,
    target: Target,
    monitor: Option<&gdk::Monitor>,
    silent: bool,
) {
    let format = prefs.image_format;
    let path = output::unique_path(directory, &output::filename(format.extension()));
    let job = Job {
        target,
        path,
        prefs: prefs.clone(),
    };
    let copy_format = prefs.copy_to_clipboard.then_some(format);
    let delay = u64::from(prefs.delay_seconds) + u64::from(extra_delay);
    let flash = prefs.flash && delay == 0;
    let loop_ref = main_loop.clone();
    let monitor = monitor.cloned();
    let start = move || {
        glib::MainContext::default().spawn_local(async move {
            if flash {
                flash::show(monitor.as_ref());
            }
            let job_for_thread = job.clone();
            let result = gio::spawn_blocking(move || backend::screenshot(&job_for_thread)).await;
            let outcome = match result {
                Ok(outcome) => outcome,
                Err(_) => Outcome::Failed("Capture was interrupted".into()),
            };
            finish(&loop_ref, outcome, copy_format, silent);
        });
    };
    if delay > 0 {
        glib::timeout_add_local_once(Duration::from_secs(delay), start);
    } else {
        start();
    }
}

/// Starts a recording and shows the control bar. The process stays alive until
/// the backend exits, which happens when the user presses stop.
fn run_recording(
    main_loop: &glib::MainLoop,
    prefs: &ScreenCapture,
    directory: &Path,
    target: Target,
    monitor: Option<&gdk::Monitor>,
    silent: bool,
) {
    let extension = container_extension(prefs.container.as_deref().unwrap_or("mp4"));
    let path = output::unique_path(directory, &output::filename(extension));
    let job = Job {
        target,
        path,
        prefs: prefs.clone(),
    };
    let socket = output::socket_path();
    let loop_ref = main_loop.clone();
    let started = Recorder::start(&job, Some(&socket), move |code| {
        // Runs on the main context once the backend is gone. Both are taken out
        // first so that closing the bar below does not ask a finished recorder
        // to stop again.
        let recorder = RECORDER.with(|slot| slot.borrow_mut().take());
        let bar = HUD.with(|slot| slot.borrow_mut().take());
        if let Some(bar) = bar {
            bar.close();
        }
        let outcome = match recorder {
            Some(recorder) => recorder.outcome(code),
            None => Outcome::Failed("Recording was interrupted".into()),
        };
        finish(&loop_ref, outcome, None, silent);
    });
    match started {
        Ok(recorder) => {
            let bar = hud::Hud::show(&recorder, monitor);
            HUD.with(|slot| *slot.borrow_mut() = Some(bar));
            RECORDER.with(|slot| *slot.borrow_mut() = Some(recorder));
        }
        Err(message) => finish(main_loop, Outcome::Failed(message), None, silent),
    }
}

/// Reports the result of a capture and stops the process.
fn finish(
    main_loop: &glib::MainLoop,
    outcome: Outcome,
    copy_format: Option<CaptureFormat>,
    silent: bool,
) {
    match outcome {
        Outcome::Saved(path) => {
            if let Some(format) = copy_format {
                if let Err(error) = clipboard::copy(&path, format) {
                    eprintln!("uflinux-screencapture: clipboard: {error}");
                }
            }
        }
        Outcome::Failed(message) => {
            eprintln!("uflinux-screencapture: {message}");
            if !silent {
                notify("Capture failed", &message);
            }
        }
    }
    main_loop.quit();
}

/// Shows a desktop notification through notify-send, when available.
fn notify(title: &str, body: &str) {
    let Some(program) = glib::find_program_in_path("notify-send") else {
        return;
    };
    let _ = std::process::Command::new(program)
        .args(["-a", APP_ID, "-u", "normal"])
        .arg(title)
        .arg(body)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// Stops a running recording when the session manager terminates us. The
/// recorder finishes the file on its own and outlives this process.
fn watch_shutdown_signals() {
    for signal in [libc::SIGINT, libc::SIGTERM] {
        glib::unix_signal_add_local_once(signal, || {
            RECORDER.with(|slot| {
                if let Some(recorder) = slot.borrow_mut().take() {
                    recorder.terminate();
                }
            });
            std::process::exit(0);
        });
    }
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Options, String> {
    let mut options = Options {
        mode: None,
        target: None,
        delay: 0,
        silent: false,
    };
    let mut args = args.peekable();
    if args.peek().is_none() {
        return Ok(options);
    }
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return Err(String::new());
            }
            "--screenshot" | "--screenshot-now" => options.mode = Some(Mode::Screenshot),
            "--record" => options.mode = Some(Mode::Record),
            "--silent" => options.silent = true,
            "--delay" => {
                let value = args.next().ok_or("--delay needs a value")?;
                let seconds: u8 = value
                    .parse()
                    .map_err(|_| format!("invalid delay: {value}"))?;
                if seconds > MAX_DELAY_SECONDS {
                    return Err(format!("delay must be at most {MAX_DELAY_SECONDS} seconds"));
                }
                options.delay = seconds;
            }
            "--target" => {
                let value = args.next().ok_or("--target needs a value")?;
                options.target = Some(parse_target(&value)?);
            }
            other => return Err(format!("unknown option: {other}")),
        }
    }
    if options.target.is_some() && options.mode.is_none() {
        return Err("--target needs --screenshot or --record".into());
    }
    if options.mode == Some(Mode::Record) && options.target == Some(CaptureTarget::Area) {
        return Err("recording an area is not supported".into());
    }
    Ok(options)
}

fn parse_target(value: &str) -> Result<CaptureTarget, String> {
    match value {
        "screen" => Ok(CaptureTarget::Screen),
        "window" => Ok(CaptureTarget::Window),
        "area" => Ok(CaptureTarget::Area),
        other => Err(format!("unknown target: {other}")),
    }
}

/// The file extension to name a recording after. The backend chooses the
/// container from the output name, so it has to match the setting.
fn container_extension(container: &str) -> &str {
    if CONTAINERS.contains(&container) {
        container
    } else {
        "mp4"
    }
}
