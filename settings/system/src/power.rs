//! The helper accepts only the three TLP profiles and three physical-button actions.
use glib::variant::ToVariant;
use settings_core::{
    bus,
    power::{POLICY, TLP_CONFIG},
    *,
};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    path::Path,
    sync::Mutex,
};
static LOCK: Mutex<()> = Mutex::new(());
fn restore(config: &Option<Vec<u8>>) -> Result<()> {
    if let Some(data) = config {
        files::atomic(Path::new(TLP_CONFIG), data, 0o644)
    } else {
        fs::remove_file(TLP_CONFIG).map_err(|e| e.to_string())
    }
}
pub fn set_profile(profile: PowerProfile) -> Result<()> {
    let _lock = LOCK.lock().map_err(|_| "Power management is busy")?;
    if !power::status().available {
        return Err("TLP is not installed".into());
    }
    let old = power::status();
    let original = fs::read(TLP_CONFIG).ok();
    let config = format!(
        "# Managed by UFLinux Settings\nTLP_AUTO_SWITCH=0\nTLP_PROFILE_DEFAULT={}\n",
        profile.code()
    );
    files::atomic(Path::new(TLP_CONFIG), config.as_bytes(), 0o644)?;
    let apply = || -> Result<()> {
        crate::operations::command("/usr/sbin/tlp", &[profile.name()], 30)?;
        if power::status().profile != Some(profile) {
            return Err("TLP did not report the requested power profile".into());
        }
        let mut policy = old.policy.clone();
        policy.profile = profile;
        files::save_json(Path::new(POLICY), &policy, 0o644)
    };
    if let Err(error) = apply() {
        restore(&original)?;
        if let Some(profile) = old.profile {
            let _ = crate::operations::command("/usr/sbin/tlp", &[profile.name()], 30);
        }
        return Err(error);
    }
    Ok(())
}
pub fn set_buttons(on_ac: PowerButtonAction, on_battery: PowerButtonAction) -> Result<()> {
    let _lock = LOCK.lock().map_err(|_| "Power management is busy")?;
    if !power::status().can_suspend && [on_ac, on_battery].contains(&PowerButtonAction::Suspend) {
        return Err("Suspend is unavailable on this system; choose Shut down or Do nothing".into());
    }
    let mut policy = power::status().policy;
    policy.on_ac = on_ac;
    policy.on_battery = on_battery;
    files::save_json(Path::new(POLICY), &policy, 0o644)
}
fn action(policy: &PowerPolicy, battery: bool) -> PowerButtonAction {
    if battery {
        policy.on_battery
    } else {
        policy.on_ac
    }
}
pub async fn button_event() -> Result<()> {
    let mut lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open("/run/uflinux-power-button.lock")
        .map_err(|e| e.to_string())?;
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Ok(());
    }
    let mut now: libc::timespec = unsafe { std::mem::zeroed() };
    unsafe {
        libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut now);
    }
    let mut previous = String::new();
    let _ = lock.read_to_string(&mut previous);
    if previous
        .trim()
        .parse::<u64>()
        .is_ok_and(|last| (now.tv_sec as u64).saturating_sub(last) < 2)
    {
        return Ok(());
    }
    lock.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    lock.set_len(0).map_err(|e| e.to_string())?;
    write!(lock, "{}", now.tv_sec).map_err(|e| e.to_string())?;
    let policy = power::status().policy;
    let method = match action(&policy, power::on_battery()) {
        PowerButtonAction::Nothing => return Ok(()),
        PowerButtonAction::Poweroff => "PowerOff",
        PowerButtonAction::Suspend => {
            if !power::status().can_suspend {
                return Err("Suspend is unavailable on this system".into());
            }
            "Suspend"
        }
    };
    let connection = gio::bus_get_future(gio::BusType::System)
        .await
        .map_err(|e| e.to_string())?;
    let reply = bus::call(
        &connection,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
        "ListInhibitors",
        None,
    )
    .await?;
    let inhibitors = reply
        .get::<(Vec<(String, String, String, String, u32, u32)>,)>()
        .ok_or("Cannot inspect system power inhibitors")?
        .0;
    let kind = if method == "Suspend" {
        "sleep"
    } else {
        "shutdown"
    };
    if inhibitors.iter().any(|(what, _, _, mode, _, _)| {
        ["block", "block-weak"].contains(&mode.as_str())
            && what
                .split(':')
                .any(|v| v == kind || v == "handle-power-key")
    }) {
        return Err("A running application is blocking this power action".into());
    }
    bus::call(
        &connection,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
        method,
        Some(&(false,).to_variant()),
    )
    .await?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn power_source_selects_only_its_configured_action() {
        let policy = PowerPolicy {
            profile: PowerProfile::Balanced,
            on_ac: PowerButtonAction::Nothing,
            on_battery: PowerButtonAction::Suspend,
        };
        assert_eq!(action(&policy, false), PowerButtonAction::Nothing);
        assert_eq!(action(&policy, true), PowerButtonAction::Suspend);
        assert_eq!(
            action(&PowerPolicy::default(), true),
            PowerButtonAction::Poweroff
        );
    }
}
