//! Probe the capture backend and validate screen capture preferences.
//!
//! `gpu-screen-recorder --info` needs a display and a GPU and can take a moment,
//! so it runs on a blocking thread once at startup and its result is cached in the
//! snapshot. Everything here degrades to "backend unavailable" rather than failing.

use settings_core::*;
use std::process::{Command, Stdio};

const BACKEND: &str = "gpu-screen-recorder";
const MAX_DIR_LEN: usize = 4096;

pub fn validate(prefs: &ScreenCapture) -> Result<()> {
    if prefs.delay_seconds > MAX_DELAY_SECONDS {
        return Err("Screenshot delay must be 30 seconds or less".into());
    }
    if prefs.fps == 0 || prefs.fps > MAX_FPS {
        return Err("Frame rate must be between 1 and 240".into());
    }
    if prefs.bitrate_kbps == 0 || prefs.bitrate_kbps > MAX_BITRATE_KBPS {
        return Err("Bitrate must be between 1 and 500000 kbps".into());
    }
    if prefs.screenshot_dir.len() > MAX_DIR_LEN
        || prefs.screenshot_dir.contains('\0')
        || !(prefs.screenshot_dir.is_empty() || prefs.screenshot_dir.starts_with('/'))
    {
        return Err("Screenshot folder must be an absolute path".into());
    }
    if prefs
        .video_codec
        .as_deref()
        .is_some_and(|c| !VIDEO_CODECS.contains(&c))
    {
        return Err("Unsupported video codec".into());
    }
    if prefs
        .container
        .as_deref()
        .is_some_and(|c| !CONTAINERS.contains(&c))
    {
        return Err("Unsupported recording container".into());
    }
    Ok(())
}

/// Runs the backend once off the main thread. Never returns an error: an absent or
/// failing backend simply reports no capabilities.
pub async fn probe() -> CaptureCapabilities {
    gio::spawn_blocking(read).await.unwrap_or_default()
}

fn read() -> CaptureCapabilities {
    let output = Command::new(BACKEND)
        .arg("--info")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success());
    let Some(output) = output else {
        return CaptureCapabilities::default();
    };
    let mut caps = parse(&String::from_utf8_lossy(&output.stdout));
    caps.backend_available = true;
    caps
}

/// Parses the `section=<name>` / one-item-per-line format emitted by `--info`.
/// Only the three sections the settings page needs are collected, and the
/// `video_yuv444` repetition of the codec list is deliberately skipped.
pub fn parse(info: &str) -> CaptureCapabilities {
    let mut caps = CaptureCapabilities::default();
    let mut section = "";
    for line in info.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = line.strip_prefix("section=") {
            section = name;
            continue;
        }
        if !line
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
        {
            continue;
        }
        match section {
            "image_formats" => push(&mut caps.image_formats, line),
            "video_codecs" => push(&mut caps.video_codecs, line),
            "containers" => push(&mut caps.containers, line),
            _ => {}
        }
    }
    caps
}

fn push(list: &mut Vec<String>, value: &str) {
    if value.len() <= 32 && !list.iter().any(|seen| seen == value) {
        list.push(value.to_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prefs() -> ScreenCapture {
        ScreenCapture::default()
    }

    #[test]
    fn defaults_are_valid() {
        assert!(validate(&prefs()).is_ok());
    }

    #[test]
    fn rejects_bad_ranges() {
        let mut p = prefs();
        p.delay_seconds = 31;
        assert!(validate(&p).is_err());
        let mut p = prefs();
        p.fps = 0;
        assert!(validate(&p).is_err());
        let mut p = prefs();
        p.bitrate_kbps = MAX_BITRATE_KBPS + 1;
        assert!(validate(&p).is_err());
    }

    #[test]
    fn rejects_relative_folder() {
        let mut p = prefs();
        p.screenshot_dir = "Screenshots".into();
        assert!(validate(&p).is_err());
        p.screenshot_dir = String::new();
        assert!(validate(&p).is_ok());
    }

    #[test]
    fn rejects_unknown_codec_or_container() {
        let mut p = prefs();
        p.video_codec = Some("rv40".into());
        assert!(validate(&p).is_err());
        p.video_codec = Some("av1_vulkan".into());
        assert!(validate(&p).is_ok());
        let mut p = prefs();
        p.container = Some("avi".into());
        assert!(validate(&p).is_err());
        p.container = Some("webm".into());
        assert!(validate(&p).is_ok());
    }

    #[test]
    fn parses_info_sections() {
        let info = "section=system_info\nos|alpine\nsection=gpu_info\nvendor|amd\n\
                    section=video_codecs\nh264\nhevc\nav1\nh264_v4l2\n\
                    section=video_yuv444\nh264\nsection=image_formats\njpeg\npng\nwebp\n\
                    section=containers\nmp4\nmkv\nwebm\nsection=capture_options\nscreen\n";
        let caps = parse(info);
        assert_eq!(caps.image_formats, ["jpeg", "png", "webp"]);
        assert_eq!(caps.video_codecs, ["h264", "hevc", "av1", "h264_v4l2"]);
        assert_eq!(caps.containers, ["mp4", "mkv", "webm"]);
        assert!(!caps.backend_available);
    }

    #[test]
    fn ignores_garbage() {
        assert_eq!(parse("").image_formats.len(), 0);
        assert_eq!(parse("section=nope\nhello world\n").image_formats.len(), 0);
    }
}
