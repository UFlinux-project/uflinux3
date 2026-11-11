//! Keep exactly one desktop process resident for the lifetime of the compositor.
//!
//! The session starts this mode with `--supervise` instead of the desktop
//! directly. It restarts a desktop that exits unexpectedly, backing off so a
//! broken upgrade cannot spin, and stops once the compositor is gone so no
//! window owner outlives its session. Doing this here rather than in a shell
//! script keeps the session's process tree, and its memory, smaller.
use std::{
    io,
    os::fd::RawFd,
    path::Path,
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicBool, AtomicI32, Ordering},
    time::{Duration, Instant},
};

type Pid = libc::pid_t;

/// Longest wait between restart attempts.
const MAX_DELAY: Duration = Duration::from_secs(30);
/// Shortest wait, and the granularity of the shutdown check while waiting.
const TICK: Duration = Duration::from_secs(1);

// Signal handlers may only use async-signal-safe operations, so the state they
// touch lives in atomics allocated before any handler can run.
static CHILD: AtomicI32 = AtomicI32::new(0);
static STOPPED: AtomicBool = AtomicBool::new(false);
static SIGNAL_READ: AtomicI32 = AtomicI32::new(-1);
static SIGNAL_WRITE: AtomicI32 = AtomicI32::new(-1);

/// The compositor pid the session handed us, if it names a process at all.
fn compositor_pid(raw: Option<&str>) -> Option<Pid> {
    raw?.trim().parse::<Pid>().ok().filter(|pid| *pid > 0)
}

/// Run the supervision loop for one compositor until the session is over.
fn supervise(
    compositor: Pid,
    program: &Path,
    arguments: &[&str],
    maximum: Duration,
    tick: Duration,
) -> i32 {
    let mut delay = tick;
    while alive(compositor) && !stopped() {
        match start(program, arguments) {
            Err(error) => {
                eprintln!("uflinux-desktop: could not start the session process: {error}");
            }
            Ok(mut child) => {
                // The desktop exits on its own once the compositor is gone, so
                // blocking here is enough to notice the end of the session.
                match child.wait() {
                    Ok(status) if status.success() => return 0,
                    Ok(_) => {}
                    Err(error) => {
                        eprintln!("uflinux-desktop: could not follow the session process: {error}");
                    }
                }
                CHILD.store(0, Ordering::SeqCst);
            }
        }
        // A desktop that exits cleanly ended the session rather than crashing.
        if !wait_out(delay, tick, compositor) {
            return 0;
        }
        delay = (delay * 2).min(maximum);
    }
    0
}

fn start(program: &Path, arguments: &[&str]) -> io::Result<Child> {
    let child = Command::new(program)
        .args(arguments)
        // The supervisor owns the session's lifetime, not its input.
        .stdin(Stdio::null())
        .spawn()?;
    CHILD.store(child.id() as Pid, Ordering::SeqCst);
    Ok(child)
}

/// Spend a restart pause, and report whether supervision should continue.
fn wait_out(delay: Duration, tick: Duration, compositor: Pid) -> bool {
    let deadline = Instant::now() + delay;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining == Duration::ZERO {
            return true;
        }
        sleep(tick.min(remaining));
        if !alive(compositor) || stopped() {
            return false;
        }
    }
}

fn alive(compositor: Pid) -> bool {
    if unsafe { libc::kill(compositor, 0) } == 0 {
        return true;
    }
    // A compositor owned by another user still exists as far as this session is
    // concerned; only its absence means the session is over.
    io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

fn stopped() -> bool {
    STOPPED.load(Ordering::SeqCst)
}

unsafe extern "C" fn interrupted(_signal: libc::c_int) {
    STOPPED.store(true, Ordering::SeqCst);
    let child = CHILD.load(Ordering::SeqCst);
    if child > 0 {
        unsafe { libc::kill(child, libc::SIGTERM) };
    }
    let writer = SIGNAL_WRITE.load(Ordering::SeqCst);
    if writer >= 0 {
        let byte: u8 = 1;
        unsafe { libc::write(writer, (&byte as *const u8).cast(), 1) };
    }
}

/// Wait for a signal, or for the tick to pass.
fn sleep(tick: Duration) {
    let reader = SIGNAL_READ.load(Ordering::SeqCst);
    if reader < 0 {
        std::thread::sleep(tick);
        return;
    }
    let mut poll = libc::pollfd {
        fd: reader,
        events: libc::POLLIN,
        revents: 0,
    };
    let timeout = i32::try_from(tick.as_millis()).unwrap_or(i32::MAX);
    if unsafe { libc::poll(&mut poll, 1, timeout) } > 0 {
        let mut drained = [0u8; 64];
        while unsafe { libc::read(reader, (&mut drained as *mut u8).cast(), 64) } == 64 {}
    }
}

/// Install the shutdown handlers. Failing here is fatal, because a supervisor
/// that a logout cannot stop would keep the desktop resident forever.
fn handle_shutdown() -> Result<(), String> {
    let mut ends = [0 as RawFd; 2];
    // A self-pipe is the only async-signal-safe way to leave the wait early.
    if unsafe { libc::pipe(ends.as_mut_ptr()) } == 0 {
        SIGNAL_READ.store(ends[0], Ordering::SeqCst);
        SIGNAL_WRITE.store(ends[1], Ordering::SeqCst);
    }
    for signal in [libc::SIGHUP, libc::SIGINT, libc::SIGTERM] {
        if unsafe { libc::signal(signal, interrupted as *const () as libc::sighandler_t) }
            == libc::SIG_ERR
        {
            return Err(io::Error::last_os_error().to_string());
        }
    }
    Ok(())
}

/// Entry point for `uflinux-desktop --supervise`; returns the exit status.
pub fn run() -> i32 {
    let Some(compositor) = compositor_pid(std::env::var("LABWC_PID").ok().as_deref()) else {
        eprintln!("uflinux-desktop: LABWC_PID is required");
        return 1;
    };
    let program = match std::env::current_exe() {
        Ok(program) => program,
        Err(error) => {
            eprintln!("uflinux-desktop: could not locate this program: {error}");
            return 1;
        }
    };
    if let Err(error) = handle_shutdown() {
        eprintln!("uflinux-desktop: could not install a signal handler: {error}");
        return 1;
    }
    supervise(compositor, &program, &["--daemon"], MAX_DELAY, TICK)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

    #[test]
    fn accepts_only_a_real_pid() {
        for (raw, expected) in [
            (None, None),
            (Some(""), None),
            (Some("labwc"), None),
            (Some("0"), None),
            (Some("-1"), None),
            (Some("42"), Some(42)),
            (Some(" 42 "), Some(42)),
        ] {
            assert_eq!(compositor_pid(raw), expected, "{raw:?}");
        }
    }

    /// A compositor that exits on its own. Waiting for it in the background
    /// keeps it from lingering as a zombie, which `kill(pid, 0)` would report
    /// as alive.
    fn compositor(seconds: &str) -> Pid {
        let mut child = Command::new("sleep")
            .arg(seconds)
            .spawn()
            .expect("compositor");
        let pid = child.id() as Pid;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        pid
    }

    /// A child that records one start, then exits with the given status.
    fn desktop(directory: &str, code: &str) -> (PathBuf, PathBuf) {
        let directory = std::env::temp_dir().join(format!("uflinux-supervisor-{}", directory));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).expect("directory");
        let counter = directory.join("starts");
        let program = directory.join("desktop");
        fs::write(
            &program,
            format!(
                "#!/bin/sh\nprintf start >> '{}'\nexit {}\n",
                counter.display(),
                code
            ),
        )
        .expect("script");
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).expect("mode");
        (program, counter)
    }

    fn starts(path: &Path) -> usize {
        fs::read_to_string(path)
            .map(|raw| raw.matches("start").count())
            .unwrap_or_default()
    }

    #[test]
    fn exits_after_a_clean_desktop_exit() {
        let (program, counter) = desktop("supervisor-clean", "0");
        let began = Instant::now();
        assert_eq!(
            supervise(
                compositor("30"),
                &program,
                &[],
                MAX_DELAY,
                Duration::from_millis(10)
            ),
            0
        );
        assert_eq!(starts(&counter), 1, "the session was restarted");
        assert!(
            began.elapsed() < TICK,
            "a clean exit was treated as a crash"
        );
    }

    #[test]
    fn never_starts_a_desktop_the_compositor_cannot_see() {
        let (program, counter) = desktop("supervisor-gone", "1");
        // A process that has already exited and been waited for is gone.
        let mut exited = Command::new("true").spawn().expect("compositor");
        let pid = exited.id() as Pid;
        exited.wait().expect("reap the compositor");
        assert_eq!(supervise(pid, &program, &[], MAX_DELAY, TICK), 0);
        assert_eq!(starts(&counter), 0, "a dead compositor kept a desktop");
    }

    #[test]
    fn restarts_a_crashing_desktop_with_doubling_backoff() {
        let (program, counter) = desktop("supervisor-crash", "1");
        // The compositor outlives the first two pauses but not the third, so a
        // fixed one-second pause would have reached a fourth start by now.
        let began = Instant::now();
        assert_eq!(
            supervise(compositor("4"), &program, &[], MAX_DELAY, TICK),
            0
        );
        let elapsed = began.elapsed();
        assert_eq!(starts(&counter), 3, "restart backoff is not doubling");
        assert!(elapsed >= Duration::from_secs(3), "backoff was skipped");
        assert!(
            elapsed < Duration::from_secs(6),
            "backoff outlived the session"
        );
    }

    #[test]
    fn a_missing_compositor_pid_is_a_startup_error() {
        unsafe { std::env::set_var("LABWC_PID", "not-a-pid") };
        assert_eq!(run(), 1, "the supervisor accepted an unusable pid");
    }
}
