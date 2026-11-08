//! Read-only TLP 1.9+ state; no polling shell processes and no privileged writes.
use crate::*;
use std::{fs, path::Path};
pub const POLICY: &str = "/etc/uflinux/power.json";
pub const TLP_CONFIG: &str = "/etc/tlp.d/90-uflinux-profile.conf";
pub fn parse_profile(state: &str) -> Option<PowerProfile> {
    match state.split_whitespace().next()? {
        "0" => Some(PowerProfile::Performance),
        "1" => Some(PowerProfile::Balanced),
        "2" => Some(PowerProfile::PowerSaver),
        _ => None,
    }
}
pub fn on_battery() -> bool {
    fs::read_dir("/sys/class/power_supply")
        .into_iter()
        .flatten()
        .flatten()
        .any(|p| {
            fs::read_to_string(p.path().join("type")).is_ok_and(|s| s.trim() == "Battery")
                && fs::read_to_string(p.path().join("status"))
                    .is_ok_and(|s| s.trim() == "Discharging")
        })
}
pub fn status() -> Power {
    let last = fs::read_to_string("/run/tlp/last_pwr").unwrap_or_default();
    let handler = fs::read_to_string("/run/uflinux-acpid.pid")
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .is_some_and(|pid| Path::new(&format!("/proc/{pid}")).exists());
    Power {
        available: Path::new("/usr/sbin/tlp").is_file(),
        profile: parse_profile(&last),
        policy: files::read_json(Path::new(POLICY)),
        on_battery: on_battery(),
        button_handler_running: handler,
        can_suspend: fs::read_to_string("/sys/power/state")
            .is_ok_and(|s| s.split_whitespace().any(|m| m == "mem")),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profiles_are_observed_not_guessed() {
        assert_eq!(parse_profile("0 0"), Some(PowerProfile::Performance));
        assert_eq!(parse_profile("1 128"), Some(PowerProfile::Balanced));
        assert_eq!(parse_profile("2 1"), Some(PowerProfile::PowerSaver));
        for state in ["", "255 0", "unknown", "PRF"] {
            assert_eq!(parse_profile(state), None);
        }
    }
    #[test]
    fn typed_power_operations_reject_commands() {
        assert!(
            serde_json::from_str::<SystemAction>(
                r#"{"operation":"set-power-profile","profile":"balanced;reboot"}"#
            )
            .is_err()
        );
        assert!(serde_json::from_str::<SystemAction>(r#"{"operation":"set-power-buttons","on_ac":"poweroff","on_battery":"nothing","command":"id"}"#).is_err());
    }
}
