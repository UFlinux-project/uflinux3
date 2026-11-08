//! Recover saved layouts against the modes and identities actually connected now.
use crate::*;

fn usable_mode(output: &Output) -> Option<Mode> {
    output
        .modes
        .iter()
        .find(|m| m.preferred)
        .or_else(|| output.modes.first())
        .cloned()
}

pub fn fallback(current: &[Output]) -> Result<Vec<OutputConfig>> {
    let selected = current
        .iter()
        .position(|o| o.enabled && usable_mode(o).is_some())
        .or_else(|| current.iter().position(|o| usable_mode(o).is_some()))
        .ok_or("No connected display has a usable mode")?;
    let configs = current
        .iter()
        .enumerate()
        .map(|(index, output)| OutputConfig {
            name: output.name.clone(),
            enabled: index == selected,
            mode: usable_mode(output),
            x: 0,
            y: 0,
            scale: 1.,
            transform: 0,
            adaptive_sync: false,
        })
        .collect::<Vec<_>>();
    validate::displays(&configs, current)?;
    Ok(configs)
}

pub fn restore(saved: &[Output], current: &[Output]) -> Result<Vec<OutputConfig>> {
    let mut configs = current
        .iter()
        .map(|output| {
            // Duplicate/missing serial numbers require connector identity as well.
            let unique = !output.serial.is_empty()
                && current
                    .iter()
                    .filter(|o| o.identity() == output.identity())
                    .count()
                    == 1
                && saved
                    .iter()
                    .filter(|o| o.identity() == output.identity())
                    .count()
                    == 1;
            let previous = saved.iter().find(|o| {
                if unique {
                    o.identity() == output.identity()
                } else {
                    o.name == output.name && o.make == output.make && o.model == output.model
                }
            });
            let mut config = previous.unwrap_or(output).config();
            config.name = output.name.clone();
            if !config.mode.as_ref().is_some_and(|m| {
                output
                    .modes
                    .iter()
                    .any(|v| (m.width, m.height, m.refresh) == (v.width, v.height, v.refresh))
            }) {
                config.mode = usable_mode(output);
            }
            config.adaptive_sync &= output.adaptive_sync_control;
            config.enabled &= config.mode.is_some();
            config
        })
        .collect::<Vec<_>>();
    if !configs.iter().any(|c| c.enabled) {
        return fallback(current);
    }
    if validate::displays(&configs, current).is_err() {
        configs = fallback(current)?;
    }
    Ok(configs)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn output(name: &str, serial: &str, enabled: bool) -> Output {
        let mode = Mode {
            width: 1920,
            height: 1080,
            refresh: 59940,
            preferred: true,
        };
        Output {
            name: name.into(),
            make: "UF".into(),
            model: "Panel".into(),
            serial: serial.into(),
            enabled,
            scale: 1.,
            current: Some(mode.clone()),
            modes: vec![mode],
            ..Output::default()
        }
    }
    #[test]
    fn unplugging_the_only_enabled_saved_display_enables_the_remaining_one() {
        let saved = vec![output("DP-1", "a", true), output("HDMI-A-1", "b", false)];
        let current = vec![saved[1].clone()];
        let restored = restore(&saved, &current).unwrap();
        assert!(restored[0].enabled);
        assert_eq!(
            (restored[0].x, restored[0].y, restored[0].scale),
            (0, 0, 1.)
        );
    }
    #[test]
    fn identities_survive_connector_changes_without_reusing_a_duplicate_serial() {
        let mut saved = vec![output("DP-1", "unique", true)];
        saved[0].scale = 1.5;
        let current = vec![output("DP-2", "unique", true)];
        assert_eq!(restore(&saved, &current).unwrap()[0].scale, 1.5);
        let current = vec![
            output("DP-2", "unique", true),
            output("DP-3", "unique", true),
        ];
        assert_eq!(restore(&saved, &current).unwrap()[0].scale, 1.);
    }
    #[test]
    fn missing_modes_vrr_and_invalid_saved_coordinates_recover() {
        let current = vec![output("DP-1", "a", true)];
        let mut saved = current.clone();
        saved[0].adaptive_sync = true;
        saved[0].current.as_mut().unwrap().refresh = 60000;
        let restored = restore(&saved, &current).unwrap();
        assert_eq!(restored[0].mode.as_ref().unwrap().refresh, 59940);
        assert!(!restored[0].adaptive_sync);
        saved[0].x = i32::MAX;
        assert_eq!(restore(&saved, &current).unwrap()[0].x, 0);
        assert!(fallback(&[]).is_err());
    }
}
