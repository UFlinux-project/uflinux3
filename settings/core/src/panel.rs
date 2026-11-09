//! Human-readable status and themed symbolic icon names for the panel.
use crate::{Snapshot, power};

pub fn status(s: &Snapshot) -> [(String, String); 3] {
    let network = &s.network;
    let active = network
        .services
        .iter()
        .filter(|s| ["online", "ready"].contains(&s.state.as_str()))
        .min_by_key(|s| if s.kind == "ethernet" { 0 } else { 1 });
    let connecting = network
        .services
        .iter()
        .find(|s| ["association", "configuration"].contains(&s.state.as_str()));
    let (net_icon, net_text) = if !network.available {
        (
            "network-error-symbolic".to_string(),
            "Network unavailable".to_string(),
        )
    } else if network.offline {
        (
            "network-offline-symbolic".to_string(),
            "Networking disabled".to_string(),
        )
    } else if let Some(service) = active {
        let icon = if service.kind == "ethernet" {
            "network-wired-symbolic".into()
        } else if service.kind == "wifi" {
            let level = match service.strength {
                0 => "none",
                1..=25 => "weak",
                26..=50 => "ok",
                51..=75 => "good",
                _ => "excellent",
            };
            format!("network-wireless-signal-{level}-symbolic")
        } else {
            "network-transmit-receive-symbolic".into()
        };
        (
            icon,
            format!(
                "{}{}",
                if service.name.is_empty() {
                    &service.kind
                } else {
                    &service.name
                },
                if service.state == "ready" {
                    " · limited connectivity"
                } else {
                    ""
                }
            ),
        )
    } else if let Some(service) = connecting {
        (
            if service.kind == "ethernet" {
                "network-wired-acquiring-symbolic"
            } else {
                "network-wireless-acquiring-symbolic"
            }
            .into(),
            format!("Connecting to {}", service.name),
        )
    } else if network.wifi_hardware_blocked
        || network.wifi_software_blocked
        || network.technologies.iter().any(|t| t.kind == "wifi")
            && !network
                .technologies
                .iter()
                .any(|t| t.kind == "wifi" && t.powered)
    {
        (
            "network-wireless-disabled-symbolic".into(),
            "Wi-Fi disabled".into(),
        )
    } else {
        ("network-offline-symbolic".into(), "Disconnected".into())
    };
    let device = s
        .audio
        .outputs
        .iter()
        .find(|d| d.name == s.audio.default_output);
    let (sound_icon, sound_text) = match device {
        None => ("audio-volume-muted-symbolic", "Audio unavailable".into()),
        Some(d) if d.muted => ("audio-volume-muted-symbolic", "Sound muted".into()),
        Some(d) => (
            if d.volume < 33. {
                "audio-volume-low-symbolic"
            } else if d.volume < 67. {
                "audio-volume-medium-symbolic"
            } else {
                "audio-volume-high-symbolic"
            },
            format!("Volume {:.0}%", d.volume),
        ),
    };
    let batteries = &s.power.batteries;
    let (power_icon, power_text) = if batteries.is_empty() {
        ("thunderbolt-symbolic".into(), "External power".into())
    } else {
        let charging = batteries.iter().any(|b| b.status == "Charging");
        match power::battery_percent(batteries) {
            Some(p) => {
                let level = (u16::from(p) + 5) / 10 * 10;
                let icon = if level == 100 && charging {
                    "battery-full-charging-symbolic".into()
                } else if level == 100 && batteries.iter().all(|b| b.status == "Full") {
                    "battery-level-100-charged-symbolic".into()
                } else {
                    format!(
                        "battery-level-{level}{}-symbolic",
                        if charging { "-charging" } else { "" }
                    )
                };
                (
                    icon,
                    format!(
                        "Battery {p}%{}",
                        if charging {
                            " · charging"
                        } else if s.power.on_battery {
                            " · discharging"
                        } else if batteries.iter().all(|b| b.status == "Full") {
                            " · fully charged"
                        } else {
                            " · not charging"
                        }
                    ),
                )
            }
            None => (
                "battery-symbolic".into(),
                "Battery level unavailable".into(),
            ),
        }
    };
    [
        (net_icon, net_text),
        (sound_icon.into(), sound_text),
        (power_icon, power_text),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AudioDevice, Battery, Service};
    #[test]
    fn panel_tracks_connections_audio_and_battery() {
        let mut s = Snapshot::default();
        assert_eq!(status(&s)[0].1, "Network unavailable");
        assert_eq!(status(&s)[2].0, "thunderbolt-symbolic");
        s.network.available = true;
        s.network.services.push(Service {
            kind: "wifi".into(),
            name: "Home".into(),
            state: "online".into(),
            strength: 55,
            ..Default::default()
        });
        assert_eq!(status(&s)[0].0, "network-wireless-signal-good-symbolic");
        s.network.services.push(Service {
            kind: "ethernet".into(),
            state: "ready".into(),
            ..Default::default()
        });
        assert_eq!(status(&s)[0].0, "network-wired-symbolic");
        assert!(status(&s)[0].1.contains("limited"));
        s.audio.default_output = "speakers".into();
        s.audio.outputs.push(AudioDevice {
            name: "speakers".into(),
            muted: true,
            volume: 80.,
            ..Default::default()
        });
        assert_eq!(status(&s)[1].0, "audio-volume-muted-symbolic");
        s.audio.outputs[0].muted = false;
        assert_eq!(status(&s)[1].0, "audio-volume-high-symbolic");
        s.power.batteries.push(Battery {
            name: "BAT0".into(),
            percent: Some(54),
            status: "Charging".into(),
            energy_full: Some(100),
        });
        assert_eq!(status(&s)[2].0, "battery-level-50-charging-symbolic");
        s.power.batteries[0].percent = None;
        assert_eq!(status(&s)[2].1, "Battery level unavailable");
    }
}
