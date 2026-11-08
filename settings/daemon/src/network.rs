use glib::variant::{ObjectPath, ToVariant};
use settings_core::{
    bus::{self, Properties},
    *,
};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashMap},
    rc::Rc,
};

const NAME: &str = "net.connman";
const MANAGER: &str = "net.connman.Manager";
const AGENT: &str = "/org/uflinux/Settings/Agent";
const AGENT_XML: &str = r#"<node><interface name="net.connman.Agent">
<method name="Release"/><method name="Cancel"/>
<method name="ReportError"><arg type="o" direction="in"/><arg type="s" direction="in"/></method>
<method name="RequestBrowser"><arg type="o" direction="in"/><arg type="s" direction="in"/></method>
<method name="RequestInput"><arg type="o" direction="in"/><arg type="a{sv}" direction="in"/><arg type="a{sv}" direction="out"/></method>
</interface></node>"#;

pub struct Client {
    pub bus: gio::DBusConnection,
    pub view: RefCell<Network>,
    owner: RefCell<String>,
    waiting: RefCell<HashMap<u64, gio::DBusMethodInvocation>>,
    scheduled: Cell<bool>,
    again: Cell<bool>,
    emit: Rc<dyn Fn(Network)>,
}
impl Client {
    pub async fn new(emit: Rc<dyn Fn(Network)>) -> Result<Rc<Self>> {
        let bus = gio::bus_get_future(gio::BusType::System)
            .await
            .map_err(|e| e.to_string())?;
        let this = Rc::new(Self {
            bus,
            view: RefCell::new(Network::default()),
            owner: RefCell::new(String::new()),
            waiting: RefCell::new(HashMap::new()),
            scheduled: Cell::new(false),
            again: Cell::new(false),
            emit,
        });
        let info = gio::DBusNodeInfo::for_xml(AGENT_XML)
            .map_err(|e| e.to_string())?
            .interfaces()[0]
            .clone();
        let weak = Rc::downgrade(&this);
        this.bus
            .register_object(AGENT, &info)
            .method_call(move |_, sender, _, _, method, args, invocation| {
                if let Some(this) = weak.upgrade() {
                    this.agent(sender, method, args, invocation);
                }
            })
            .build()
            .map_err(|e| e.to_string())?;
        let weak = Rc::downgrade(&this);
        this.bus.signal_subscribe(
            Some(NAME),
            None,
            None,
            None,
            None,
            gio::DBusSignalFlags::NONE,
            move |_, _, _, _, _, _| {
                if let Some(this) = weak.upgrade() {
                    this.schedule();
                }
            },
        );
        let weak = Rc::downgrade(&this);
        this.bus.signal_subscribe(
            Some("org.freedesktop.DBus"),
            Some("org.freedesktop.DBus"),
            Some("NameOwnerChanged"),
            Some("/org/freedesktop/DBus"),
            Some(NAME),
            gio::DBusSignalFlags::NONE,
            move |_, _, _, _, _, _| {
                if let Some(this) = weak.upgrade() {
                    this.cancel_auth();
                    let task = this.clone();
                    glib::MainContext::default().spawn_local(async move {
                        task.initialize().await;
                    });
                }
            },
        );
        this.initialize().await;
        Ok(this)
    }
    fn publish(&self) {
        (self.emit)(self.view.borrow().clone());
    }
    fn schedule(self: &Rc<Self>) {
        if self.scheduled.replace(true) {
            self.again.set(true);
            return;
        }
        let this = self.clone();
        glib::idle_add_local_once(move || {
            glib::MainContext::default().spawn_local(async move {
                let _ = this.reload().await;
                this.scheduled.set(false);
                if this.again.replace(false) {
                    this.schedule();
                }
            });
        });
    }
    async fn initialize(self: &Rc<Self>) {
        let owner = bus::call(
            &self.bus,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "GetNameOwner",
            Some(&(NAME,).to_variant()),
        )
        .await;
        *self.owner.borrow_mut() = owner
            .ok()
            .and_then(|v| v.get::<(String,)>())
            .map(|v| v.0)
            .unwrap_or_default();
        if self.owner.borrow().is_empty() {
            *self.view.borrow_mut() = Network::default();
            self.publish();
            return;
        }
        let path = ObjectPath::try_from(AGENT).expect("constant object path");
        let _ = self
            .manager("RegisterAgent", Some(&(path,).to_variant()))
            .await;
        let _ = self.reload().await;
    }
    pub async fn manager(
        &self,
        method: &str,
        args: Option<&glib::Variant>,
    ) -> Result<glib::Variant> {
        bus::call(&self.bus, NAME, "/", MANAGER, method, args).await
    }
    async fn service_call(
        &self,
        path: &str,
        method: &str,
        args: Option<&glib::Variant>,
    ) -> Result<glib::Variant> {
        validate::service(path)?;
        if !self.view.borrow().services.iter().any(|s| s.path == path) {
            return Err("The connection is no longer available".into());
        }
        bus::call(&self.bus, NAME, path, "net.connman.Service", method, args).await
    }
    pub async fn reload(&self) -> Result<()> {
        let properties = self
            .manager("GetProperties", None)
            .await?
            .get::<(Properties,)>()
            .ok_or("Invalid ConnMan properties")?
            .0;
        let technologies = self
            .manager("GetTechnologies", None)
            .await?
            .get::<(Vec<(ObjectPath, Properties)>,)>()
            .ok_or("Invalid ConnMan technology list")?
            .0;
        let services = self
            .manager("GetServices", None)
            .await?
            .get::<(Vec<(ObjectPath, Properties)>,)>()
            .ok_or("Invalid ConnMan connection list")?
            .0;
        {
            let mut view = self.view.borrow_mut();
            view.available = true;
            view.state = bus::text(&properties, "State");
            view.offline = bus::boolean(&properties, "OfflineMode");
            view.wifi_hardware_blocked = false;
            view.wifi_software_blocked = false;
            for entry in std::fs::read_dir("/sys/class/rfkill")
                .into_iter()
                .flatten()
                .flatten()
            {
                if std::fs::read_to_string(entry.path().join("type"))
                    .unwrap_or_default()
                    .trim()
                    == "wlan"
                {
                    view.wifi_hardware_blocked |=
                        std::fs::read_to_string(entry.path().join("hard"))
                            .unwrap_or_default()
                            .trim()
                            == "1";
                    view.wifi_software_blocked |=
                        std::fs::read_to_string(entry.path().join("soft"))
                            .unwrap_or_default()
                            .trim()
                            == "1";
                }
            }
            view.technologies = technologies
                .into_iter()
                .map(|(path, p)| Technology {
                    path: path.to_string(),
                    kind: bus::text(&p, "Type"),
                    powered: bus::boolean(&p, "Powered"),
                    connected: bus::boolean(&p, "Connected"),
                })
                .collect();
            view.services = services
                .into_iter()
                .map(|(path, p)| {
                    let ethernet = bus::dict(&p, "Ethernet");
                    let proxy = bus::dict(&p, "Proxy.Configuration");
                    Service {
                        path: path.to_string(),
                        name: bus::text(&p, "Name"),
                        kind: bus::text(&p, "Type"),
                        state: bus::text(&p, "State"),
                        security: bus::strings(&p, "Security"),
                        strength: p.get("Strength").and_then(|v| v.get::<u8>()).unwrap_or(0),
                        favorite: bus::boolean(&p, "Favorite"),
                        autoconnect: bus::boolean(&p, "AutoConnect"),
                        interface: bus::text(&ethernet, "Interface"),
                        address: bus::text(&ethernet, "Address"),
                        ipv4: bus::text_dict(&p, "IPv4"),
                        ipv6: bus::text_dict(&p, "IPv6"),
                        dns: bus::strings(&p, "Nameservers"),
                        domains: bus::strings(&p, "Domains"),
                        proxy: Proxy {
                            method: bus::text(&proxy, "Method"),
                            url: bus::text(&proxy, "URL"),
                            servers: bus::strings(&proxy, "Servers"),
                            excludes: bus::strings(&proxy, "Excludes"),
                        },
                        config: ConnectionConfig {
                            ipv4: bus::text_dict(&p, "IPv4.Configuration"),
                            ipv6: bus::text_dict(&p, "IPv6.Configuration"),
                            dns: bus::strings(&p, "Nameservers.Configuration"),
                            domains: bus::strings(&p, "Domains.Configuration"),
                            proxy: Proxy {
                                method: bus::text(&proxy, "Method"),
                                url: bus::text(&proxy, "URL"),
                                servers: bus::strings(&proxy, "Servers"),
                                excludes: bus::strings(&proxy, "Excludes"),
                            },
                        },
                    }
                })
                .collect();
        }
        self.publish();
        Ok(())
    }
    pub async fn execute(&self, action: &Action) -> Result<()> {
        match action {
            Action::Networking { enabled } => {
                self.manager(
                    "SetProperty",
                    Some(&("OfflineMode", (!enabled).to_variant()).to_variant()),
                )
                .await?;
            }
            Action::Wifi { enabled } => {
                let technologies = self.view.borrow().technologies.clone();
                let wifi = technologies
                    .iter()
                    .filter(|t| t.kind == "wifi")
                    .collect::<Vec<_>>();
                if wifi.is_empty() {
                    return Err(
                        "No Wi-Fi device is available. Check the hardware switch or firmware."
                            .into(),
                    );
                }
                for t in wifi {
                    bus::call(
                        &self.bus,
                        NAME,
                        &t.path,
                        "net.connman.Technology",
                        "SetProperty",
                        Some(&("Powered", enabled.to_variant()).to_variant()),
                    )
                    .await?;
                }
            }
            Action::Scan => {
                let technologies = self.view.borrow().technologies.clone();
                let t = technologies
                    .iter()
                    .find(|t| t.kind == "wifi" && t.powered)
                    .ok_or("Turn Wi-Fi on before scanning")?;
                bus::call(
                    &self.bus,
                    NAME,
                    &t.path,
                    "net.connman.Technology",
                    "Scan",
                    None,
                )
                .await?;
            }
            Action::Connect { service } => {
                self.service_call(service, "Connect", None).await?;
            }
            Action::Disconnect { service } => {
                self.service_call(service, "Disconnect", None).await?;
            }
            Action::Forget { service } => {
                self.service_call(service, "Remove", None).await?;
            }
            Action::AutoConnect { service, enabled } => {
                self.service_call(
                    service,
                    "SetProperty",
                    Some(&("AutoConnect", enabled.to_variant()).to_variant()),
                )
                .await?;
            }
            Action::ConfigureConnection { service, config } => {
                validate::connection(config)?;
                let old = self
                    .view
                    .borrow()
                    .services
                    .iter()
                    .find(|s| &s.path == service)
                    .cloned()
                    .ok_or("Connection disappeared")?;
                // ConnMan validates every property. Stop immediately if it rejects one.
                let mut values: Vec<(&str, glib::Variant)> = Vec::new();
                for (key, value) in [
                    ("IPv4.Configuration", &config.ipv4),
                    ("IPv6.Configuration", &config.ipv6),
                ] {
                    let map: Properties = value
                        .iter()
                        .map(|(k, v)| {
                            (
                                k.clone(),
                                if k == "PrefixLength" {
                                    v.parse::<u8>().unwrap_or(64).to_variant()
                                } else {
                                    v.to_variant()
                                },
                            )
                        })
                        .collect();
                    if !map.is_empty() {
                        values.push((key, map.to_variant()));
                    }
                }
                values.push(("Nameservers.Configuration", config.dns.to_variant()));
                values.push(("Domains.Configuration", config.domains.to_variant()));
                let mut proxy: Properties = HashMap::new();
                proxy.insert(
                    "Method".into(),
                    if config.proxy.method.is_empty() {
                        "direct"
                    } else {
                        &config.proxy.method
                    }
                    .to_variant(),
                );
                if config.proxy.method == "auto" {
                    proxy.insert("URL".into(), config.proxy.url.to_variant());
                }
                if config.proxy.method == "manual" {
                    proxy.insert("Servers".into(), config.proxy.servers.to_variant());
                    proxy.insert("Excludes".into(), config.proxy.excludes.to_variant());
                }
                values.push(("Proxy.Configuration", proxy.to_variant()));
                for (key, value) in values {
                    if self
                        .service_call(service, "SetProperty", Some(&(key, value).to_variant()))
                        .await
                        .is_err()
                    {
                        return Err(format!(
                            "ConnMan rejected {key} for {}. Earlier fields may already have been applied; review the connection.",
                            old.name
                        ));
                    }
                }
            }
            Action::AuthCancel { id } => self.reply(*id, None)?,
            Action::AuthReply { id, values } => self.reply(*id, Some(values))?,
            _ => return Err("Not a networking operation".into()),
        }
        let _ = self.reload().await;
        Ok(())
    }
    fn reply(&self, id: u64, values: Option<&BTreeMap<String, String>>) -> Result<()> {
        if let Some(values) = values {
            if values.iter().any(|(k, v)| {
                !["Name", "Identity", "Passphrase", "WPS"].contains(&k.as_str())
                    || v.len() > 1024
                    || v.contains('\0')
            }) {
                return Err("Invalid authentication response".into());
            }
        }
        let invocation = self
            .waiting
            .borrow_mut()
            .remove(&id)
            .ok_or("Authentication request expired")?;
        if let Some(values) = values {
            let values: Properties = values
                .iter()
                .map(|(k, v)| (k.clone(), v.to_variant()))
                .collect();
            invocation.return_value(Some(&(values,).to_variant()));
        } else {
            invocation.return_dbus_error("net.connman.Agent.Error.Canceled", "Canceled by user");
        }
        self.view.borrow_mut().authentication = None;
        self.publish();
        Ok(())
    }
    fn cancel_auth(&self) {
        for (_, v) in self.waiting.borrow_mut().drain() {
            v.return_dbus_error("net.connman.Agent.Error.Canceled", "Request canceled");
        }
        self.view.borrow_mut().authentication = None;
        self.publish();
    }
    fn agent(
        self: &Rc<Self>,
        sender: Option<&str>,
        method: &str,
        args: glib::Variant,
        invocation: gio::DBusMethodInvocation,
    ) {
        if sender != Some(self.owner.borrow().as_str()) || self.owner.borrow().is_empty() {
            invocation.return_dbus_error(
                "org.freedesktop.DBus.Error.AccessDenied",
                "Only ConnMan may call this agent",
            );
            return;
        }
        match method {
            "RequestInput" => {
                self.cancel_auth();
                let Some((service, fields)) = args.get::<(ObjectPath, Properties)>() else {
                    invocation
                        .return_dbus_error("net.connman.Agent.Error.Canceled", "Invalid fields");
                    return;
                };
                let fields: Vec<AuthField> = fields
                    .into_iter()
                    .filter(|(k, _)| {
                        ["Name", "Identity", "Passphrase", "WPS"].contains(&k.as_str())
                    })
                    .map(|(name, p)| {
                        let p = bus::properties(&p);
                        AuthField {
                            secret: bus::text(&p, "Type") == "psk" || name == "Passphrase",
                            required: bus::text(&p, "Requirement") == "mandatory",
                            name,
                        }
                    })
                    .collect();
                let id = files::unique();
                let name = self
                    .view
                    .borrow()
                    .services
                    .iter()
                    .find(|s| s.path == service.as_str())
                    .map(|s| s.name.clone())
                    .unwrap_or_else(|| "Wi-Fi".into());
                self.waiting.borrow_mut().insert(id, invocation);
                self.view.borrow_mut().authentication = Some(AuthRequest {
                    id,
                    service: service.to_string(),
                    name,
                    fields,
                });
                self.publish();
                let _ = std::process::Command::new("uflinux-settings")
                    .args(["--network-auth", &id.to_string()])
                    .spawn();
                let weak = Rc::downgrade(self);
                glib::timeout_add_local_once(std::time::Duration::from_secs(90), move || {
                    if let Some(this) = weak.upgrade() {
                        let _ = this.reply(id, None);
                    }
                });
            }
            "RequestBrowser" => {
                if let Some((_, url)) = args.get::<(ObjectPath, String)>() {
                    if validate::http_url(&url) {
                        self.view.borrow_mut().portal = Some(url);
                        self.publish();
                    }
                }
                invocation.return_value(Some(&().to_variant()));
            }
            "Release" | "Cancel" => {
                self.cancel_auth();
                invocation.return_value(Some(&().to_variant()));
            }
            "ReportError" => {
                invocation.return_value(Some(&().to_variant()));
            }
            _ => invocation.return_dbus_error(
                "org.freedesktop.DBus.Error.UnknownMethod",
                "Unknown agent operation",
            ),
        }
    }
}
