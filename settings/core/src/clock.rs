//! ConnMan SNTP is the only clock writer. Read its policy over the system bus and
//! take synchronization truth from the kernel, never from a daemon claiming success.
use crate::{DateTime, Result, bus};
use glib::variant::{ObjectPath, ToVariant};
use std::{fs, path::Path};
pub const CLOCK_INTERFACE: &str = "net.connman.Clock";
const MAIN_CONF: &str = "/etc/connman/main.conf";
const SETTINGS: &str = "/var/lib/connman/settings";
/// Values of a `key=value` entry inside one section of a ConnMan key file.
/// Comma-separated lists are expanded, matching ConnMan's own parser.
fn key_file_values(path: &Path, section: &str, key: &str) -> Vec<String> {
    let text = fs::read_to_string(path).unwrap_or_default();
    let mut current = "";
    let mut found = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            current = name;
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        if current != section || name.trim() != key {
            continue;
        }
        found.extend(
            value
                .split(',')
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_string),
        );
    }
    found
}
/// ConnMan's persisted policy, used when ConnMan itself cannot be reached.
fn persisted_auto(key: &str) -> bool {
    key_file_values(Path::new(SETTINGS), "global", key)
        .last()
        .is_some_and(|v| v == "auto")
}
/// The list ConnMan would actually query, in ConnMan's own precedence order:
/// service-provided (DHCP) servers, then the global list, then the fallback list.
fn timeservers(
    bus: Option<&gio::DBusConnection>,
    clock: Option<&bus::Properties>,
    global: &[String],
) -> Vec<String> {
    let mut servers: Vec<String> = Vec::new();
    let mut push = |server: &str| {
        if crate::validate::server(server) && !servers.iter().any(|s| s == server) {
            servers.push(server.to_string());
        }
    };
    if let Some(bus) = bus {
        if let Ok(reply) = bus.call_sync(
            Some("net.connman"),
            "/",
            "net.connman.Manager",
            "GetServices",
            None,
            None,
            gio::DBusCallFlags::NONE,
            2000,
            gio::Cancellable::NONE,
        ) {
            for (_, properties) in reply
                .get::<(Vec<(ObjectPath, bus::Properties)>,)>()
                .unwrap_or_default()
                .0
            {
                if !["ready", "online"].contains(&bus::text(&properties, "State").as_str()) {
                    continue;
                }
                for server in bus::strings(&properties, "Timeservers") {
                    push(&server);
                }
            }
        }
    }
    let advertised = clock
        .map(|clock| bus::strings(clock, "Timeservers"))
        .unwrap_or_default();
    for server in global.iter().chain(advertised.iter()) {
        push(server);
    }
    for server in key_file_values(Path::new(MAIN_CONF), "General", "FallbackTimeservers") {
        push(&server);
    }
    servers
}
pub fn status() -> DateTime {
    let timezone = fs::read_to_string("/etc/timezone")
        .unwrap_or_else(|_| "UTC".into())
        .trim()
        .to_string();
    let bus = gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE).ok();
    let clock = bus.as_ref().and_then(|bus| {
        bus.call_sync(
            Some("net.connman"),
            "/",
            CLOCK_INTERFACE,
            "GetProperties",
            None,
            None,
            gio::DBusCallFlags::NONE,
            2000,
            gio::Cancellable::NONE,
        )
        .ok()
        // GetProperties returns the dictionary wrapped in a one-element tuple.
        .and_then(|reply| reply.get::<(bus::Properties,)>().map(|v| v.0))
    });
    let enabled = clock
        .as_ref()
        .map(|clock| bus::text(clock, "TimeUpdates") == "auto")
        .unwrap_or_else(|| persisted_auto("TimeUpdates"));
    let global = key_file_values(Path::new(SETTINGS), "global", "Timeservers");
    let mut time: libc::timex = unsafe { std::mem::zeroed() };
    let result = unsafe { libc::adjtimex(&mut time) };
    DateTime {
        timezone,
        time_sync_enabled: enabled,
        // ConnMan only queries a timeserver while its Clock interface answers and
        // its policy is automatic, so this is the synchronizer's real state.
        sntp_running: clock.is_some() && enabled,
        synchronized: if result >= 0 && enabled {
            // STA_UNSYNC: the kernel clears it once a time source disciplines the clock.
            Some(time.status & 0x40 == 0)
        } else {
            None
        },
        timeservers: timeservers(bus.as_ref(), clock.as_ref(), &global),
        utc_seconds: unsafe { libc::time(std::ptr::null_mut()) } as i64,
    }
}
/// Flip ConnMan's own policy. ConnMan persists it and re-synchronizes immediately.
pub fn set_time_sync(enabled: bool) -> Result<()> {
    let bus = gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE)
        .map_err(|e| e.message().to_string())?;
    let value = if enabled { "auto" } else { "manual" };
    bus.call_sync(
        Some("net.connman"),
        "/",
        CLOCK_INTERFACE,
        "SetProperty",
        Some(&("TimeUpdates", value.to_variant()).to_variant()),
        None,
        gio::DBusCallFlags::NONE,
        10_000,
        gio::Cancellable::NONE,
    )
    .map_err(|e| e.message().to_string())?;
    Ok(())
}
