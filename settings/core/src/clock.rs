//! Read-only clock inspection does not require a privileged service.
use crate::DateTime;
use std::{fs, path::Path};
pub fn status() -> DateTime {
    let timezone = fs::read_to_string("/etc/timezone")
        .unwrap_or_else(|_| "UTC".into())
        .trim()
        .to_string();
    let config = fs::read_to_string("/etc/conf.d/ntpd").unwrap_or_default();
    let tokens = config
        .split_whitespace()
        .map(|s| s.trim_matches(['\"', '\'']))
        .collect::<Vec<_>>();
    let servers = tokens
        .windows(2)
        .filter(|w| w[0] == "-p")
        .map(|w| w[1].to_string())
        .filter(|s| crate::validate::server(s))
        .collect::<Vec<_>>();
    let mut time: libc::timex = unsafe { std::mem::zeroed() };
    let result = unsafe { libc::adjtimex(&mut time) };
    let running = Path::new("/run/openrc/started/ntpd").exists();
    DateTime {
        timezone,
        ntp_enabled: Path::new("/etc/runlevels/default/ntpd").is_symlink(),
        ntp_running: running,
        synchronized: if result >= 0 && running {
            Some(time.status & 0x40 == 0)
        } else {
            None
        },
        servers: if servers.is_empty() {
            vec!["pool.ntp.org".into()]
        } else {
            servers
        },
        utc_seconds: unsafe { libc::time(std::ptr::null_mut()) } as i64,
    }
}
