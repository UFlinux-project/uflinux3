//! Only exact routes recorded by UFLinux, tagged with protocol 242, are removed.
use glib::variant::ObjectPath;
use settings_core::{
    bus::{self, Properties},
    *,
};
use std::{
    collections::BTreeMap,
    ffi::CString,
    net::IpAddr,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    sync::Mutex,
};

const DATABASE: &str = "/var/lib/uflinux-settings/routes.json";
// Serialize persistence and rtnetlink updates, including signal-triggered restore.
static ROUTE_LOCK: Mutex<()> = Mutex::new(());
pub type Routes = BTreeMap<String, Vec<Route>>;
pub fn configured() -> bool {
    !files::read_json::<Routes>(std::path::Path::new(DATABASE)).is_empty()
}
pub fn list(service: &str) -> Result<Vec<Route>> {
    validate::service(service)?;
    Ok(files::read_json::<Routes>(std::path::Path::new(DATABASE))
        .remove(service)
        .unwrap_or_default())
}
async fn active_interfaces(bus: &gio::DBusConnection) -> Result<BTreeMap<String, String>> {
    let reply = bus::call(
        bus,
        "net.connman",
        "/",
        "net.connman.Manager",
        "GetServices",
        None,
    )
    .await?;
    let list = reply
        .get::<(Vec<(ObjectPath, Properties)>,)>()
        .ok_or("Invalid ConnMan service list")?
        .0;
    Ok(list
        .into_iter()
        .filter(|(_, p)| ["ready", "online"].contains(&bus::text(p, "State").as_str()))
        .map(|(path, p)| {
            (
                path.to_string(),
                bus::text(&bus::dict(&p, "Ethernet"), "Interface"),
            )
        })
        .collect())
}
pub async fn set(bus: &gio::DBusConnection, service: String, routes: Vec<Route>) -> Result<()> {
    validate::service(&service)?;
    if routes.len() > 32 {
        return Err("At most 32 additional routes per connection are supported".into());
    }
    for r in &routes {
        validate::route(r)?;
    }
    let interfaces = active_interfaces(bus).await?;
    gio::spawn_blocking(move || {
        let _lock = ROUTE_LOCK.lock().map_err(|_| "Route worker unavailable")?;
        let mut all = files::read_json::<Routes>(std::path::Path::new(DATABASE));
        let before = all.get(&service).cloned().unwrap_or_default();
        let interface = interfaces.get(&service);
        if let Some(interface) = interface {
            replace(interface, &before, &routes)?;
        }
        if routes.is_empty() {
            all.remove(&service);
        } else {
            all.insert(service, routes.clone());
        }
        if let Err(error) = files::save_json(std::path::Path::new(DATABASE), &all, 0o600) {
            if let Some(interface) = interface {
                replace(interface, &routes, &before).map_err(|rollback| {
                    format!("Cannot save routes: {error}; rollback failed: {rollback}")
                })?;
            }
            return Err(error);
        }
        Ok(())
    })
    .await
    .map_err(|_| "Route worker failed")?
}
pub async fn restore(bus: gio::DBusConnection) -> Result<()> {
    let interfaces = active_interfaces(&bus).await?;
    gio::spawn_blocking(move || {
        let _lock = ROUTE_LOCK.lock().map_err(|_| "Route worker unavailable")?;
        let all = files::read_json::<Routes>(std::path::Path::new(DATABASE));
        for (service, routes) in all {
            if let Some(interface) = interfaces.get(&service) {
                for route in routes {
                    if let Err(error) = apply(interface, &route, true) {
                        if error != "An equivalent route already exists" {
                            return Err(error);
                        }
                    }
                }
            }
        }
        Ok::<_, String>(())
    })
    .await
    .map_err(|_| "Route worker failed")?
}
fn replace(interface: &str, before: &[Route], after: &[Route]) -> Result<()> {
    transaction(before, after, |route, add| apply(interface, route, add))
}
fn transaction(
    before: &[Route],
    after: &[Route],
    mut change: impl FnMut(&Route, bool) -> Result<()>,
) -> Result<()> {
    let mut added = Vec::new();
    let mut removed = Vec::new();
    for (route, add) in after
        .iter()
        .filter(|r| !before.contains(r))
        .map(|r| (r, true))
        .chain(
            before
                .iter()
                .filter(|r| !after.contains(r))
                .map(|r| (r, false)),
        )
    {
        if let Err(error) = change(route, add) {
            let mut failures = Vec::new();
            for r in removed.iter().rev() {
                if let Err(e) = change(r, true) {
                    failures.push(e);
                }
            }
            for r in added.iter().rev() {
                if let Err(e) = change(r, false) {
                    failures.push(e);
                }
            }
            return if failures.is_empty() {
                Err(error)
            } else {
                Err(format!(
                    "{error}; route rollback failed: {}",
                    failures.join("; ")
                ))
            };
        }
        if add {
            added.push(route.clone());
        } else {
            removed.push(route.clone());
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_delete_restores_previous_routes() {
        let r = |destination: &str| Route {
            destination: destination.into(),
            gateway: None,
            metric: 12,
        };
        let old = vec![r("192.0.2.0/24"), r("198.51.100.0/24")];
        let new = vec![r("203.0.113.0/24")];
        let mut actual = old.clone();
        let failure = old[1].clone();
        assert!(
            transaction(&old, &new, |route, add| {
                if !add && *route == failure {
                    return Err("Simulated authorization loss".into());
                }
                if add {
                    actual.push(route.clone());
                } else {
                    actual.retain(|r| r != route);
                }
                Ok(())
            })
            .is_err()
        );
        assert_eq!(actual.len(), old.len());
        for route in old {
            assert!(actual.contains(&route));
        }
    }
}
fn attribute(packet: &mut Vec<u8>, kind: u16, value: &[u8]) {
    packet.extend_from_slice(&((value.len() + 4) as u16).to_ne_bytes());
    packet.extend_from_slice(&kind.to_ne_bytes());
    packet.extend_from_slice(value);
    while packet.len() % 4 != 0 {
        packet.push(0);
    }
}
fn apply(interface: &str, route: &Route, add: bool) -> Result<()> {
    validate::route(route)?;
    let c = CString::new(interface).map_err(|_| "Invalid network interface")?;
    let index = unsafe { libc::if_nametoindex(c.as_ptr()) };
    if index == 0 {
        return Err("Network interface disappeared".into());
    }
    let (destination, prefix) = route.destination.split_once('/').ok_or("Invalid route")?;
    let destination = destination.parse::<IpAddr>().map_err(|_| "Invalid route")?;
    let mut packet = vec![0u8; 16];
    // rtmsg: family, dst prefix, src prefix, tos, table, protocol, scope, type, flags.
    packet.extend_from_slice(&[
        if destination.is_ipv4() {
            libc::AF_INET as u8
        } else {
            libc::AF_INET6 as u8
        },
        prefix.parse().map_err(|_| "Invalid prefix")?,
        0,
        0,
        254,
        242,
        if route.gateway.is_some() { 0 } else { 253 },
        1,
        0,
        0,
        0,
        0,
    ]);
    let bytes = |ip: IpAddr| match ip {
        IpAddr::V4(v) => v.octets().to_vec(),
        IpAddr::V6(v) => v.octets().to_vec(),
    };
    attribute(&mut packet, 1, &bytes(destination));
    attribute(&mut packet, 4, &index.to_ne_bytes());
    attribute(&mut packet, 6, &route.metric.to_ne_bytes());
    if let Some(gateway) = &route.gateway {
        attribute(
            &mut packet,
            5,
            &bytes(gateway.parse().map_err(|_| "Invalid gateway")?),
        );
    }
    let length = packet.len() as u32;
    packet[0..4].copy_from_slice(&length.to_ne_bytes());
    packet[4..6].copy_from_slice(&(if add { 24u16 } else { 25u16 }).to_ne_bytes());
    packet[6..8].copy_from_slice(&(1u16 | 4 | if add { 0x400 | 0x200 } else { 0 }).to_ne_bytes()); // REQUEST|ACK|CREATE|EXCL
    packet[8..12].copy_from_slice(&1u32.to_ne_bytes());
    let fd = unsafe {
        libc::socket(
            libc::AF_NETLINK,
            libc::SOCK_RAW | libc::SOCK_CLOEXEC,
            libc::NETLINK_ROUTE,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    let mut address: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
    address.nl_family = libc::AF_NETLINK as u16;
    let timeout = libc::timeval {
        tv_sec: 3,
        tv_usec: 0,
    };
    unsafe {
        libc::setsockopt(
            fd.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_RCVTIMEO,
            (&timeout as *const libc::timeval).cast(),
            std::mem::size_of_val(&timeout) as u32,
        );
    }
    let sent = unsafe {
        libc::sendto(
            fd.as_raw_fd(),
            packet.as_ptr().cast(),
            packet.len(),
            0,
            (&address as *const libc::sockaddr_nl).cast(),
            std::mem::size_of_val(&address) as u32,
        )
    };
    if sent < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let mut response = [0u8; 8192];
    let count = unsafe {
        libc::recv(
            fd.as_raw_fd(),
            response.as_mut_ptr().cast(),
            response.len(),
            0,
        )
    };
    if count < 20 {
        return Err("No route acknowledgement received".into());
    }
    let error = i32::from_ne_bytes(response[16..20].try_into().unwrap());
    if error == 0 || (!add && [-libc::ESRCH, -libc::ENOENT].contains(&error)) {
        return Ok(());
    }
    if add && error == -libc::EEXIST {
        // Never replace an existing route belonging to another owner.
        return Err("An equivalent route already exists".into());
    }
    Err(std::io::Error::from_raw_os_error(-error).to_string())
}
