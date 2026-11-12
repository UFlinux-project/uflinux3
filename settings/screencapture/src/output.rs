//! Output paths. Screenshots and recordings share one folder and one name shape.
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Longest path accepted by `sockaddr_un` on Linux (108 bytes including the NUL).
const SOCKET_PATH_LIMIT: usize = 107;

/// Empty preference means "$XDG_PICTURES_DIR/Screenshots", falling back to
/// "$HOME/Pictures/Screenshots" when the user dir is not configured.
pub fn screenshot_dir(configured: &str) -> PathBuf {
    if !configured.is_empty() {
        return PathBuf::from(configured);
    }
    pictures_dir().join("Screenshots").to_path_buf()
}

fn pictures_dir() -> PathBuf {
    glib::user_special_dir(glib::UserDirectory::Pictures)
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or_else(|| glib::home_dir().join("Pictures"))
}

/// GNOME-style timestamped name. The time uses dashes rather than colons
/// because colons in filenames break scp, rsync and several file managers.
pub fn filename(extension: &str) -> String {
    let stamp = glib::DateTime::now_local()
        .and_then(|time| time.format("%Y-%m-%d %H-%M-%S"))
        .ok();
    match stamp {
        Some(stamp) => format!("uflinux-screencapture-{stamp}.{extension}"),
        None => {
            let seconds = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|elapsed| elapsed.as_secs())
                .unwrap_or(0);
            format!("uflinux-screencapture-{seconds}.{extension}")
        }
    }
}

/// First free path in `directory` for `name`, adding " (1)", " (2)", … on
/// collision. The backend writes `-o` directly without checking for an existing
/// file, so the tool has to hand it a name that is not in use.
pub fn unique_path(directory: &Path, name: &str) -> PathBuf {
    let candidate = directory.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem.to_string(), format!(".{extension}")),
        _ => (name.to_string(), String::new()),
    };
    for index in 1..1000 {
        let candidate = directory.join(format!("{stem} ({index}){extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    // Still taken after 999 collisions: fall back to a name that cannot clash.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    directory.join(format!("{stem} ({nanos}){extension}"))
}

/// Creates the folder (and missing parents) with 0700, like the rest of the
/// project's user directories.
pub fn ensure_dir(path: &Path) -> settings_core::Result<()> {
    std::fs::create_dir_all(path).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Only tighten directories this tool just created; a user-chosen folder
        // keeps whatever permissions the user gave it.
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

/// Per-run IPC socket. The backend unlinks a stale unused socket itself, so the
/// tool must not pre-create the file.
pub fn socket_path() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let candidate = base.join(format!("uflinux-screencapture-{}.sock", std::process::id()));
    if candidate.as_os_str().len() <= SOCKET_PATH_LIMIT {
        return candidate;
    }
    // A very long XDG_RUNTIME_DIR would silently truncate into a wrong path, so
    // fall back to /tmp rather than to a socket the backend cannot bind.
    PathBuf::from(format!(
        "/tmp/uflinux-screencapture-{}.sock",
        std::process::id()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_is_timestamped() {
        let name = filename("webp");
        assert!(name.starts_with("uflinux-screencapture-"));
        assert!(name.ends_with(".webp"));
        assert_eq!(
            name.len(),
            "uflinux-screencapture-2025-01-01 00-00-00.webp".len()
        );
    }

    #[test]
    fn configured_dir_wins() {
        assert_eq!(screenshot_dir("/tmp/shots"), PathBuf::from("/tmp/shots"));
    }

    #[test]
    fn default_dir_is_pictures_screenshots() {
        let dir = screenshot_dir("");
        assert!(dir.ends_with("Screenshots"));
        assert!(dir.is_absolute());
    }

    #[test]
    fn collision_gets_suffix() {
        let root = std::env::temp_dir().join(format!("uflinux-sc-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let first = unique_path(&root, "shot.webp");
        assert_eq!(first, root.join("shot.webp"));
        std::fs::write(&first, b"x").unwrap();
        assert_eq!(unique_path(&root, "shot.webp"), root.join("shot (1).webp"));
        std::fs::write(root.join("shot (1).webp"), b"x").unwrap();
        assert_eq!(unique_path(&root, "shot.webp"), root.join("shot (2).webp"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn socket_path_fits_unix_socket() {
        assert!(socket_path().as_os_str().len() <= SOCKET_PATH_LIMIT);
    }
}
