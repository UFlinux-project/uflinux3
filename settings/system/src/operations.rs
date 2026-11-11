use crate::routes;
use glib::variant::ToVariant;
use settings_core::{bus, *};
use sha2::{Digest, Sha256};
use std::{
    cell::Cell,
    collections::HashMap,
    fs,
    io::Read,
    os::unix::fs::PermissionsExt,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    rc::Rc,
    sync::Mutex,
    time::{Duration, Instant},
};

pub(crate) fn command(executable: &str, args: &[&str], timeout: u64) -> Result<String> {
    let mut process = Command::new(executable);
    process.args(args).env("LC_ALL", "C");
    // TLP's non-secret runtime status must remain readable in the user session.
    unsafe {
        process.pre_exec(|| {
            libc::umask(0o022);
            Ok(())
        });
    }
    let mut child = process
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| format!("Cannot start {executable}"))?;
    let began = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            let mut output = String::new();
            if let Some(stdout) = child.stdout.take() {
                stdout
                    .take(128 * 1024)
                    .read_to_string(&mut output)
                    .map_err(|e| e.to_string())?;
            }
            if !status.success() {
                return Err(format!("{executable} rejected the operation"));
            }
            return Ok(output);
        }
        if began.elapsed() > Duration::from_secs(timeout) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{executable} timed out"));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
async fn authorized(
    bus: &gio::DBusConnection,
    sender: &str,
    operation: &SystemAction,
) -> Result<()> {
    let action = match operation {
        SystemAction::Status | SystemAction::PowerStatus | SystemAction::GetRoutes { .. } => {
            return Ok(());
        }
        SystemAction::SetPowerProfile { .. } => "org.uflinux.settings.power-profile",
        SystemAction::SetPowerButtons { .. } => "org.uflinux.settings.power-admin",
        SystemAction::DdcDiscover | SystemAction::DdcBrightness { .. } => {
            "org.uflinux.settings.brightness"
        }
        SystemAction::ForgetCredentials { ssid } if !credentials().contains_key(ssid) => {
            return Ok(());
        }
        SystemAction::HiddenProfile { .. } | SystemAction::ForgetCredentials { .. } => {
            "org.uflinux.settings.connect"
        }
        SystemAction::SetRoutes { .. } | SystemAction::Enterprise { .. } => {
            "org.uflinux.settings.network-admin"
        }
        _ => "org.uflinux.settings.datetime",
    };
    let details: HashMap<String, String> = HashMap::new();
    let subject = HashMap::from([("name".to_string(), sender.to_variant())]);
    let parameters = (("system-bus-name", subject), action, details, 1u32, "").to_variant();
    let response = bus::call(
        bus,
        "org.freedesktop.PolicyKit1",
        "/org/freedesktop/PolicyKit1/Authority",
        "org.freedesktop.PolicyKit1.Authority",
        "CheckAuthorization",
        Some(&parameters),
    )
    .await?;
    let yes = response
        .get::<((bool, bool, HashMap<String, String>),)>()
        .map(|r| r.0.0)
        .unwrap_or(false);
    if yes {
        Ok(())
    } else {
        Err("Administrator authorization was denied or canceled".into())
    }
}
pub fn run() -> Result<()> {
    if unsafe { libc::geteuid() } != 0 {
        return Err("The system service must be D-Bus activated as root".into());
    }
    if std::env::args().any(|a| a == "--power-event") {
        return glib::MainContext::default().block_on(crate::power::button_event());
    }
    if std::env::args().any(|a| a == "--restore-routes") && !routes::configured() {
        return Ok(());
    }
    let context = glib::MainContext::default();
    let (_connection, mainloop) = context.block_on(async {
        let connection = gio::bus_get_future(gio::BusType::System)
            .await
            .map_err(|e| e.to_string())?;
        let mainloop = glib::MainLoop::new(None, false);
        let active = Rc::new(Cell::new(0u32));
        let last = Rc::new(Cell::new(Instant::now()));
        let info = gio::DBusNodeInfo::for_xml(bus::SYSTEM_XML)
            .map_err(|e| e.to_string())?
            .interfaces()[0]
            .clone();
        let live = active.clone();
        let recent = last.clone();
        connection
            .register_object(SYSTEM_PATH, &info)
            .method_call(move |conn, sender, _, _, method, parameters, invocation| {
                if method != "Execute" {
                    invocation.return_dbus_error(
                        "org.freedesktop.DBus.Error.UnknownMethod",
                        "Unknown operation",
                    );
                    return;
                }
                let Some((raw,)) = parameters.get::<(String,)>() else {
                    invocation
                        .return_dbus_error("org.uflinux.Settings.Error.Invalid", "Invalid request");
                    return;
                };
                if raw.len() > MAX_REQUEST || live.get() >= 8 {
                    invocation.return_dbus_error(
                        "org.uflinux.Settings.Error.Busy",
                        "Request too large or service busy",
                    );
                    return;
                }
                let Ok(operation) = serde_json::from_str::<SystemAction>(&raw) else {
                    invocation.return_dbus_error(
                        "org.uflinux.Settings.Error.Invalid",
                        "Unknown typed operation",
                    );
                    return;
                };
                let Some(sender) = sender.map(str::to_string) else {
                    return;
                };
                let live = live.clone();
                let recent = recent.clone();
                live.set(live.get() + 1);
                recent.set(Instant::now());
                let conn = conn.clone();
                glib::MainContext::default().spawn_local(async move {
                    let result = async {
                        authorized(&conn, &sender, &operation).await?;
                        execute(conn, &operation).await
                    }
                    .await;
                    match result {
                        Ok(value) => {
                            invocation.return_value(Some(&(value.to_string(),).to_variant()))
                        }
                        Err(error) => invocation
                            .return_dbus_error("org.uflinux.Settings.Error.Failed", &error),
                    }
                    live.set(live.get() - 1);
                    recent.set(Instant::now());
                });
            })
            .build()
            .map_err(|e| e.to_string())?;
        // D-Bus forwards the activating call as soon as the name is acquired.
        // Export the object first so that the first request cannot race startup.
        bus::request_name(&connection, SYSTEM_NAME)?;
        let restoring = Rc::new(Cell::new(false));
        let in_progress = restoring.clone();
        connection.signal_subscribe(
            Some("net.connman"),
            None,
            None,
            None,
            None,
            gio::DBusSignalFlags::NONE,
            move |bus, _, _, _, _, _| {
                if !routes::configured() || in_progress.replace(true) {
                    return;
                }
                let bus = bus.clone();
                let flag = in_progress.clone();
                glib::timeout_add_local_once(Duration::from_millis(300), move || {
                    glib::MainContext::default().spawn_local(async move {
                        let _ = routes::restore(bus).await;
                        flag.set(false);
                    });
                });
            },
        );
        let bus = connection.clone();
        glib::MainContext::default().spawn_local(async move {
            let _ = routes::restore(bus).await;
        });
        let quit = mainloop.clone();
        glib::timeout_add_local(Duration::from_secs(10), move || {
            if active.get() == 0
                && last.get().elapsed() > Duration::from_secs(45)
                && !routes::configured()
            {
                quit.quit();
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
        Ok::<_, String>((connection, mainloop))
    })?;
    mainloop.run();
    Ok(())
}
async fn execute(bus: gio::DBusConnection, operation: &SystemAction) -> Result<serde_json::Value> {
    match operation {
        SystemAction::SetRoutes {
            service,
            routes: items,
        } => {
            routes::set(&bus, service.clone(), items.clone()).await?;
            return Ok(serde_json::json!({"ok":true}));
        }
        SystemAction::GetRoutes { service } => {
            return serde_json::to_value(routes::list(service)?).map_err(|e| e.to_string());
        }
        _ => {}
    }
    let operation = operation.clone();
    gio::spawn_blocking(move||match operation {
        SystemAction::Status=>serde_json::to_value(status()).map_err(|e|e.to_string()),
        SystemAction::PowerStatus=>serde_json::to_value(settings_core::power::status()).map_err(|e|e.to_string()),
        SystemAction::SetPowerProfile{profile}=>{crate::power::set_profile(profile)?;serde_json::to_value(settings_core::power::status()).map_err(|e|e.to_string())}
        SystemAction::SetPowerButtons{on_ac,on_battery}=>{crate::power::set_buttons(on_ac,on_battery)?;serde_json::to_value(settings_core::power::status()).map_err(|e|e.to_string())}
        SystemAction::SetTimezone{timezone}=>{timezone_set(&timezone)?;Ok(serde_json::json!({"ok":true}))}
        SystemAction::SetTime{utc_seconds}=>{
            if status().time_sync_enabled{return Err("Disable automatic time before setting the clock".into());}
            if !(0..=253402300799).contains(&utc_seconds){return Err("Date must be between 1970 and 9999".into());}
            let time=libc::timespec{tv_sec:utc_seconds as _,tv_nsec:0};
            if unsafe{libc::clock_settime(libc::CLOCK_REALTIME,&time)}!=0{return Err("The kernel rejected the clock change".into());}
            if Path::new("/dev/rtc").exists()||Path::new("/dev/rtc0").exists(){let _=command("/sbin/hwclock",&["-w","-u"],3);}
            Ok(serde_json::json!({"ok":true}))
        }
        SystemAction::SetTimeSync{enabled}=>{
            settings_core::clock::set_time_sync(enabled)?;
            Ok(serde_json::json!({"ok":true}))
        }
        SystemAction::DdcDiscover=>serde_json::to_value(ddc_discover()?).map_err(|e|e.to_string()),
        SystemAction::DdcBrightness{id,percent}=>{ddc_set(&id,percent)?;Ok(serde_json::json!({"ok":true}))}
        SystemAction::Enterprise{profile}=>{enterprise(&profile)?;Ok(serde_json::json!({"ok":true}))}
        SystemAction::ForgetCredentials{ssid}=>{
            let mut known=credentials();if let Some(directory)=known.remove(&ssid){
                files::save_json(Path::new("/var/lib/uflinux-settings/credentials.json"),&known,0o600)?;
                remove_certificate(&directory)?;
            }Ok(serde_json::json!({"ok":true}))
        }
        SystemAction::HiddenProfile{ssid,passphrase}=>{
            if ssid.is_empty()||ssid.len()>32||ssid.contains(['\0','\r','\n'])||(!passphrase.is_empty()&&!(8..=63).contains(&passphrase.len()))||passphrase.contains(['\0','\r','\n']){return Err("Use a 1–32 byte SSID and an 8–63 character passphrase, or no passphrase for an open network".into());}
            let path=iwd_path(&ssid,if passphrase.is_empty(){"open"}else{"psk"});private_directory(Path::new("/var/lib/iwd"))?;
            let security=if passphrase.is_empty(){String::new()}else{format!("[Security]\nPassphrase={}\n",escape(&passphrase))};
            files::atomic(&path,format!("{security}[Settings]\nHidden=true\nAutoConnect=false\n").as_bytes(),0o600)?;
            Ok(serde_json::json!({"ok":true}))
        }
        _=>Err("Unknown operation".into()),
    }).await.map_err(|_|"System worker failed".to_string())?
}
fn status() -> DateTime {
    settings_core::clock::status()
}
fn timezone_set(timezone: &str) -> Result<()> {
    if timezone.is_empty()
        || timezone.len() > 128
        || timezone.split('/').any(|p| {
            p.is_empty()
                || p == "."
                || p == ".."
                || !p
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_+-".contains(&c))
        })
    {
        return Err("Choose an installed IANA timezone".into());
    }
    let directory = Path::new("/usr/share/zoneinfo")
        .canonicalize()
        .map_err(|_| "Timezone data is unavailable")?;
    let source = directory
        .join(timezone)
        .canonicalize()
        .map_err(|_| "Unknown timezone")?;
    if !source.starts_with(&directory)
        || !source.is_file()
        || !fs::read(&source)
            .map_err(|e| e.to_string())?
            .starts_with(b"TZif")
    {
        return Err("Invalid timezone data".into());
    }
    let temporary = PathBuf::from(format!("/etc/.uflinux-localtime-{}", std::process::id()));
    let _ = fs::remove_file(&temporary);
    std::os::unix::fs::symlink(&source, &temporary).map_err(|e| e.to_string())?;
    fs::rename(&temporary, "/etc/localtime").map_err(|e| e.to_string())?;
    files::atomic(
        Path::new("/etc/timezone"),
        format!("{timezone}\n").as_bytes(),
        0o644,
    )
}
fn private_directory(path: &Path) -> Result<()> {
    fs::create_dir_all(path).map_err(|e| e.to_string())?;
    if path.is_symlink() {
        return Err("Protected state directory must not be a symbolic link".into());
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())
}
fn iwd_path(ssid: &str, extension: &str) -> PathBuf {
    // Match iwd storage_get_network_file_path(), not just its filename decoder:
    // iwd reopens safe ASCII names using their plain canonical filename.
    if ssid
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"-_ ".contains(&b))
    {
        return Path::new("/var/lib/iwd").join(format!("{ssid}.{extension}"));
    }
    let hex = ssid
        .as_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    Path::new("/var/lib/iwd").join(format!("={hex}.{extension}"))
}
fn escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace(' ', "\\s")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}
fn enterprise(profile: &Enterprise) -> Result<()> {
    validate::enterprise(profile)?;
    private_directory(Path::new("/var/lib/iwd"))?;
    let mut digest = Sha256::new();
    for value in [
        &profile.ssid,
        &profile.ca_pem,
        &profile.client_certificate_pem,
        &profile.private_key_pem,
    ] {
        digest.update(value.as_bytes());
        digest.update([0]);
    }
    let identifier = format!("{:x}", digest.finalize());
    let dir = Path::new("/var/lib/uflinux-settings/certificates").join(&identifier);
    private_directory(&dir)?;
    let ca = dir.join("ca.pem");
    files::atomic(&ca, profile.ca_pem.as_bytes(), 0o600)?;
    command(
        "/usr/bin/openssl",
        &[
            "x509",
            "-in",
            ca.to_str().ok_or("Invalid certificate path")?,
            "-noout",
            "-checkend",
            "0",
        ],
        5,
    )?;
    let mut config = format!(
        "[Security]\nEAP-Method={}\nEAP-Identity={}\nEAP-{}-CACert={}\nEAP-{}-ServerDomainMask={}\n",
        profile.method,
        escape(if profile.anonymous_identity.is_empty() {
            &profile.identity
        } else {
            &profile.anonymous_identity
        }),
        profile.method,
        ca.display(),
        profile.method,
        profile.server_domain
    );
    if profile.method == "TLS" {
        let cert = dir.join("client.pem");
        let key = dir.join("key.pem");
        files::atomic(&cert, profile.client_certificate_pem.as_bytes(), 0o600)?;
        files::atomic(&key, profile.private_key_pem.as_bytes(), 0o600)?;
        command(
            "/usr/bin/openssl",
            &[
                "x509",
                "-in",
                cert.to_str().unwrap(),
                "-noout",
                "-checkend",
                "0",
            ],
            5,
        )?;
        config.push_str(&format!(
            "EAP-TLS-ClientCert={}\nEAP-TLS-ClientKey={}\n",
            cert.display(),
            key.display()
        ));
        // iwd rejects even an empty passphrase property for unencrypted keys.
        if !profile.key_password.is_empty() {
            config.push_str(&format!(
                "EAP-TLS-ClientKeyPassphrase={}\n",
                escape(&profile.key_password)
            ));
        }
    } else {
        let phase = if profile.method == "PEAP" {
            "MSCHAPV2"
        } else {
            "Tunneled-PAP"
        };
        config.push_str(&format!(
            "EAP-{}-Phase2-Method={}\nEAP-{}-Phase2-Identity={}\nEAP-{}-Phase2-Password={}\n",
            profile.method,
            phase,
            profile.method,
            escape(&profile.identity),
            profile.method,
            escape(&profile.password)
        ));
    }
    config.push_str("[Settings]\nAutoConnect=false\n");
    files::atomic(&iwd_path(&profile.ssid, "8021x"), config.as_bytes(), 0o600)?;
    let mut known = credentials();
    let previous = known.insert(profile.ssid.clone(), identifier.clone());
    files::save_json(
        Path::new("/var/lib/uflinux-settings/credentials.json"),
        &known,
        0o600,
    )?;
    if let Some(previous) = previous {
        if previous != identifier && !known.values().any(|v| v == &previous) {
            remove_certificate(&previous)?;
        }
    }
    Ok(())
}
fn credentials() -> std::collections::BTreeMap<String, String> {
    files::read_json(Path::new("/var/lib/uflinux-settings/credentials.json"))
}
fn remove_certificate(identifier: &str) -> Result<()> {
    if identifier.len() != 64 || !identifier.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Invalid protected certificate identifier".into());
    }
    let path = Path::new("/var/lib/uflinux-settings/certificates").join(identifier);
    if path.is_symlink() {
        fs::remove_file(path).map_err(|e| e.to_string())?;
    } else if path.is_dir() {
        fs::remove_dir_all(path).map_err(|e| e.to_string())?;
    }
    Ok(())
}
struct Monitor {
    id: String,
    name: String,
    bus: String,
}
fn monitors() -> Result<Vec<Monitor>> {
    let mut found = Vec::new();
    for entry in fs::read_dir("/sys/class/drm")
        .map_err(|_| "No DRM display connectors are available")?
        .flatten()
    {
        let path = entry.path();
        if fs::read_to_string(path.join("status"))
            .unwrap_or_default()
            .trim()
            != "connected"
        {
            continue;
        }
        let Ok(edid) = fs::read(path.join("edid")) else {
            continue;
        };
        if edid.is_empty() {
            continue;
        }
        let Ok(ddc) = path.join("ddc").canonicalize() else {
            continue;
        };
        let Some(bus) = ddc
            .file_name()
            .and_then(|s| s.to_str())
            .and_then(|s| s.strip_prefix("i2c-"))
            .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        else {
            continue;
        };
        let name = entry.file_name().to_string_lossy().to_string();
        let hash = format!("{:x}", Sha256::digest(edid));
        found.push(Monitor {
            id: format!("{name}:{}", &hash[..16]),
            name,
            bus: bus.to_string(),
        });
    }
    Ok(found)
}
fn vcp(m: &Monitor) -> Result<(u32, u32)> {
    let output = command(
        "/usr/bin/ddcutil",
        &["--bus", &m.bus, "--terse", "getvcp", "10"],
        4,
    )?;
    let values = output.split_whitespace().collect::<Vec<_>>();
    let at = values
        .iter()
        .position(|s| *s == "C")
        .ok_or("Monitor did not report brightness")?;
    let current = values
        .get(at + 1)
        .and_then(|s| s.parse::<u32>().ok())
        .ok_or("Invalid monitor brightness")?;
    let maximum = values
        .get(at + 2)
        .and_then(|s| s.parse::<u32>().ok())
        .filter(|n| *n > 0 && *n <= 65535)
        .ok_or("Invalid monitor brightness range")?;
    if current > maximum {
        return Err("Invalid monitor brightness".into());
    }
    Ok((current, maximum))
}
static DDC: Mutex<()> = Mutex::new(());
fn ddc_discover() -> Result<Vec<Brightness>> {
    let _guard = DDC.lock().map_err(|_| "DDC worker failed")?;
    let mut result = Vec::new();
    for monitor in monitors()?.into_iter().take(16) {
        if let Ok((value, max)) = vcp(&monitor) {
            result.push(Brightness {
                id: monitor.id,
                name: monitor.name,
                percent: value as f64 / max as f64 * 100.,
                ddc: true,
            });
        }
    }
    Ok(result)
}
fn ddc_set(id: &str, percent: f64) -> Result<()> {
    validate::percent(percent)?;
    let _guard = DDC.lock().map_err(|_| "DDC worker failed")?;
    let monitor = monitors()?
        .into_iter()
        .find(|m| m.id == id)
        .ok_or("Display disconnected or its identity changed")?;
    let (_, maximum) = vcp(&monitor)?;
    let value = ((percent * maximum as f64 / 100.).round() as u32)
        .clamp(1, maximum)
        .to_string();
    command(
        "/usr/bin/ddcutil",
        &["--bus", &monitor.bus, "setvcp", "10", &value],
        4,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn iwd_uses_its_canonical_plain_or_hex_filename() {
        assert_eq!(
            iwd_path("Test Wi-Fi", "psk"),
            Path::new("/var/lib/iwd/Test Wi-Fi.psk")
        );
        assert_eq!(
            iwd_path("a/b", "psk"),
            Path::new("/var/lib/iwd/=612f62.psk")
        );
    }
    #[test]
    fn protected_fields_escape_keyfile_metacharacters() {
        let text = " space \\ value\t";
        let key = glib::KeyFile::new();
        key.load_from_data(
            &format!("[Security]\nPassword={}\n", escape(text)),
            glib::KeyFileFlags::NONE,
        )
        .unwrap();
        assert_eq!(key.string("Security", "Password").unwrap().as_str(), text);
    }
    #[test]
    fn timezone_paths_cannot_escape() {
        for s in [
            "../../etc",
            "/etc/passwd",
            "Europe/../etc",
            "Europe/Prague\n",
        ] {
            assert!(timezone_set(s).is_err());
        }
    }
    #[test]
    fn certificate_cleanup_rejects_paths() {
        assert!(remove_certificate("../outside").is_err());
    }
}
