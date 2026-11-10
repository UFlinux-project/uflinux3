use super::*;
use settings_core::bus;
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    fs,
    io::Read,
};

pub fn page(app: &Rc<App>, parent: &gtk::Box) {
    let status = description("Connecting to ConnMan…");
    parent.append(&status);
    let target = status.downgrade();
    app.bind_for(Surface::Full, &status, move |s| {
        if let Some(label) = target.upgrade() {
            label.set_text(if !s.network.available {
                "ConnMan is unavailable. Check its OpenRC service."
            } else if s.network.wifi_hardware_blocked {
                "Wi-Fi is disabled by a hardware switch. Enable it on the device before connecting."
            } else if s.network.offline {
                "Networking is disabled."
            } else if s.network.wifi_software_blocked {
                "Wi-Fi is blocked in software. Turn Wi-Fi on to reconnect."
            } else {
                "ConnMan manages addresses, DNS and routes; iwd handles Wi-Fi authentication."
            });
        }
    });
    toggle(
        app,
        Surface::Full,
        parent,
        "Networking",
        |s| !s.network.offline,
        |enabled| Action::Networking { enabled },
    );
    toggle(
        app,
        Surface::Full,
        parent,
        "Wi-Fi",
        |s| {
            s.network
                .technologies
                .iter()
                .any(|t| t.kind == "wifi" && t.powered)
        },
        |enabled| Action::Wifi { enabled },
    );
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.append(&button(app, "Scan", || Action::Scan));
    let hidden = gtk::Button::with_label("Join hidden network…");
    let weak = Rc::downgrade(app);
    hidden.connect_clicked(move |_| {
        if let Some(app) = weak.upgrade() {
            hidden_network(&app);
        }
    });
    buttons.append(&hidden);
    let enterprise = gtk::Button::with_label("Enterprise Wi-Fi…");
    let weak = Rc::downgrade(app);
    enterprise.connect_clicked(move |_| {
        if let Some(app) = weak.upgrade() {
            enterprise_network(&app);
        }
    });
    buttons.append(&enterprise);
    parent.append(&buttons);
    section(parent, "Connections");
    list(app, parent, Surface::Full, false);
}
pub fn list(app: &Rc<App>, parent: &gtk::Box, surface: Surface, compact: bool) {
    let model = gtk::StringList::new(&[]);
    let selection = gtk::NoSelection::new(Some(model.clone()));
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().unwrap();
        item.set_child(Some(&gtk::Box::new(gtk::Orientation::Vertical, 0)));
    });
    let weak = Rc::downgrade(app);
    factory.connect_bind(move |_, object| {
        let Some(app) = weak.upgrade() else {
            return;
        };
        let item = object.downcast_ref::<gtk::ListItem>().unwrap();
        let Some(value) = item.item().and_downcast::<gtk::StringObject>() else {
            return;
        };
        let Ok(service) = serde_json::from_str::<Service>(&value.string()) else {
            return;
        };
        let container = item.child().and_downcast::<gtk::Box>().unwrap();
        clear(&container);
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        line.add_css_class("settings-row");
        let text = column();
        text.set_hexpand(true);
        let name = gtk::Label::new(Some(if service.name.is_empty() {
            "Unnamed / hidden network"
        } else {
            &service.name
        }));
        name.set_xalign(0.);
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        name.add_css_class("network-name");
        text.append(&name);
        text.append(&description(&format!(
            "{} · {}{}",
            service.kind,
            service.state,
            if service.kind == "wifi" {
                format!(
                    " · {}% · {}",
                    service.strength,
                    service.security.join(" / ")
                )
            } else {
                String::new()
            }
        )));
        line.append(&text);
        let active =
            ["ready", "online", "association", "configuration"].contains(&service.state.as_str());
        let path = service.path.clone();
        line.append(&button(
            &app,
            if active { "Disconnect" } else { "Connect" },
            move || {
                if active {
                    Action::Disconnect {
                        service: path.clone(),
                    }
                } else {
                    Action::Connect {
                        service: path.clone(),
                    }
                }
            },
        ));
        if !compact {
            let details = gtk::Button::from_icon_name("emblem-system-symbolic");
            details.set_tooltip_text(Some("Connection settings"));
            let weak = Rc::downgrade(&app);
            details.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    connection(&app, service.clone());
                }
            });
            line.append(&details);
        }
        container.append(&line);
    });
    let list = gtk::ListView::new(Some(selection), Some(factory));
    list.set_single_click_activate(false);
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .min_content_height(if compact { 64 } else { 180 })
        .max_content_height(if compact { 128 } else { 460 })
        .propagate_natural_height(true)
        .child(&list)
        .build();
    parent.append(&scroll);
    let signature = Rc::new(RefCell::new(Vec::<String>::new()));
    let weak_model = model.downgrade();
    app.bind_for(surface, &list, move |s| {
        let services = s
            .network
            .services
            .iter()
            .take(if compact { 32 } else { 128 })
            .collect::<Vec<_>>();
        let values = services
            .iter()
            .filter_map(|s| serde_json::to_string(s).ok())
            .collect::<Vec<_>>();
        let mut previous = signature.borrow_mut();
        if *previous == values {
            return;
        }
        if let Some(model) = weak_model.upgrade() {
            let prefix = previous
                .iter()
                .zip(&values)
                .take_while(|(a, b)| a == b)
                .count();
            let suffix = previous[prefix..]
                .iter()
                .rev()
                .zip(values[prefix..].iter().rev())
                .take_while(|(a, b)| a == b)
                .count();
            let refs = values[prefix..values.len() - suffix]
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>();
            model.splice(
                prefix as u32,
                (previous.len() - prefix - suffix) as u32,
                &refs,
            );
        }
        *previous = values;
    });
}
pub fn authentication(app: &Rc<App>, request: AuthRequest) {
    let inline = app.quick_auth.borrow().clone();
    let (window, parent) = if let Some(parent) = inline {
        clear(&parent);
        (None, parent)
    } else {
        let (window, parent) = dialog(app, "Wi-Fi authentication");
        app.auth_window.replace(Some(window.clone()));
        (Some(window), parent)
    };
    parent.append(&description(&format!("Connect to {}", request.name)));
    let mut fields = Vec::new();
    for field in &request.fields {
        let input = if field.secret {
            secret(&parent, &field.name)
        } else {
            entry(&parent, &field.name, "")
        };
        fields.push((field.name.clone(), input));
    }
    let fields = Rc::new(fields);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let submitted = Rc::new(Cell::new(false));
    let submit = gtk::Button::with_label("Connect");
    submit.add_css_class("suggested-action");
    let weak = Rc::downgrade(app);
    let values = fields.clone();
    let done = submitted.clone();
    let id = request.id;
    submit.connect_clicked(move |button| {
        if let Some(app) = weak.upgrade() {
            let response = values
                .iter()
                .map(|(name, entry)| (name.clone(), entry.text().to_string()))
                .collect();
            done.set(true);
            app.execute(
                Action::AuthReply {
                    id,
                    values: response,
                },
                Some(button.clone().upcast()),
            );
            for (_, entry) in values.iter() {
                entry.set_text("");
            }
        }
    });
    buttons.append(&submit);
    buttons.append(&button(app, "Cancel", move || Action::AuthCancel { id }));
    parent.append(&buttons);
    if let Some(window) = window {
        let weak = Rc::downgrade(app);
        window.connect_close_request(move |_| {
            if !submitted.get() {
                if let Some(app) = weak.upgrade() {
                    if app
                        .state
                        .borrow()
                        .network
                        .authentication
                        .as_ref()
                        .is_some_and(|r| r.id == id)
                    {
                        app.execute(Action::AuthCancel { id }, None);
                    }
                }
            }
            glib::Propagation::Proceed
        });
        window.present();
    }
    if let Some((_, entry)) = fields.first() {
        entry.grab_focus();
    }
}
fn hidden_network(app: &Rc<App>) {
    let (window, parent) = dialog(app, "Join hidden Wi-Fi");
    let ssid = entry(&parent, "Network name (SSID)", "");
    let password = secret(&parent, "Password");
    parent.append(&description(
        "Leave the password empty for an open network. The exact network name is required.",
    ));
    parent.append(&button(app, "Connect", move || Action::ConnectHidden {
        ssid: ssid.text().to_string(),
        passphrase: password.text().to_string(),
    }));
    window.present();
}
fn connection(app: &Rc<App>, service: Service) {
    let (window, parent) = dialog(
        app,
        if service.name.is_empty() {
            "Connection"
        } else {
            &service.name
        },
    );
    parent.append(&description(&format!(
        "{} · {} · {}",
        service.interface, service.address, service.state
    )));
    parent.append(&description(&format!(
        "Current IPv4: {}\nCurrent IPv6: {}\nDNS: {}",
        service
            .ipv4
            .get("Address")
            .map(String::as_str)
            .unwrap_or("Unavailable"),
        service
            .ipv6
            .get("Address")
            .map(String::as_str)
            .unwrap_or("Unavailable"),
        service.dns.join(", ")
    )));
    let autoconnect = gtk::CheckButton::with_label("Connect automatically");
    autoconnect.set_active(service.autoconnect);
    parent.append(&autoconnect);
    let p = service.path.clone();
    let weak = Rc::downgrade(app);
    autoconnect.connect_toggled(move |b| {
        if let Some(app) = weak.upgrade() {
            app.execute(
                Action::AutoConnect {
                    service: p.clone(),
                    enabled: b.is_active(),
                },
                None,
            );
        }
    });
    let advanced = gtk::Expander::builder()
        .label("Addresses, DNS and proxy")
        .build();
    let form = column();
    advanced.set_child(Some(&form));
    parent.append(&advanced);
    let v4method = choose(
        &form,
        "IPv4",
        &[
            ("dhcp", "Automatic (DHCP)"),
            ("manual", "Manual"),
            ("off", "Disabled"),
        ],
        service
            .config
            .ipv4
            .get("Method")
            .map(String::as_str)
            .unwrap_or("dhcp"),
    );
    let v4address = entry(
        &form,
        "IPv4 address",
        service
            .config
            .ipv4
            .get("Address")
            .map(String::as_str)
            .unwrap_or(""),
    );
    let v4mask = entry(
        &form,
        "Netmask",
        service
            .config
            .ipv4
            .get("Netmask")
            .map(String::as_str)
            .unwrap_or("255.255.255.0"),
    );
    let v4gateway = entry(
        &form,
        "IPv4 gateway",
        service
            .config
            .ipv4
            .get("Gateway")
            .map(String::as_str)
            .unwrap_or(""),
    );
    let v6method = choose(
        &form,
        "IPv6",
        &[
            ("auto", "Automatic"),
            ("manual", "Manual"),
            ("off", "Disabled"),
        ],
        service
            .config
            .ipv6
            .get("Method")
            .map(String::as_str)
            .unwrap_or("auto"),
    );
    let v6address = entry(
        &form,
        "IPv6 address",
        service
            .config
            .ipv6
            .get("Address")
            .map(String::as_str)
            .unwrap_or(""),
    );
    let v6prefix = entry(
        &form,
        "Prefix length",
        service
            .config
            .ipv6
            .get("PrefixLength")
            .map(String::as_str)
            .unwrap_or("64"),
    );
    let v6gateway = entry(
        &form,
        "IPv6 gateway",
        service
            .config
            .ipv6
            .get("Gateway")
            .map(String::as_str)
            .unwrap_or(""),
    );
    let dns = entry(&form, "DNS servers", &service.config.dns.join(", "));
    let domains = entry(&form, "Search domains", &service.config.domains.join(", "));
    parent.append(&description(
        "Leave DNS and search domains empty to use the network’s automatic configuration.",
    ));
    let proxy = choose(
        &form,
        "Proxy",
        &[
            ("direct", "None"),
            ("auto", "Automatic (PAC)"),
            ("manual", "Manual"),
        ],
        &service.proxy.method,
    );
    let proxyurl = entry(&form, "PAC URL", &service.proxy.url);
    let servers = entry(&form, "Proxy servers", &service.proxy.servers.join(", "));
    let excludes = entry(
        &form,
        "Proxy exceptions",
        &service.proxy.excludes.join(", "),
    );
    let path = service.path.clone();
    parent.append(&button(app, "Apply connection settings", move || {
        let mut ipv4 = BTreeMap::from([(
            "Method".into(),
            ["dhcp", "manual", "off"][v4method.selected().min(2) as usize].into(),
        )]);
        if v4method.selected() == 1 {
            for (key, value) in [
                ("Address", v4address.text()),
                ("Netmask", v4mask.text()),
                ("Gateway", v4gateway.text()),
            ] {
                if !value.is_empty() {
                    ipv4.insert(key.into(), value.to_string());
                }
            }
        }
        let mut ipv6 = BTreeMap::from([(
            "Method".into(),
            ["auto", "manual", "off"][v6method.selected().min(2) as usize].into(),
        )]);
        if v6method.selected() == 1 {
            for (key, value) in [
                ("Address", v6address.text()),
                ("PrefixLength", v6prefix.text()),
                ("Gateway", v6gateway.text()),
            ] {
                if !value.is_empty() {
                    ipv6.insert(key.into(), value.to_string());
                }
            }
        }
        Action::ConfigureConnection {
            service: path.clone(),
            config: ConnectionConfig {
                ipv4,
                ipv6,
                dns: split(&dns.text()),
                domains: split(&domains.text()),
                proxy: Proxy {
                    method: ["direct", "auto", "manual"][proxy.selected().min(2) as usize].into(),
                    url: proxyurl.text().to_string(),
                    servers: split(&servers.text()),
                    excludes: split(&excludes.text()),
                },
            },
        }
    }));
    let routes = gtk::Button::with_label("Additional routes…");
    let weak = Rc::downgrade(app);
    let s = service.clone();
    routes.connect_clicked(move |_| {
        if let Some(app) = weak.upgrade() {
            route_dialog(&app, &s);
        }
    });
    parent.append(&routes);
    let path = service.path.clone();
    parent.append(&button(app, "Forget this connection", move || {
        Action::Forget {
            service: path.clone(),
        }
    }));
    window.present();
}
fn split(value: &str) -> Vec<String> {
    value
        .split([',', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}
fn route_dialog(app: &Rc<App>, service: &Service) {
    let (window, parent) = dialog(app, "Additional routes");
    parent.append(&description("Routes are saved for this connection and restored when it connects. Only routes created by UFLinux Settings are removed."));
    let current = route_summary(&service.interface);
    parent.append(&description(&current));
    let rows = Rc::new(RefCell::new(
        Vec::<(gtk::Entry, gtk::Entry, gtk::Entry)>::new(),
    ));
    let group = column();
    parent.append(&group);
    let add = gtk::Button::with_label("Add route");
    let list = rows.clone();
    let target = group.clone();
    add.connect_clicked(move |_| add_route(&target, &list, None));
    parent.append(&add);
    let path = service.path.clone();
    let values = rows.clone();
    parent.append(&button(app, "Save additional routes", move || {
        Action::System {
            operation: SystemAction::SetRoutes {
                service: path.clone(),
                routes: values
                    .borrow()
                    .iter()
                    .map(|(destination, gateway, metric)| Route {
                        destination: destination.text().to_string(),
                        gateway: if gateway.text().is_empty() {
                            None
                        } else {
                            Some(gateway.text().to_string())
                        },
                        metric: metric.text().parse().unwrap_or(u32::MAX),
                    })
                    .collect(),
            },
        }
    }));
    let path = service.path.clone();
    let target = group.clone();
    let list = rows.clone();
    let weak = Rc::downgrade(app);
    glib::MainContext::default().spawn_local(async move {
        match bus::execute(&Action::System {
            operation: SystemAction::GetRoutes { service: path },
        })
        .await
        {
            Ok(value) => {
                if let Ok(routes) = serde_json::from_value::<Vec<Route>>(value) {
                    for route in routes {
                        add_route(&target, &list, Some(route));
                    }
                }
            }
            Err(e) => {
                if let Some(app) = weak.upgrade() {
                    app.error(&e);
                }
            }
        }
    });
    window.present();
}
fn add_route(
    parent: &gtk::Box,
    rows: &Rc<RefCell<Vec<(gtk::Entry, gtk::Entry, gtk::Entry)>>>,
    route: Option<Route>,
) {
    let group = column();
    group.add_css_class("settings-group");
    let destination = entry(
        &group,
        "Destination (CIDR)",
        route.as_ref().map(|r| r.destination.as_str()).unwrap_or(""),
    );
    let gateway = entry(
        &group,
        "Gateway (optional)",
        route
            .as_ref()
            .and_then(|r| r.gateway.as_deref())
            .unwrap_or(""),
    );
    let metric = entry(
        &group,
        "Metric",
        &route.as_ref().map(|r| r.metric).unwrap_or(100).to_string(),
    );
    rows.borrow_mut()
        .push((destination.clone(), gateway, metric));
    let remove = gtk::Button::with_label("Remove route");
    let list = rows.clone();
    let container = parent.downgrade();
    let target = group.downgrade();
    remove.connect_clicked(move |_| {
        list.borrow_mut().retain(|(d, _, _)| d != &destination);
        if let (Some(parent), Some(target)) = (container.upgrade(), target.upgrade()) {
            parent.remove(&target);
        }
    });
    group.append(&remove);
    parent.append(&group);
}
fn route_summary(interface: &str) -> String {
    let mut lines = Vec::new();
    for line in fs::read_to_string("/proc/net/route")
        .unwrap_or_default()
        .lines()
        .skip(1)
    {
        let p = line.split_whitespace().collect::<Vec<_>>();
        if p.len() < 8 || p[0] != interface {
            continue;
        }
        let ip = |s: &str| {
            u32::from_str_radix(s, 16)
                .ok()
                .map(|n| std::net::Ipv4Addr::from(n.to_le_bytes()))
        };
        if let (Some(destination), Some(gateway), Ok(mask)) =
            (ip(p[1]), ip(p[2]), u32::from_str_radix(p[7], 16))
        {
            lines.push(format!(
                "{}/{} via {} · metric {}",
                destination,
                mask.count_ones(),
                gateway,
                p[6]
            ));
        }
    }
    for line in fs::read_to_string("/proc/net/ipv6_route")
        .unwrap_or_default()
        .lines()
    {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() != 10 || fields[9] != interface {
            continue;
        }
        let address = |s: &str| {
            u128::from_str_radix(s, 16)
                .ok()
                .map(std::net::Ipv6Addr::from)
        };
        if let (Some(destination), Ok(prefix), Some(gateway), Ok(metric)) = (
            address(fields[0]),
            u8::from_str_radix(fields[1], 16),
            address(fields[4]),
            u32::from_str_radix(fields[5], 16),
        ) {
            lines.push(format!(
                "{destination}/{prefix} via {gateway} · metric {metric}"
            ));
        }
    }
    if lines.is_empty() {
        "No active routes for this interface.".into()
    } else {
        format!("Active routes:\n{}", lines.join("\n"))
    }
}
fn certificate(app: &Rc<App>, parent: &gtk::Box, label: &str) -> Rc<RefCell<String>> {
    let value = Rc::new(RefCell::new(String::new()));
    let button = gtk::Button::with_label("Choose file…");
    parent.append(&row(label, &button));
    let weak = Rc::downgrade(app);
    let data = value.clone();
    button.connect_clicked(move |button| {
        let weak = weak.clone();
        let data = data.clone();
        let target = button.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let chooser = gtk::FileDialog::builder()
                .title("Import certificate or private key")
                .build();
            if let Ok(file) = chooser.open_future(app.parent().as_ref()).await {
                if let Some(path) = file.path() {
                    let display = path
                        .file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let result = gio::spawn_blocking(move || {
                        let file = fs::File::open(path).map_err(|_| "Cannot read certificate")?;
                        let mut text = String::new();
                        file.take(128 * 1024 + 1)
                            .read_to_string(&mut text)
                            .map_err(|_| "Certificate must be PEM text")?;
                        if text.len() > 128 * 1024 {
                            return Err("Certificate file is too large");
                        }
                        Ok(text)
                    })
                    .await;
                    match result {
                        Ok(Ok(text)) => {
                            data.replace(text);
                            if let Some(button) = target.upgrade() {
                                button.set_label(&display);
                            }
                        }
                        _ => app.error(
                            "Cannot import certificate/key; use a PEM file smaller than 128 KiB",
                        ),
                    }
                }
            }
        });
    });
    value
}
fn enterprise_network(app: &Rc<App>) {
    let (window, parent) = dialog(app, "Enterprise Wi-Fi");
    parent.append(&description("Server verification is required. Obtain the CA certificate and server domain from your network administrator."));
    let ssid = entry(&parent, "Network name", "");
    let method = choose(
        &parent,
        "Authentication",
        &[
            ("PEAP", "PEAP / MSCHAPv2"),
            ("TTLS", "TTLS / PAP"),
            ("TLS", "TLS / certificate"),
        ],
        "PEAP",
    );
    let identity = entry(&parent, "Identity", "");
    let anonymous = entry(&parent, "Anonymous identity (optional)", "");
    let password = secret(&parent, "Password");
    let domain = entry(&parent, "Server domain", "");
    let ca = certificate(app, &parent, "CA certificate");
    let cert = certificate(app, &parent, "Client certificate (TLS)");
    let key = certificate(app, &parent, "Client key (TLS)");
    let key_password = secret(&parent, "Key password (optional)");
    parent.append(&button(app, "Save enterprise profile", move || {
        Action::System {
            operation: SystemAction::Enterprise {
                profile: Enterprise {
                    ssid: ssid.text().to_string(),
                    method: ["PEAP", "TTLS", "TLS"][method.selected().min(2) as usize].into(),
                    identity: identity.text().to_string(),
                    anonymous_identity: anonymous.text().to_string(),
                    password: password.text().to_string(),
                    server_domain: domain.text().to_string(),
                    ca_pem: ca.borrow().clone(),
                    client_certificate_pem: cert.borrow().clone(),
                    private_key_pem: key.borrow().clone(),
                    key_password: key_password.text().to_string(),
                },
            },
        }
    }));
    parent.append(&description("After saving, scan and connect using the connection list. ConnMan will manage the connection; iwd reads the protected profile."));
    window.present();
}
