//! The only place that knows gpu-screen-recorder's command line.
//!
//! The argument builder is pure and unit-tested so the preset mapping can be
//! checked without a GPU. The recorder itself is a plain child process: the tool
//! is its parent, which is what keeps the backend daemonless.
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gtk::glib;
use settings_core::{AudioSource, CaptureQuality, ScreenCapture};

/// Program name installed by the gpu-screen-recorder package.
pub const BACKEND: &str = "gpu-screen-recorder";

/// Wayland capture source understood by the backend.
pub const PORTAL_SOURCE: &str = "portal";

pub const EXIT_OK: i32 = 0;
pub const EXIT_USAGE: i32 = 1;
pub const EXIT_INVALID_ARGS: i32 = 2;

/// How long to wait for the recorder to close the file after a stop request.
/// The backend answers `stop` only once the muxer is done, which can take a
/// moment on a long recording.
const STOP_TIMEOUT: Duration = Duration::from_secs(30);

/// Pause and resume are answered immediately, so a short timeout keeps a wedged
/// socket from freezing the UI.
const QUICK_TIMEOUT: Duration = Duration::from_millis(500);

/// What to capture. `monitor` is a GTK connector name ("DP-1") or
/// `PORTAL_SOURCE`. `region` is in GTK logical coordinates: the backend locates
/// the monitor from the region centre and applies the scale itself, so the
/// values are passed through unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub monitor: String,
    pub region: Option<(i32, i32, i32, i32)>,
}

impl Target {
    pub fn portal() -> Self {
        Self {
            monitor: PORTAL_SOURCE.to_string(),
            region: None,
        }
    }

    pub fn monitor(name: &str) -> Self {
        Self {
            monitor: name.to_string(),
            region: None,
        }
    }

    pub fn region(name: &str, x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            monitor: name.to_string(),
            region: Some((x, y, width, height)),
        }
    }

    /// The `-w` value. A region uses the inline `WxH+X+Y` form, which the backend
    /// prefers over the deprecated `-region` option.
    fn capture_source(&self) -> String {
        match self.region {
            Some((x, y, width, height)) => format!("{width}x{height}+{x}+{y}"),
            None => self.monitor.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Job {
    pub target: Target,
    pub path: PathBuf,
    pub prefs: ScreenCapture,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Saved(PathBuf),
    Failed(String),
}

fn no_yes(flag: bool) -> &'static str {
    if flag { "no" } else { "yes" }
}

/// Common flags for both stills and recordings.
fn capture_flags(job: &Job) -> Vec<String> {
    vec![
        "-w".to_string(),
        job.target.capture_source(),
        "-o".to_string(),
        job.path.to_string_lossy().into_owned(),
        "-cursor".to_string(),
        no_yes(!job.prefs.include_cursor).to_string(),
        // "-v" is the FPS/damage readout, not verbosity; it would spam stderr.
        "-v".to_string(),
        "no".to_string(),
    ]
}

/// Quality layer, mirroring `add_quality_command_line_args` in the official GTK
/// frontend: named quality, or a constant bitrate when the user picks "Bitrate".
fn quality_flags(prefs: &ScreenCapture) -> Vec<String> {
    match prefs.quality {
        CaptureQuality::ConstantBitrate => vec![
            "-bm".to_string(),
            "cbr".to_string(),
            "-q".to_string(),
            prefs.bitrate_kbps.to_string(),
        ],
        quality => vec!["-q".to_string(), quality.id().to_string()],
    }
}

/// Advanced layer. A `None`/`Auto` value means "Backend default" and is left off
/// the command line entirely, which keeps the tool honest when the backend
/// changes its own defaults.
fn advanced_flags(prefs: &ScreenCapture) -> Vec<String> {
    let mut argv = Vec::new();
    if let Some(codec) = prefs.video_codec.as_deref() {
        argv.extend(["-k".to_string(), codec.to_string()]);
    }
    if let Some(mode) = prefs.framerate_mode.backend() {
        argv.extend(["-fm".to_string(), mode.to_string()]);
    }
    if let Some(range) = prefs.color_range.backend() {
        argv.extend(["-cr".to_string(), range.to_string()]);
    }
    if let Some(container) = prefs.container.as_deref() {
        argv.extend(["-c".to_string(), container.to_string()]);
    }
    if let Some(codec) = prefs.audio_codec.as_ref() {
        argv.extend(["-ac".to_string(), codec.id().to_string()]);
    }
    argv
}

/// Audio layer. `-a` is a repeatable list argument, so each source is its own
/// pair of flags.
fn audio_flags(prefs: &ScreenCapture) -> Vec<String> {
    let mut argv = Vec::new();
    match prefs.audio_source {
        AudioSource::None => {}
        AudioSource::Output | AudioSource::Input => {
            if let Some(source) = prefs.audio_source.backend() {
                argv.extend(["-a".to_string(), source.to_string()]);
            }
        }
        AudioSource::OutputAndInput => {
            argv.extend(["-a".to_string(), "default_output".to_string()]);
            argv.extend(["-a".to_string(), "default_input".to_string()]);
        }
    }
    argv
}

/// Flags for a single still image. The backend picks the encoder from the `-o`
/// extension, so no container or quality flags apply here.
pub fn screenshot_args(job: &Job) -> Vec<String> {
    capture_flags(job)
}

/// Flags for a video recording. `socket` enables the IPC control channel; when
/// it is `None` the recorder can only be stopped with a signal.
pub fn recording_args(job: &Job, socket: Option<&Path>) -> Vec<String> {
    let mut argv = capture_flags(job);
    argv.extend(["-f".to_string(), job.prefs.fps.to_string()]);
    argv.extend(quality_flags(&job.prefs));
    argv.extend(audio_flags(&job.prefs));
    argv.extend(advanced_flags(&job.prefs));
    if let Some(socket) = socket {
        argv.extend(["-ipc".to_string(), socket.to_string_lossy().into_owned()]);
    }
    argv
}

fn program() -> Option<PathBuf> {
    glib::find_program_in_path(BACKEND)
}

/// True when the recorder is installed. The overlay uses this to hide recording
/// rather than failing after the user presses the button.
pub fn backend_available() -> bool {
    program().is_some()
}

/// Runs the backend for a still image and reports the result. Blocking, so it is
/// called from a worker thread.
pub fn screenshot(job: &Job) -> Outcome {
    let Some(program) = program() else {
        return Outcome::Failed(format!("{BACKEND} is not installed"));
    };
    let output = match Command::new(program)
        .args(screenshot_args(job))
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
    {
        Ok(output) => output,
        Err(error) => return Outcome::Failed(format!("Cannot start recorder: {error}")),
    };
    match output.status.code() {
        Some(EXIT_OK) => Outcome::Saved(job.path.clone()),
        Some(EXIT_USAGE | EXIT_INVALID_ARGS) => Outcome::Failed(
            first_line(&output.stderr).unwrap_or_else(|| "Bad recorder arguments".to_string()),
        ),
        Some(code) => Outcome::Failed(recorder_error(code, &output.stderr)),
        None => Outcome::Failed("Recorder was interrupted".to_string()),
    }
}

/// The backend reports failures as exit codes; the human-readable reason is on
/// stderr, so show the last message there when there is one.
fn recorder_error(code: i32, stderr: &[u8]) -> String {
    match last_log_line(stderr) {
        Some(detail) => detail,
        None => format!("Recorder failed (exit {code})"),
    }
}

fn first_line(stderr: &[u8]) -> Option<String> {
    String::from_utf8_lossy(stderr)
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(|line| line.trim().to_string())
}

fn last_log_line(stderr: &[u8]) -> Option<String> {
    String::from_utf8_lossy(stderr)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .next_back()
        .map(|line| {
            let line = line.trim();
            line.strip_prefix("gsr error: ").unwrap_or(line).to_string()
        })
}

/// A running recorder. The child is reaped in a worker thread and the result is
/// delivered back on the main context, so the HUD stays responsive for the whole
/// recording.
#[derive(Clone, Debug)]
pub struct Recorder {
    pid: u32,
    socket: Option<PathBuf>,
    path: PathBuf,
    stopping: Arc<AtomicBool>,
    /// Path reported by the IPC `stop` request, when it got one.
    saved: Arc<Mutex<Option<PathBuf>>>,
}

impl Recorder {
    /// Spawns the recorder and calls `on_exit` on the main context when it is
    /// gone. `on_exit` receives the exit code, or `None` when the recorder was
    /// killed by a signal.
    pub fn start(
        job: &Job,
        socket: Option<&Path>,
        on_exit: impl FnOnce(Option<i32>) + Send + 'static,
    ) -> Result<Self, String> {
        let Some(program) = program() else {
            return Err(format!("{BACKEND} is not installed"));
        };
        let mut child = Command::new(program)
            .args(recording_args(job, socket))
            .stdin(Stdio::null())
            // The recorder logs to stderr for the whole recording. Draining the
            // pipe would need a second thread and a full pipe would stall the
            // capture, so its output goes to the void and failures are reported
            // by exit code.
            .stderr(Stdio::null())
            .stdout(Stdio::null())
            .spawn()
            .map_err(|error| format!("Cannot start recorder: {error}"))?;
        let pid = child.id();
        let context = glib::MainContext::default();
        std::thread::spawn(move || {
            let code = child.wait().ok().and_then(|status| status.code());
            let _ = context.invoke(move || on_exit(code));
        });
        Ok(Self {
            pid,
            socket: socket.map(Path::to_path_buf),
            path: job.path.clone(),
            stopping: Arc::new(AtomicBool::new(false)),
            saved: Arc::new(Mutex::new(None)),
        })
    }

    /// Asks the recorder to finish the file. The blocking IPC call runs in a
    /// worker thread; the exit callback is what reports the outcome.
    pub fn stop(&self) {
        self.stopping.store(true, Ordering::Relaxed);
        let Some(socket) = self.socket.clone() else {
            kill(self.pid, libc::SIGINT);
            return;
        };
        let saved = self.saved.clone();
        let pid = self.pid;
        std::thread::spawn(
            move || match ipc_request(&socket, "stop", None, STOP_TIMEOUT) {
                Ok(reply) => {
                    if let Some(path) = reply_path(&reply) {
                        *saved.lock().unwrap() = Some(path);
                    }
                }
                Err(error) => {
                    eprintln!("uflinux-screencapture: IPC stop failed: {error}, using SIGINT");
                    kill(pid, libc::SIGINT);
                }
            },
        );
    }

    /// Pauses or resumes the capture. Falls back to SIGUSR2, which the backend
    /// handles as a pause toggle, when there is no control socket.
    pub fn set_paused(&self, paused: bool) -> Result<(), String> {
        match &self.socket {
            Some(socket) => {
                let data = if paused { "true" } else { "false" };
                ipc_request(socket, "set-paused", Some(data), QUICK_TIMEOUT).map(|_| ())
            }
            None => {
                kill(self.pid, libc::SIGUSR2);
                Ok(())
            }
        }
    }

    /// Asks the recorder to finish the file and forgets about it, so that this
    /// process can exit while the recorder writes out the last frames. Used when
    /// the session terminates us in the middle of a recording.
    pub fn terminate(self) {
        self.stopping.store(true, Ordering::Relaxed);
        kill(self.pid, libc::SIGINT);
        std::mem::forget(self);
    }

    /// Called from the exit callback: the file is closed by now, so the path is
    /// final. Prefers the path the recorder reported over the one we asked for.
    pub fn outcome(&self, code: Option<i32>) -> Outcome {
        match code {
            Some(EXIT_OK) => {
                let path = self
                    .saved
                    .lock()
                    .unwrap()
                    .take()
                    .unwrap_or_else(|| self.path.clone());
                if path.exists() {
                    Outcome::Saved(path)
                } else {
                    Outcome::Failed("Recorder did not write the file".to_string())
                }
            }
            Some(code) if code > 0 => Outcome::Failed(format!("Recorder failed (exit {code})")),
            _ => Outcome::Failed("Recording was interrupted".to_string()),
        }
    }
}

fn kill(pid: u32, signal: i32) -> bool {
    // SAFETY: kill(2) on our own child is always safe; failure just means it is
    // already gone.
    unsafe { libc::kill(pid as libc::pid_t, signal) == 0 }
}

fn ipc_request(
    socket: &Path,
    name: &str,
    data: Option<&str>,
    timeout: Duration,
) -> Result<String, String> {
    let mut stream = UnixStream::connect(socket).map_err(|error| error.to_string())?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|error| error.to_string())?;
    let payload = match data {
        Some(data) => format!("{{\"id\":1,\"name\":\"{name}\",\"data\":{data}}}\n"),
        None => format!("{{\"id\":1,\"name\":\"{name}\"}}\n"),
    };
    stream
        .write_all(payload.as_bytes())
        .map_err(|error| error.to_string())?;
    stream.flush().map_err(|error| error.to_string())?;
    let reply = read_reply(&mut stream)?;
    if reply.contains("\"result\":\"ok\"") {
        Ok(reply)
    } else {
        Err(ipc_error(&reply))
    }
}

/// Reads the reply to a request. Deferred names such as `stop` are answered only
/// after the muxer has closed the file.
fn read_reply(stream: &mut UnixStream) -> Result<String, String> {
    let mut buffer = String::new();
    let mut chunk = [0u8; 4096];
    loop {
        let read = stream.read(&mut chunk).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        buffer.push_str(&String::from_utf8_lossy(&chunk[..read]));
        if buffer.contains('\n') {
            break;
        }
    }
    if buffer.trim().is_empty() {
        return Err("Recorder exited before the request finished".to_string());
    }
    Ok(buffer)
}

fn ipc_error(reply: &str) -> String {
    reply_text(reply).unwrap_or_else(|| format!("Unexpected recorder reply: {reply}"))
}

fn reply_text(reply: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(reply.trim())
        .ok()?
        .get("data")
        .and_then(|data| data.as_str())
        .map(str::to_string)
}

fn reply_path(reply: &str) -> Option<PathBuf> {
    reply_text(reply)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use settings_core::{ColorRange, FramerateMode};

    fn job(extension: &str) -> Job {
        Job {
            target: Target::monitor("DP-1"),
            path: PathBuf::from(format!("/tmp/shot.{extension}")),
            prefs: ScreenCapture::default(),
        }
    }

    fn flag<'a>(argv: &'a [String], key: &str) -> Option<&'a str> {
        argv.iter()
            .position(|arg| arg == key)
            .and_then(|index| argv.get(index + 1))
            .map(String::as_str)
    }

    #[test]
    fn screenshot_uses_monitor_and_webp() {
        let argv = screenshot_args(&job("webp"));
        assert_eq!(flag(&argv, "-w"), Some("DP-1"));
        assert_eq!(flag(&argv, "-o"), Some("/tmp/shot.webp"));
        assert_eq!(flag(&argv, "-cursor"), Some("yes"));
        assert_eq!(flag(&argv, "-v"), Some("no"));
        assert!(!argv.iter().any(|arg| arg == "-f"));
    }

    #[test]
    fn area_uses_inline_region_capture_source() {
        let mut j = job("webp");
        j.target = Target::region("DP-1", 10, 20, 300, 400);
        assert_eq!(flag(&screenshot_args(&j), "-w"), Some("300x400+10+20"));
    }

    #[test]
    fn window_uses_portal() {
        let mut j = job("webp");
        j.target = Target::portal();
        assert_eq!(flag(&screenshot_args(&j), "-w"), Some("portal"));
    }

    #[test]
    fn cursor_can_be_dropped() {
        let mut j = job("mp4");
        j.prefs.include_cursor = false;
        assert_eq!(flag(&recording_args(&j, None), "-cursor"), Some("no"));
    }

    #[test]
    fn recording_defaults_match_backend() {
        let argv = recording_args(&job("mp4"), None);
        assert_eq!(flag(&argv, "-f"), Some("60"));
        assert_eq!(flag(&argv, "-q"), Some("very_high"));
        assert_eq!(flag(&argv, "-c"), None);
        assert_eq!(flag(&argv, "-k"), None);
        assert_eq!(flag(&argv, "-fm"), None);
        assert_eq!(flag(&argv, "-cr"), None);
        assert_eq!(flag(&argv, "-a"), None);
        assert_eq!(flag(&argv, "-ac"), None);
    }

    #[test]
    fn named_quality_maps_to_backend_names() {
        for (quality, name) in [
            (CaptureQuality::Medium, "medium"),
            (CaptureQuality::High, "high"),
            (CaptureQuality::VeryHigh, "very_high"),
            (CaptureQuality::Ultra, "ultra"),
        ] {
            let mut j = job("mp4");
            j.prefs.quality = quality;
            let argv = recording_args(&j, None);
            assert_eq!(flag(&argv, "-q"), Some(name));
            assert_eq!(flag(&argv, "-bm"), None);
        }
    }

    #[test]
    fn bitrate_quality_switches_to_cbr() {
        let mut j = job("mp4");
        j.prefs.quality = CaptureQuality::ConstantBitrate;
        j.prefs.bitrate_kbps = 25000;
        let argv = recording_args(&j, None);
        assert_eq!(flag(&argv, "-bm"), Some("cbr"));
        assert_eq!(flag(&argv, "-q"), Some("25000"));
    }

    #[test]
    fn audio_sources_are_repeated() {
        let mut j = job("mp4");
        j.prefs.audio_source = AudioSource::Output;
        assert_eq!(
            flag(&recording_args(&j, None), "-a"),
            Some("default_output")
        );
        j.prefs.audio_source = AudioSource::Input;
        assert_eq!(flag(&recording_args(&j, None), "-a"), Some("default_input"));
        j.prefs.audio_source = AudioSource::OutputAndInput;
        let argv = recording_args(&j, None);
        let sources: Vec<&str> = argv
            .iter()
            .enumerate()
            .filter(|(_, arg)| *arg == "-a")
            .filter_map(|(index, _)| argv.get(index + 1))
            .map(String::as_str)
            .collect();
        assert_eq!(sources, vec!["default_output", "default_input"]);
        j.prefs.audio_source = AudioSource::None;
        assert!(!recording_args(&j, None).iter().any(|arg| arg == "-a"));
    }

    #[test]
    fn advanced_values_are_passed_through() {
        let mut j = job("mkv");
        j.prefs.video_codec = Some("hevc".to_string());
        j.prefs.framerate_mode = FramerateMode::Constant;
        j.prefs.color_range = ColorRange::Full;
        j.prefs.container = Some("mkv".to_string());
        let argv = recording_args(&j, None);
        assert_eq!(flag(&argv, "-k"), Some("hevc"));
        assert_eq!(flag(&argv, "-fm"), Some("cfr"));
        assert_eq!(flag(&argv, "-cr"), Some("full"));
        assert_eq!(flag(&argv, "-c"), Some("mkv"));
    }

    #[test]
    fn ipc_socket_is_last() {
        let socket = Path::new("/run/user/1000/uflinux-screencapture-1.sock");
        let argv = recording_args(&job("mp4"), Some(socket));
        assert_eq!(
            argv.last().map(String::as_str),
            Some(socket.to_str().unwrap())
        );
        assert_eq!(&argv[argv.len() - 2], "-ipc");
    }

    #[test]
    fn reply_path_comes_from_data() {
        assert_eq!(
            reply_path("{\"id\":1,\"result\":\"ok\",\"data\":\"/tmp/a.mp4\"}"),
            Some(PathBuf::from("/tmp/a.mp4"))
        );
        assert_eq!(reply_path("{\"id\":1,\"result\":\"ok\"}"), None);
    }

    #[test]
    fn error_reply_data_is_the_message() {
        assert_eq!(
            ipc_error("{\"id\":1,\"result\":\"error\",\"data\":\"already stopping\"}"),
            "already stopping"
        );
    }

    #[test]
    fn stderr_log_lines_are_readable() {
        let stderr = b"gsr info: hello\ngsr error: monitor not found\n";
        assert_eq!(last_log_line(stderr).as_deref(), Some("monitor not found"));
        assert_eq!(first_line(stderr).as_deref(), Some("gsr info: hello"));
    }
}
