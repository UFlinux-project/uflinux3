mod app;
mod auth;
mod pages;
#[path = "../../shared/popup.rs"]
mod popup;
mod widgets;
fn main() {
    // Compatibility for existing user key bindings; the launcher owns its process.
    if std::env::args().any(|arg| arg == "--start-menu") {
        use std::os::unix::process::CommandExt;
        let error = std::process::Command::new("uflinux-start-menu")
            .args(std::env::args_os().skip(1))
            .exec();
        eprintln!("Could not start the launcher: {error}");
        std::process::exit(1);
    }
    app::run();
}
