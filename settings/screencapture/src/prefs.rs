//! Reads user preferences from uflinux-settingsd over D-Bus.
//!
//! The tool never writes preferences and never talks to the daemon again after
//! this single call, which keeps it daemonless: if the daemon is not running the
//! compiled-in defaults are used and the capture still works.
use settings_core::{ScreenCapture, Snapshot};

/// A blocking `GetState` is safe here: it happens once, before any window is
/// shown, and the daemon answers from memory. The timeout is short so a hung
/// daemon degrades into defaults instead of freezing the PrintScreen key.
const CALL_TIMEOUT_MS: i32 = 2000;

pub fn load() -> ScreenCapture {
    match state() {
        Ok(json) => serde_json::from_str::<Snapshot>(&json)
            .map(|snapshot| snapshot.screen_capture)
            .unwrap_or_default(),
        Err(_) => ScreenCapture::default(),
    }
}

fn state() -> settings_core::Result<String> {
    let connection = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)
        .map_err(|error| error.to_string())?;
    let reply = connection
        .call_sync(
            Some(settings_core::SESSION_NAME),
            settings_core::SESSION_PATH,
            settings_core::SESSION_NAME,
            "GetState",
            Option::<&glib::Variant>::None,
            Some(glib::VariantTy::STRING),
            gio::DBusCallFlags::NONE,
            CALL_TIMEOUT_MS,
            gio::Cancellable::NONE,
        )
        .map_err(|error| error.to_string())?;
    reply
        .get::<(String,)>()
        .map(|(json,)| json)
        .ok_or_else(|| "Bad GetState reply".to_string())
}
