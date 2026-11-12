//! Copies the saved capture to the clipboard, the way GNOME Screenshot does:
//! the image itself plus its file location, so a paste into an editor gets the
//! picture while a paste into a file manager gets the file.
//!
//! The bytes are handed to the clipboard directly instead of being decoded into
//! a `GdkTexture`. Alpine ships no gdk-pixbuf WebP loader, so decoding the
//! default format would fail, and the clipboard only needs the encoded bytes.
use std::path::Path;

use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use settings_core::CaptureFormat;

fn mime_type(format: CaptureFormat) -> &'static str {
    match format {
        CaptureFormat::Png => "image/png",
        CaptureFormat::Jpeg => "image/jpeg",
        CaptureFormat::Webp => "image/webp",
    }
}

/// Returns `Err` when the file cannot be read; a clipboard failure is never
/// fatal because the file is already saved.
pub fn copy(path: &Path, format: CaptureFormat) -> settings_core::Result<()> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let image = gdk::ContentProvider::for_bytes(mime_type(format), &glib::Bytes::from(&bytes[..]));
    let provider = match uri_list(path) {
        Some(list) => gdk::ContentProvider::new_union(&[
            image,
            gdk::ContentProvider::for_bytes("text/uri-list", &glib::Bytes::from(list.as_bytes())),
        ]),
        None => image,
    };
    let Some(clipboard) = gdk::Display::default().map(|display| display.clipboard()) else {
        return Ok(());
    };
    clipboard
        .set_content(Some(&provider))
        .map_err(|error| error.to_string())?;
    // Asks the clipboard to take ownership of the data now, so the file can be
    // renamed or deleted later without breaking the paste.
    clipboard.store_async(glib::Priority::DEFAULT, gio::Cancellable::NONE, |result| {
        if let Err(error) = result {
            eprintln!("uflinux-screencapture: clipboard store failed: {error}");
        }
    });
    Ok(())
}

fn uri_list(path: &Path) -> Option<String> {
    let uri = gio::File::for_path(path).uri();
    Some(format!("{uri}\n"))
}
