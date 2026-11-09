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
    batteries(Path::new("/sys/class/power_supply"))
        .iter()
        .any(|battery| battery.status == "Discharging")
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
        batteries: batteries(Path::new("/sys/class/power_supply")),
    }
}

pub fn batteries(root: &Path) -> Vec<Battery> {
    let mut batteries = fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let read = |name| {
                fs::read_to_string(path.join(name))
                    .ok()
                    .map(|s| s.trim().to_owned())
            };
            if read("type").as_deref() != Some("Battery")
                || read("scope").as_deref() == Some("Device")
                || read("present").as_deref() == Some("0")
            {
                return None;
            }
            let number = |name| read(name).and_then(|s| s.parse::<u64>().ok());
            let energy_full = number("energy_full").filter(|v| *v > 0);
            let percent = number("capacity")
                .filter(|v| *v <= 100)
                .or_else(|| {
                    let (now, full) = (number("energy_now")?, energy_full?);
                    Some(now.saturating_mul(100).checked_div(full)?.min(100))
                })
                .map(|v| v as u8);
            Some(Battery {
                name: entry.file_name().to_string_lossy().into_owned(),
                percent,
                status: read("status").unwrap_or_else(|| "Unknown".into()),
                energy_full,
            })
        })
        .collect::<Vec<_>>();
    batteries.sort_by(|a, b| a.name.cmp(&b.name));
    batteries
}

pub fn battery_percent(batteries: &[Battery]) -> Option<u8> {
    let known = batteries
        .iter()
        .filter_map(|b| b.percent.map(|p| (p, b.energy_full.filter(|v| *v > 0))))
        .collect::<Vec<_>>();
    if known.is_empty() {
        return None;
    }
    let value = if known.iter().all(|(_, full)| full.is_some()) {
        let total: u128 = known
            .iter()
            .map(|(_, full)| u128::from(full.unwrap()))
            .sum();
        let sum: u128 = known
            .iter()
            .map(|(p, full)| u128::from(*p) * u128::from(full.unwrap()))
            .sum();
        ((sum + total / 2) / total) as u8
    } else {
        ((known.iter().map(|(p, _)| u32::from(*p)).sum::<u32>() + known.len() as u32 / 2)
            / known.len() as u32) as u8
    };
    Some(value)
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

#[cfg(test)]
mod battery_tests {
    use super::*;
    #[test]
    fn system_batteries_and_weighted_charge() {
        let root = std::env::temp_dir().join(format!("uflinux-batteries-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for (name, scope, percent, full) in [
            ("BAT0", "System", "20", "100"),
            ("BAT1", "System", "80", "300"),
            ("mouse", "Device", "99", "100"),
        ] {
            let dir = root.join(name);
            fs::create_dir_all(&dir).unwrap();
            for (file, value) in [
                ("type", "Battery"),
                ("scope", scope),
                ("capacity", percent),
                ("energy_full", full),
                ("status", "Discharging"),
            ] {
                fs::write(dir.join(file), value).unwrap();
            }
        }
        let list = batteries(&root);
        assert_eq!(list.len(), 2);
        assert_eq!(battery_percent(&list), Some(65));
        fs::remove_file(root.join("BAT1/energy_full")).unwrap();
        assert_eq!(battery_percent(&batteries(&root)), Some(50));
        fs::remove_file(root.join("BAT0/capacity")).unwrap();
        fs::remove_file(root.join("BAT1/capacity")).unwrap();
        assert_eq!(battery_percent(&batteries(&root)), None);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn old_power_snapshots_are_accepted() {
        let mut value = serde_json::to_value(Power::default()).unwrap();
        value.as_object_mut().unwrap().remove("batteries");
        assert!(
            serde_json::from_value::<Power>(value)
                .unwrap()
                .batteries
                .is_empty()
        );
    }
}
