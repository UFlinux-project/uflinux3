mod audio;
mod capture;
mod devices;
mod display;
mod network;
mod session;
fn main() {
    if let Err(error) = session::run() {
        eprintln!("uflinux-settingsd: {error}");
        std::process::exit(1);
    }
}
