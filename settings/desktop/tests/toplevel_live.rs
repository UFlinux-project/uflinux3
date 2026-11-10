//! Run with `cargo test -p uflinux-desktop --test toplevel_live -- --ignored`
//! inside a disposable labwc session with `foot` installed.
use std::{
    process::{Child, Command as Process, Stdio},
    time::{Duration, Instant},
};
use uflinux_desktop::toplevel::{Client, Command, Snapshot};

struct Terminal(Child);
impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn until(
    client: &Client,
    notifications: &async_channel::Receiver<()>,
    predicate: impl Fn(&Snapshot) -> bool,
) -> Snapshot {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let snapshot = client.snapshot();
        assert!(
            snapshot.error.is_none(),
            "Window tracker failed: {:?}",
            snapshot.error
        );
        if predicate(&snapshot) {
            return snapshot;
        }
        let timeout = deadline.saturating_duration_since(Instant::now());
        assert!(
            !timeout.is_zero(),
            "Timed out waiting for window state: {snapshot:?}"
        );
        match notifications.try_recv() {
            Ok(()) => {}
            Err(async_channel::TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(5)),
            Err(error) => panic!("Window tracker stopped: {error}"),
        }
    }
}
#[test]
#[ignore = "requires a disposable Wayland compositor and foot"]
fn tracks_and_controls_real_window() {
    let (mut client, notifications) = Client::start().expect("start tracker");
    let _terminal = Terminal(
        Process::new("foot")
            .args([
                "--app-id=uflinux-desktop-test",
                "--title=UFLinux tracker fixture",
                "sh",
                "-c",
                "sleep 60",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start foot"),
    );
    let _second_terminal = Terminal(
        Process::new("foot")
            .args([
                "--app-id=uflinux-desktop-test",
                "--title=UFLinux second tracker fixture",
                "sh",
                "-c",
                "sleep 60",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start second foot"),
    );
    let seen = until(&client, &notifications, |s| {
        s.windows
            .iter()
            .any(|w| w.app_id == "uflinux-desktop-test" && w.title == "UFLinux tracker fixture")
            && s.windows.iter().any(|w| {
                w.app_id == "uflinux-desktop-test" && w.title == "UFLinux second tracker fixture"
            })
    });
    let id = seen
        .windows
        .iter()
        .find(|w| w.title == "UFLinux tracker fixture")
        .unwrap()
        .id;
    let second_id = seen
        .windows
        .iter()
        .find(|w| w.title == "UFLinux second tracker fixture")
        .unwrap()
        .id;
    assert_ne!(id, second_id);
    client.send(Command::Minimize(id)).unwrap();
    until(&client, &notifications, |s| {
        s.windows.iter().any(|w| w.id == id && w.minimized)
    });
    client.send(Command::Activate(id)).unwrap();
    until(&client, &notifications, |s| {
        s.windows
            .iter()
            .any(|w| w.id == id && !w.minimized && w.focused)
    });
    client.send(Command::Close(id)).unwrap();
    until(&client, &notifications, |s| {
        !s.windows.iter().any(|w| w.id == id) && s.windows.iter().any(|w| w.id == second_id)
    });
    client.send(Command::Close(second_id)).unwrap();
    until(&client, &notifications, |s| {
        !s.windows.iter().any(|w| w.id == second_id)
    });
}
