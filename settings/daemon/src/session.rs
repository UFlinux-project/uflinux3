use crate::{audio, devices, display, network};
use gio::prelude::*;
use glib::variant::ToVariant;
use settings_core::{bus, *};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::{net::UnixStream, process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    rc::Rc,
    time::Duration,
};

struct Pending {
    id: u64,
    previous: Vec<OutputConfig>,
    hdr: bool,
    caller: String,
    previous_xml: Option<Vec<u8>>,
}
struct Applying<'a>(&'a Cell<bool>);
impl Drop for Applying<'_> {
    fn drop(&mut self) {
        self.0.set(false);
    }
}
struct App {
    bus: gio::DBusConnection,
    system: gio::DBusConnection,
    view: RefCell<Snapshot>,
    network: RefCell<Option<Rc<network::Client>>>,
    audio: RefCell<Option<Rc<audio::Client>>>,
    audio_retry: Cell<bool>,
    display: RefCell<Option<display::Client>>,
    pending: RefCell<Option<Pending>>,
    applying: Cell<bool>,
    wallpaper: RefCell<Option<Child>>,
    publishing: Cell<bool>,
    restored: Cell<bool>,
    brightness_pending: Cell<bool>,
    ddc_pending: Cell<bool>,
    ddc_busy: Cell<bool>,
    ddc_dirty: Cell<bool>,
}
impl Drop for App {
    fn drop(&mut self) {
        if let Some(mut child) = self.wallpaper.get_mut().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

pub fn run() -> Result<()> {
    if std::env::var_os("XDG_RUNTIME_DIR").is_none() {
        return Err("A user login session is required".into());
    }
    let (_app,_monitor,_power_monitors,_device_monitor,mainloop)=glib::MainContext::default().block_on(async {
        let connection=gio::bus_get_future(gio::BusType::Session).await.map_err(|e|e.to_string())?;
        let system=gio::bus_get_future(gio::BusType::System).await.map_err(|e|e.to_string())?;
        let appearance=files::read_json(&files::config_dir().join("appearance.json"));
        let app=Rc::new(App{bus:connection.clone(),system,view:RefCell::new(Snapshot{appearance,datetime:clock::status(),power:power::status(),about:information(),..Snapshot::default()}),
            network:RefCell::new(None),audio:RefCell::new(None),audio_retry:Cell::new(false),display:RefCell::new(None),pending:RefCell::new(None),applying:Cell::new(false),wallpaper:RefCell::new(None),publishing:Cell::new(false),restored:Cell::new(false),brightness_pending:Cell::new(false),ddc_pending:Cell::new(false),ddc_busy:Cell::new(false),ddc_dirty:Cell::new(false)});
        app.view.borrow_mut().displays.renderer=std::env::var("UFLINUX_RENDERER").unwrap_or_else(|_|"User-selected / compositor default".into());
        app.view.borrow_mut().displays.hdr_requested=fs::read_to_string(hdr_path()).ok().and_then(|s|files::hdr_requested(&s)).unwrap_or(false);
        app.view.borrow_mut().displays.hdr_status="Labwc applies HDR automatically where supported. Per-monitor activation is not reported by this protocol; mirroring is unavailable.".into();
        app.view.borrow_mut().displays.brightness=backlights();
        let info=gio::DBusNodeInfo::for_xml(bus::SESSION_XML).map_err(|e|e.to_string())?.interfaces()[0].clone();
        let weak=Rc::downgrade(&app);
        connection.register_object(SESSION_PATH,&info).method_call(move|_,sender,_,_,method,parameters,invocation|{
            let Some(app)=weak.upgrade() else{return;};
            if method=="GetState" {
                invocation.return_value(Some(&(serde_json::to_string(&*app.view.borrow()).unwrap_or_default(),).to_variant()));return;
            }
            if method!="Execute"{invocation.return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod","Unknown operation");return;}
            let Some((request,))=parameters.get::<(String,)>() else{invocation.return_dbus_error("org.uflinux.Settings.Error.Invalid","Invalid request");return;};
            if request.len()>MAX_REQUEST{invocation.return_dbus_error("org.uflinux.Settings.Error.Invalid","Request too large");return;}
            let Ok(action)=serde_json::from_str::<Action>(&request) else{invocation.return_dbus_error("org.uflinux.Settings.Error.Invalid","Unknown typed operation");return;};
            let sender=sender.unwrap_or_default().to_string();
            glib::MainContext::default().spawn_local(async move {
                app.view.borrow_mut().error=None;app.publish();
                let result=app.execute(action,&sender).await;
                match result{
                    Ok(value)=>invocation.return_value(Some(&(value.to_string(),).to_variant())),
                    Err(error)=>{app.view.borrow_mut().error=Some(error.clone());app.publish();invocation.return_dbus_error("org.uflinux.Settings.Error.Failed",&error);}
                }
            });
        }).build().map_err(|e|e.to_string())?;
        bus::request_name(&connection,SESSION_NAME)?;
        let weak=Rc::downgrade(&app);
        connection.signal_subscribe(Some("org.freedesktop.DBus"),Some("org.freedesktop.DBus"),Some("NameOwnerChanged"),None,None,gio::DBusSignalFlags::NONE,move|_,_,_,_,_,params|{
            if let Some((name,_,new))=params.get::<(String,String,String)>() {
                if new.is_empty(){if let Some(app)=weak.upgrade(){let revert=app.pending.borrow().as_ref().is_some_and(|p|p.caller==name);if revert{let task=app.clone();glib::MainContext::default().spawn_local(async move{let _=task.revert().await;});}}}
            }
        });
        let weak=Rc::downgrade(&app);
        let network=network::Client::new(Rc::new(move|network|if let Some(app)=weak.upgrade(){app.view.borrow_mut().network=network;app.publish();})).await?;
        app.network.replace(Some(network));app.start_audio();
        let (send,receive)=async_channel::bounded(16);
        match display::Client::start(send){Ok(client)=>{app.display.replace(Some(client));},Err(error)=>app.view.borrow_mut().error=Some(error)}
        let weak=Rc::downgrade(&app);
        glib::MainContext::default().spawn_local(async move {
            while let Ok(event)=receive.recv().await {let Some(app)=weak.upgrade() else{break;};match event{
                display::Event::Outputs(outputs)=>{
                    let topology=|values:&[Output]|values.iter().map(|o|(o.name.clone(),o.identity(),o.modes.clone())).collect::<Vec<_>>();
                    let changed=topology(&app.view.borrow().displays.outputs)!=topology(&outputs);
                    {let mut view=app.view.borrow_mut();view.displays.outputs=outputs;view.displays.available=true;}
                    let initial=!app.restored.replace(true);
                    if changed||initial{app.queue_brightness(true);let task=app.clone();glib::MainContext::default().spawn_local(async move{if task.pending.borrow().is_some(){let _=task.revert().await;}let _=task.restore_displays().await;});}
                    app.publish();
                }
                display::Event::Unavailable(reason)=>{app.view.borrow_mut().displays.available=false;app.view.borrow_mut().displays.hdr_status=reason;app.publish();}
            }}
        });
        let runtime=gio::File::for_path(std::env::var_os("XDG_RUNTIME_DIR").unwrap());
        let _monitor=runtime.monitor_directory(gio::FileMonitorFlags::NONE,gio::Cancellable::NONE).ok();
        if let Some(monitor)=&_monitor{let weak=Rc::downgrade(&app);monitor.connect_changed(move|_,_,_,_|if let Some(app)=weak.upgrade(){app.publish();});}
        let mut power_monitors=Vec::new();
        for path in ["/run/tlp","/etc/uflinux"]{
            if let Ok(monitor)=gio::File::for_path(path).monitor_directory(gio::FileMonitorFlags::NONE,gio::Cancellable::NONE){
                let weak=Rc::downgrade(&app);monitor.connect_changed(move|_,_,_,_|if let Some(app)=weak.upgrade(){app.refresh_power();});power_monitors.push(monitor);
            }
        }
        let weak=Rc::downgrade(&app);
        let _device_monitor=devices::Monitor::new(move|event|if let Some(app)=weak.upgrade(){
            if event.backlight||event.drm{app.queue_brightness(event.drm);}
            if event.power{app.refresh_power();}
        }).map_err(|error| eprintln!("uflinux-settingsd: {error}")).ok();
        let weak=Rc::downgrade(&app);
        app.system.signal_subscribe(Some("org.freedesktop.login1"),Some("org.freedesktop.login1.Manager"),Some("PrepareForSleep"),None,None,gio::DBusSignalFlags::NONE,move|_,_,_,_,_,params|{
            if params.get::<(bool,)>()==Some((false,)){if let Some(app)=weak.upgrade(){app.queue_brightness(true);app.refresh_power();let task=app.clone();glib::MainContext::default().spawn_local(async move{let _=task.restore_displays().await;});}}
        });
        let initial=app.clone();glib::MainContext::default().spawn_local(async move{
            let appearance=initial.view.borrow().appearance.clone();let _=initial.set_appearance(appearance,false).await;
        });
        let weak=Rc::downgrade(&app);
        glib::timeout_add_local(Duration::from_secs(15),move||{
            let Some(app)=weak.upgrade() else{return glib::ControlFlow::Break;};
            let next=clock::status();
            let changed={let view=app.view.borrow();let old=&view.datetime;
                old.timezone!=next.timezone||old.ntp_enabled!=next.ntp_enabled||old.ntp_running!=next.ntp_running||old.synchronized!=next.synchronized||old.servers!=next.servers};
            app.view.borrow_mut().datetime=next;if changed{app.publish();}
            app.refresh_power();
            app.publish(); // Resynchronize after Ironbar has restarted.
            glib::ControlFlow::Continue
        });
        let mainloop=glib::MainLoop::new(None,false);
        for signal in [libc::SIGTERM,libc::SIGINT]{let quit=mainloop.clone();glib::source::unix_signal_add_local(signal,move||{quit.quit();glib::ControlFlow::Break});}
        Ok::<_,String>((app,_monitor,power_monitors,_device_monitor,mainloop))
    })?;
    mainloop.run();
    // GLib sources may retain App until process exit. Explicitly reap owned children.
    if let Some(mut child) = _app.wallpaper.borrow_mut().take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    Ok(())
}
impl App {
    fn queue_brightness(self: &Rc<Self>, ddc: bool) {
        if ddc {
            self.queue_ddc();
        }
        if self.brightness_pending.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(120), move || {
            let Some(app) = weak.upgrade() else {
                return;
            };
            app.brightness_pending.set(false);
            let mut next = backlights();
            next.extend(
                app.view
                    .borrow()
                    .displays
                    .brightness
                    .iter()
                    .filter(|b| b.ddc)
                    .cloned(),
            );
            if next != app.view.borrow().displays.brightness {
                app.view.borrow_mut().displays.brightness = next;
                app.publish();
            }
        });
    }
    fn queue_ddc(self: &Rc<Self>) {
        if self.ddc_busy.get() {
            self.ddc_dirty.set(true);
            return;
        }
        if self.ddc_pending.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(250), move || {
            let Some(app) = weak.upgrade() else {
                return;
            };
            app.ddc_pending.set(false);
            let connected = fs::read_dir("/sys/class/drm")
                .into_iter()
                .flatten()
                .flatten()
                .any(|entry| {
                    entry.path().join("ddc").exists()
                        && fs::read_to_string(entry.path().join("status"))
                            .is_ok_and(|s| s.trim() == "connected")
                });
            if !connected {
                let mut view = app.view.borrow_mut();
                view.displays.brightness.retain(|b| !b.ddc);
                view.displays.brightness_status.clear();
                drop(view);
                app.publish();
                return;
            }
            app.ddc_busy.set(true);
            glib::MainContext::default().spawn_local(async move {
                let result=app.system(&SystemAction::DdcDiscover).await.and_then(|value|
                    serde_json::from_value::<Vec<Brightness>>(value).map_err(|_|"Invalid brightness reply".into()));
                {
                    let mut view=app.view.borrow_mut();
                    view.displays.brightness.retain(|b|!b.ddc);
                    match result {
                        Ok(devices)=>{view.displays.brightness.extend(devices);view.displays.brightness_status.clear();},
                        Err(_)=>view.displays.brightness_status="External-display brightness is unavailable. An active local session and DDC/CI support are required.".into(),
                    }
                }
                app.ddc_busy.set(false); app.publish();
                if app.ddc_dirty.replace(false) { app.queue_ddc(); }
            });
        });
    }
    fn refresh_power(self: &Rc<Self>) {
        let state = power::status();
        if self.view.borrow().power != state {
            self.view.borrow_mut().power = state;
            self.publish();
        }
    }
    fn publish(self: &Rc<Self>) {
        if self.publishing.replace(true) {
            return;
        }
        let this = self.clone();
        glib::timeout_add_local_once(Duration::from_millis(40), move || {
            this.publishing.set(false);
            let view = this.view.borrow().clone();
            let raw = serde_json::to_string(&view).unwrap_or_default();
            let _ = this.bus.emit_signal(
                None,
                SESSION_PATH,
                SESSION_NAME,
                "Changed",
                Some(&(raw,).to_variant()),
            );
            let network =
                if !view.network.available {
                    "Network unavailable"
                } else if view.network.offline {
                    "Network off"
                } else if view.network.services.iter().any(|s| {
                    s.kind == "ethernet" && ["ready", "online"].contains(&s.state.as_str())
                }) {
                    "Ethernet"
                } else if view
                    .network
                    .services
                    .iter()
                    .any(|s| s.kind == "wifi" && ["ready", "online"].contains(&s.state.as_str()))
                {
                    "Wi-Fi"
                } else {
                    "Disconnected"
                };
            let output = view
                .audio
                .outputs
                .iter()
                .find(|d| d.name == view.audio.default_output);
            let volume = output
                .map(|d| {
                    if d.muted {
                        "Muted".into()
                    } else {
                        format!("Vol {:.0}%", d.volume)
                    }
                })
                .unwrap_or_else(|| "Audio unavailable".into());
            let power = view.power.clone();
            let icons = panel::status(&view);
            glib::MainContext::default().spawn_local(async move {
                let _ =
                    gio::spawn_blocking(move || ironbar(network, &volume, &power, &icons)).await;
            });
        });
    }
    fn start_audio(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        match audio::Client::new(Rc::new(move |audio| {
            if let Some(app) = weak.upgrade() {
                let unavailable = !audio.available;
                app.view.borrow_mut().audio = audio;
                app.publish();
                if unavailable {
                    app.retry_audio();
                }
            }
        })) {
            Ok(audio) => {
                self.audio.replace(Some(audio));
            }
            Err(_) => {}
        }
        self.retry_audio();
    }
    fn retry_audio(self: &Rc<Self>) {
        if self.audio_retry.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_secs(3), move || {
            if let Some(app) = weak.upgrade() {
                app.audio_retry.set(false);
                if !app.audio.borrow().as_ref().is_some_and(|a| a.ready()) {
                    app.start_audio();
                }
            }
        });
    }
    async fn system(&self, operation: &SystemAction) -> Result<serde_json::Value> {
        let request =
            serde_json::to_string(operation).map_err(|_| "Cannot encode system operation")?;
        let reply = bus::call(
            &self.system,
            SYSTEM_NAME,
            SYSTEM_PATH,
            SYSTEM_NAME,
            "Execute",
            Some(&(request,).to_variant()),
        )
        .await?;
        let raw = reply.get::<(String,)>().ok_or("Invalid system response")?.0;
        serde_json::from_str(&raw).map_err(|_| "Invalid system response".into())
    }
    async fn execute(self: &Rc<Self>, action: Action, caller: &str) -> Result<serde_json::Value> {
        match &action {
            Action::Refresh => {
                self.view.borrow_mut().error = None;
                self.view.borrow_mut().about = information();
                if let Some(network) = self.network.borrow().clone() {
                    let _ = network.reload().await;
                }
                self.view.borrow_mut().datetime = clock::status();
                self.refresh_power();
                self.queue_brightness(true);
                self.publish();
            }
            Action::SetPowerProfile { profile } => {
                let value = self
                    .system(&SystemAction::SetPowerProfile { profile: *profile })
                    .await?;
                self.view.borrow_mut().power =
                    serde_json::from_value(value).map_err(|_| "Invalid power state")?;
                self.publish();
            }
            Action::Networking { .. }
            | Action::Wifi { .. }
            | Action::Scan
            | Action::Connect { .. }
            | Action::Disconnect { .. }
            | Action::Forget { .. }
            | Action::AutoConnect { .. }
            | Action::ConfigureConnection { .. }
            | Action::AuthReply { .. }
            | Action::AuthCancel { .. } => {
                let network = self.network.borrow().clone().ok_or("ConnMan unavailable")?;
                let forgotten = if let Action::Forget { service } = &action {
                    network
                        .view
                        .borrow()
                        .services
                        .iter()
                        .find(|s| s.path == *service && s.kind == "wifi")
                        .map(|s| s.name.clone())
                } else {
                    None
                };
                network.execute(&action).await?;
                if let Some(ssid) = forgotten {
                    self.system(&SystemAction::ForgetCredentials { ssid })
                        .await?;
                }
            }
            Action::ConnectHidden { ssid, passphrase } => {
                self.system(&SystemAction::HiddenProfile {
                    ssid: ssid.clone(),
                    passphrase: passphrase.clone(),
                })
                .await?;
                let network = self.network.borrow().clone().ok_or("ConnMan unavailable")?;
                // iwd's protected Hidden profile adds the SSID to directed scans.
                // The first scan can be passive; it must find the hidden BSS
                // before iwd includes provisioned SSIDs in a directed scan.
                let mut found = None;
                for attempt in 0..80 {
                    if attempt % 12 == 0 {
                        if let Err(error) = network.execute(&Action::Scan).await {
                            if !["Busy", "InProgress", "in progress"]
                                .iter()
                                .any(|word| error.contains(word))
                            {
                                return Err(error);
                            }
                        }
                    }
                    network.reload().await?;
                    found = network
                        .view
                        .borrow()
                        .services
                        .iter()
                        .find(|s| s.name == *ssid && s.kind == "wifi")
                        .map(|s| s.path.clone());
                    if found.is_some() {
                        break;
                    }
                    glib::timeout_future(Duration::from_millis(250)).await;
                }
                let found=found.ok_or("Hidden network not found. Verify its name, signal and radio state, then scan again.")?;
                network.execute(&Action::Connect { service: found }).await?;
            }
            Action::SetVolume { .. }
            | Action::SetMute { .. }
            | Action::SetDefault { .. }
            | Action::SetBalance { .. }
            | Action::SetPort { .. }
            | Action::SetProfile { .. }
            | Action::SetStream { .. } => {
                let audio = self.audio.borrow().clone().ok_or("Audio is unavailable")?;
                audio.execute(&action).await?;
            }
            Action::AdjustVolume { delta } => {
                if !delta.is_finite() || delta.abs() > 100. {
                    return Err("Invalid volume adjustment".into());
                }
                let audio = self.audio.borrow().clone().ok_or("Audio is unavailable")?;
                let device = audio.device(false, "")?;
                let volume = audio
                    .view
                    .borrow()
                    .outputs
                    .iter()
                    .find(|d| d.name == device)
                    .map(|d| d.volume)
                    .unwrap_or(0.);
                audio
                    .execute(&Action::SetVolume {
                        input: false,
                        device,
                        percent: (volume + delta).clamp(0., 100.),
                    })
                    .await?;
            }
            Action::ToggleMute => {
                let audio = self.audio.borrow().clone().ok_or("Audio is unavailable")?;
                let device = audio.device(false, "")?;
                let muted = !audio
                    .view
                    .borrow()
                    .outputs
                    .iter()
                    .find(|d| d.name == device)
                    .map(|d| d.muted)
                    .unwrap_or(false);
                audio
                    .execute(&Action::SetMute {
                        input: false,
                        device,
                        muted,
                    })
                    .await?;
            }
            Action::Brightness { id, percent } => {
                self.brightness(id, *percent).await?;
            }
            Action::AdjustBrightness { delta } => {
                if !delta.is_finite() || delta.abs() > 100. {
                    return Err("Invalid brightness adjustment".into());
                }
                let b = self
                    .view
                    .borrow()
                    .displays
                    .brightness
                    .first()
                    .cloned()
                    .ok_or("No controllable brightness device")?;
                self.brightness(&b.id, (b.percent + delta).clamp(1., 100.))
                    .await?;
            }
            Action::ApplyDisplays { outputs, hdr } => {
                return self.apply_displays(outputs.clone(), *hdr, caller).await;
            }
            Action::ConfirmDisplays { id } => {
                if !self
                    .pending
                    .borrow()
                    .as_ref()
                    .is_some_and(|p| p.id == *id && p.caller == caller)
                {
                    return Err("Display confirmation expired or belongs to another window".into());
                }
                self.pending.borrow_mut().take();
                self.view.borrow_mut().displays.pending = None;
                let outputs = self.view.borrow().displays.outputs.clone();
                files::save_json(&files::config_dir().join("displays.json"), &outputs, 0o600)?;
                self.publish();
            }
            Action::RevertDisplays => {
                self.revert().await?;
            }
            Action::SetAppearance { appearance } => {
                self.set_appearance(appearance.clone(), true).await?;
            }
            Action::System { operation } => {
                if matches!(operation, SystemAction::DdcDiscover) {
                    self.queue_brightness(true);
                    return Ok(serde_json::json!([]));
                }
                let value = match self.system(operation).await {
                    Ok(value) => value,
                    Err(_) if matches!(operation, SystemAction::DdcDiscover) => {
                        self.view.borrow_mut().displays.brightness_status="External-display brightness is unavailable. An active local session and DDC/CI support are required.".into();
                        self.publish();
                        return Ok(serde_json::json!([]));
                    }
                    Err(error) => return Err(error),
                };
                if matches!(
                    operation,
                    SystemAction::PowerStatus
                        | SystemAction::SetPowerButtons { .. }
                        | SystemAction::SetPowerProfile { .. }
                ) {
                    self.view.borrow_mut().power =
                        serde_json::from_value(value.clone()).map_err(|_| "Invalid power state")?;
                    self.publish();
                }
                if matches!(
                    operation,
                    SystemAction::SetNtp { .. }
                        | SystemAction::SetTimezone { .. }
                        | SystemAction::SetTime { .. }
                ) {
                    if let Ok(value) = self.system(&SystemAction::Status).await {
                        if let Ok(status) = serde_json::from_value(value) {
                            self.view.borrow_mut().datetime = status;
                            self.publish();
                        }
                    }
                }
                return Ok(value);
            }
        }
        Ok(serde_json::json!({"ok":true}))
    }
    async fn brightness(self: &Rc<Self>, id: &str, percent: f64) -> Result<()> {
        validate::percent(percent)?;
        if self
            .view
            .borrow()
            .displays
            .brightness
            .iter()
            .any(|b| b.id == id && b.ddc)
        {
            self.system(&SystemAction::DdcBrightness {
                id: id.into(),
                percent,
            })
            .await?;
        } else {
            let device = id
                .strip_prefix("backlight:")
                .ok_or("Unknown brightness device")?;
            let available = backlights();
            if !available.iter().any(|b| b.id == id) {
                return Err("Backlight disappeared".into());
            }
            let maximum = fs::read_to_string(
                Path::new("/sys/class/backlight")
                    .join(device)
                    .join("max_brightness"),
            )
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .ok_or("Invalid backlight range")?;
            let value = ((percent * maximum as f64 / 100.).round() as u32).clamp(1, maximum);
            bus::call(
                &self.system,
                "org.freedesktop.login1",
                "/org/freedesktop/login1/session/auto",
                "org.freedesktop.login1.Session",
                "SetBrightness",
                Some(&("backlight", device, value).to_variant()),
            )
            .await?;
        }
        if let Some(b) = self
            .view
            .borrow_mut()
            .displays
            .brightness
            .iter_mut()
            .find(|b| b.id == id)
        {
            b.percent = percent;
        }
        self.publish();
        Ok(())
    }
    async fn apply_displays(
        self: &Rc<Self>,
        outputs: Vec<OutputConfig>,
        hdr: bool,
        caller: &str,
    ) -> Result<serde_json::Value> {
        if self.applying.replace(true) {
            return Err("A display change is already being tested".into());
        }
        let _applying = Applying(&self.applying);
        if self.pending.borrow().is_some() {
            return Err("Confirm or revert the previous display change first".into());
        }
        validate::displays(&outputs, &self.view.borrow().displays.outputs)?;
        let client = self
            .display
            .borrow()
            .clone()
            .ok_or("Output management is unavailable")?;
        let previous = self
            .view
            .borrow()
            .displays
            .outputs
            .iter()
            .map(Output::config)
            .collect();
        let previous_hdr = self.view.borrow().displays.hdr_requested;
        let path = hdr_path();
        let previous_xml = fs::read(&path).ok();
        if hdr != previous_hdr {
            if hdr && self.view.borrow().displays.renderer != "vulkan" {
                return Err(
                    "HDR needs the Vulkan renderer; log in using hardware Vulkan first".into(),
                );
            }
            write_hdr(hdr)?;
        }
        if let Err(error) = client.apply(outputs).await {
            restore_xml(&path, &previous_xml)?;
            let _ = Command::new("labwc").arg("--reconfigure").status();
            return Err(error);
        }
        let id = files::unique();
        self.pending.replace(Some(Pending {
            id,
            previous,
            hdr: previous_hdr,
            caller: caller.into(),
            previous_xml,
        }));
        {
            let mut v = self.view.borrow_mut();
            v.displays.pending = Some(id);
            v.displays.hdr_requested = hdr;
        }
        self.publish();
        let owner = bus::call(
            &self.bus,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "NameHasOwner",
            Some(&(caller,).to_variant()),
        )
        .await?;
        if owner.get::<(bool,)>() != Some((true,)) {
            self.revert().await?;
            return Err(
                "The requesting application closed; the display change was reverted".into(),
            );
        }
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_secs(15), move || {
            if let Some(app) = weak.upgrade() {
                if app.pending.borrow().as_ref().is_some_and(|p| p.id == id) {
                    let task = app.clone();
                    glib::MainContext::default().spawn_local(async move {
                        let _ = task.revert().await;
                    });
                }
            }
        });
        Ok(serde_json::json!({"id":id,"confirmation_seconds":15}))
    }
    async fn revert(self: &Rc<Self>) -> Result<()> {
        let Some(pending) = self.pending.borrow_mut().take() else {
            return Ok(());
        };
        restore_xml(&hdr_path(), &pending.previous_xml)?;
        let _ = Command::new("labwc").arg("--reconfigure").status();
        let current = self.view.borrow().displays.outputs.clone();
        let mut configs = current
            .iter()
            .map(|o| {
                pending
                    .previous
                    .iter()
                    .find(|c| c.name == o.name)
                    .cloned()
                    .unwrap_or_else(|| o.config())
            })
            .collect::<Vec<_>>();
        if !configs.iter().any(|c| c.enabled) {
            if let Some(c) = configs.first_mut() {
                c.enabled = true;
                c.mode = current[0]
                    .modes
                    .iter()
                    .find(|m| m.preferred)
                    .or(current[0].modes.first())
                    .cloned();
            }
        }
        {
            let mut v = self.view.borrow_mut();
            v.displays.pending = None;
            v.displays.hdr_requested = pending.hdr;
        }
        let client = self
            .display
            .borrow()
            .clone()
            .ok_or("Output management disappeared")?;
        let result = client.apply(configs).await;
        self.publish();
        result
    }
    async fn restore_displays(self: &Rc<Self>) -> Result<()> {
        if self.applying.get() {
            return Ok(());
        }
        let saved = files::read_json::<Vec<Output>>(&files::config_dir().join("displays.json"));
        let current = self.view.borrow().displays.outputs.clone();
        if saved.is_empty() && current.iter().any(|o| o.enabled) {
            return Ok(());
        }
        if current.is_empty() {
            return Ok(());
        }
        let configs = layout::restore(&saved, &current)?;
        let client = self
            .display
            .borrow()
            .clone()
            .ok_or("Output management unavailable")?;
        if client.apply(configs).await.is_err() {
            // A compositor can reject an individually valid saved combination.
            // Retry once with a single preferred-mode display, never in a loop.
            client
                .apply(layout::fallback(&self.view.borrow().displays.outputs)?)
                .await?;
        }
        Ok(())
    }

    async fn set_appearance(self: &Rc<Self>, appearance: Appearance, persist: bool) -> Result<()> {
        let copy = appearance.clone();
        let mut command = gio::spawn_blocking(move || {
            if copy.wallpapers.len() > 16
                || copy.color.len() != 7
                || !copy.color.starts_with('#')
                || !copy.color[1..].bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err("Invalid background settings".to_string());
            }
            let mut command = Command::new("swaybg");
            command
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            for wallpaper in &copy.wallpapers {
                if !["fill", "fit", "stretch", "center", "tile"].contains(&wallpaper.fit.as_str())
                    || wallpaper.output.len() > 128
                    || wallpaper.output.contains('\0')
                {
                    return Err("Invalid wallpaper layout".into());
                }
                let path = Path::new(&wallpaper.file);
                let metadata = path
                    .metadata()
                    .map_err(|_| "Wallpaper file is unavailable")?;
                if !metadata.is_file() || metadata.len() > 32 * 1024 * 1024 {
                    return Err("Wallpaper must be a regular image smaller than 32 MiB".into());
                }
                let (_, width, height) =
                    gdk_pixbuf::Pixbuf::file_info(path).ok_or("Unsupported wallpaper image")?;
                if width <= 0 || height <= 0 || width as i64 * height as i64 > 16_777_216 {
                    return Err("Wallpaper exceeds the 16 megapixel memory limit".into());
                }
                command.args([
                    "-o",
                    &wallpaper.output,
                    "-c",
                    &copy.color,
                    "-i",
                    &wallpaper.file,
                    "-m",
                    &wallpaper.fit,
                ]);
            }
            if copy.wallpapers.is_empty() {
                command.args(["-c", &copy.color]);
            }
            Ok(command)
        })
        .await
        .map_err(|_| "Wallpaper worker failed")??;
        let owner = unsafe { libc::getpid() };
        // Spawn from the main thread: PDEATHSIG follows the spawning thread's
        // lifetime, so a temporary validation worker must never own this child.
        unsafe {
            command.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::getppid() != owner {
                    return Err(std::io::Error::from_raw_os_error(libc::ESRCH));
                }
                Ok(())
            });
        }
        let mut child = command
            .spawn()
            .map_err(|_| "Cannot start wallpaper renderer")?;
        glib::timeout_future(Duration::from_millis(150)).await;
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            return Err("Wallpaper renderer rejected this image".into());
        }
        if persist {
            if let Err(error) = files::save_json(
                &files::config_dir().join("appearance.json"),
                &appearance,
                0o600,
            ) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        }
        if let Some(mut old) = self.wallpaper.replace(Some(child)) {
            let _ = old.kill();
            let _ = old.wait();
        }
        self.view.borrow_mut().appearance = appearance;
        self.publish();
        Ok(())
    }
}
fn hdr_path() -> PathBuf {
    files::config_dir().parent().unwrap().join("labwc/rc.xml")
}
fn write_hdr(enabled: bool) -> Result<()> {
    let path = hdr_path();
    let xml = match fs::read_to_string(&path) {
        Ok(xml) => xml,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            "<labwc_config></labwc_config>".into()
        }
        Err(_) => return Err("Cannot read your labwc configuration".into()),
    };
    let xml = files::set_hdr_xml(&xml, enabled)?;
    files::atomic(&path, xml.as_bytes(), 0o600)?;
    Command::new("labwc")
        .arg("--reconfigure")
        .status()
        .map_err(|_| "Cannot reload compositor settings")?;
    Ok(())
}
fn restore_xml(path: &Path, previous: &Option<Vec<u8>>) -> Result<()> {
    match previous {
        Some(bytes) => files::atomic(path, bytes, 0o600),
        None => {
            let _ = fs::remove_file(path);
            Ok(())
        }
    }
}
fn backlights() -> Vec<Brightness> {
    devices::backlights(Path::new("/sys/class/backlight"))
}

fn information() -> BTreeMap<String, String> {
    let mut data = BTreeMap::new();
    let os = fs::read_to_string("/etc/os-release").unwrap_or_default();
    for line in os.lines() {
        if let Some((key, value)) = line.split_once('=') {
            if ["PRETTY_NAME", "VERSION_ID"].contains(&key) {
                data.insert(key.into(), value.trim_matches('"').into());
            }
        }
    }
    data.insert("Architecture".into(), std::env::consts::ARCH.into());
    data.insert(
        "Kernel".into(),
        fs::read_to_string("/proc/sys/kernel/osrelease")
            .unwrap_or_default()
            .trim()
            .into(),
    );
    data.insert(
        "Hostname".into(),
        fs::read_to_string("/proc/sys/kernel/hostname")
            .unwrap_or_default()
            .trim()
            .into(),
    );
    if let Some(cpu) = fs::read_to_string("/proc/cpuinfo")
        .unwrap_or_default()
        .lines()
        .find_map(|l| {
            l.split_once(':')
                .filter(|(k, _)| ["model name", "Processor", "cpu"].contains(&k.trim()))
                .map(|(_, v)| v.trim().to_string())
        })
    {
        data.insert("Processor".into(), cpu);
    }
    for line in fs::read_to_string("/proc/meminfo")
        .unwrap_or_default()
        .lines()
    {
        if let Some((key, value)) = line.split_once(':') {
            if ["MemTotal", "MemAvailable"].contains(&key) {
                data.insert(key.into(), value.trim().into());
            }
        }
    }
    let mut storage: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c"/".as_ptr(), &mut storage) } == 0 {
        data.insert(
            "Root storage".into(),
            format!(
                "{:.1} GiB available / {:.1} GiB total",
                storage.f_bavail as f64 * storage.f_frsize as f64 / 1073741824.,
                storage.f_blocks as f64 * storage.f_frsize as f64 / 1073741824.
            ),
        );
    }
    data.insert("Session".into(), "labwc / Wayland".into());
    data.insert("Settings".into(), env!("CARGO_PKG_VERSION").into());
    data
}
fn ironbar(
    network: &str,
    volume: &str,
    power: &Power,
    icons: &[(String, String); 3],
) -> Result<()> {
    let path =
        PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").ok_or("Missing runtime directory")?)
            .join("ironbar-ipc.sock");
    let marker = |profile: PowerProfile| {
        format!(
            "{} {}",
            if power.profile == Some(profile) {
                "●"
            } else {
                "○"
            },
            profile.label()
        )
    };
    for (key, value) in [
        ("uflinux_network_icon", icons[0].0.clone()),
        ("uflinux_network_tooltip", icons[0].1.clone()),
        ("uflinux_sound_icon", icons[1].0.clone()),
        ("uflinux_sound_tooltip", icons[1].1.clone()),
        ("uflinux_power_icon", icons[2].0.clone()),
        ("uflinux_power_tooltip", icons[2].1.clone()),
        ("uflinux_network", network.to_string()),
        ("uflinux_volume", volume.to_string()),
        (
            "uflinux_power",
            power
                .profile
                .map(PowerProfile::label)
                .unwrap_or("Unavailable")
                .into(),
        ),
        ("uflinux_power_saver", marker(PowerProfile::PowerSaver)),
        ("uflinux_power_balanced", marker(PowerProfile::Balanced)),
        (
            "uflinux_power_performance",
            marker(PowerProfile::Performance),
        ),
    ] {
        let mut socket = UnixStream::connect(&path).map_err(|_| "Panel is not running")?;
        socket
            .set_read_timeout(Some(Duration::from_millis(150)))
            .map_err(|e| e.to_string())?;
        socket
            .set_write_timeout(Some(Duration::from_millis(150)))
            .map_err(|e| e.to_string())?;
        let message =
            serde_json::json!({"command":"var","subcommand":"set","key":key,"value":value})
                .to_string();
        socket
            .write_all(format!("{message}\n").as_bytes())
            .map_err(|e| e.to_string())?;
        let mut reply = String::new();
        BufReader::new(socket)
            .read_line(&mut reply)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
