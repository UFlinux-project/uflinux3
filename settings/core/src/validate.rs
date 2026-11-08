use crate::*;
use std::net::IpAddr;

pub fn percent(n: f64) -> Result<()> {
    if !n.is_finite() || !(0.0..=100.0).contains(&n) {
        Err("Choose a percentage from 0 to 100".into())
    } else {
        Ok(())
    }
}
pub fn service(path: &str) -> Result<()> {
    let id = path
        .strip_prefix("/net/connman/service/")
        .ok_or("Invalid connection")?;
    if id.is_empty()
        || id.len() > 512
        || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
    {
        return Err("Invalid connection".into());
    }
    Ok(())
}
pub fn hostname(s: &str) -> bool {
    s.len() <= 253
        && !s.is_empty()
        && s.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        })
}
pub fn server(s: &str) -> bool {
    s.parse::<IpAddr>().is_ok() || hostname(s)
}
pub fn connection(c: &ConnectionConfig) -> Result<()> {
    for (v, ipv6) in [(&c.ipv4, false), (&c.ipv6, true)] {
        for key in v.keys() {
            if ![
                "Method",
                "Address",
                "Netmask",
                "PrefixLength",
                "Gateway",
                "Privacy",
            ]
            .contains(&key.as_str())
            {
                return Err("Unsupported address property".into());
            }
        }
        let method =
            v.get("Method")
                .map(String::as_str)
                .unwrap_or(if ipv6 { "auto" } else { "dhcp" });
        let methods: &[&str] = if ipv6 {
            &["auto", "manual", "off"]
        } else {
            &["dhcp", "manual", "off"]
        };
        if !methods.contains(&method) {
            return Err("Unsupported address method".into());
        }
        for key in ["Address", "Gateway"] {
            if let Some(value) = v.get(key).filter(|s| !s.is_empty()) {
                let ip = value
                    .parse::<IpAddr>()
                    .map_err(|_| format!("Invalid {key}"))?;
                if ip.is_ipv6() != ipv6 {
                    return Err("Address family does not match".into());
                }
            }
        }
        if method == "manual" {
            if !v.contains_key("Address") {
                return Err("A manual address is required".into());
            }
            if ipv6 {
                if v.get("PrefixLength")
                    .and_then(|s| s.parse::<u8>().ok())
                    .filter(|n| *n <= 128)
                    .is_none()
                {
                    return Err("Choose an IPv6 prefix between 0 and 128".into());
                }
            } else {
                let mask = v
                    .get("Netmask")
                    .ok_or("A netmask is required")?
                    .parse::<std::net::Ipv4Addr>()
                    .map_err(|_| "Invalid netmask")?;
                let bits = !u32::from(mask);
                if bits & bits.wrapping_add(1) != 0 {
                    return Err("Netmask must be contiguous".into());
                }
            }
        }
    }
    if c.dns.len() > 16 || c.dns.iter().any(|s| s.parse::<IpAddr>().is_err()) {
        return Err("DNS servers must be IP addresses".into());
    }
    if c.domains.len() > 16 || c.domains.iter().any(|s| !hostname(s)) {
        return Err("Invalid search domain; use ASCII/punycode names".into());
    }
    if !["", "direct", "auto", "manual"].contains(&c.proxy.method.as_str()) {
        return Err("Invalid proxy method".into());
    }
    if c.proxy.method == "auto" && !http_url(&c.proxy.url) {
        return Err("Proxy configuration must use an HTTP or HTTPS URL".into());
    }
    if c.proxy.servers.len() > 16
        || c.proxy.servers.iter().any(|s| {
            s.len() > 512
                || s.contains(['\n', '\r', '\0', '@'])
                || s.chars().any(char::is_whitespace)
        })
    {
        return Err("Invalid proxy server; embedded credentials are not supported".into());
    }
    Ok(())
}
pub fn http_url(s: &str) -> bool {
    s.len() <= 2048
        && s.starts_with(
            ["http://", "https://"]
                .into_iter()
                .find(|p| s.starts_with(p))
                .unwrap_or("\0"),
        )
        && !s.contains(['\n', '\r', '\0'])
        && !s.chars().any(char::is_whitespace)
}
pub fn route(r: &Route) -> Result<()> {
    let (address, prefix) = r
        .destination
        .split_once('/')
        .ok_or("Use a destination in CIDR notation")?;
    let address = address
        .parse::<IpAddr>()
        .map_err(|_| "Invalid route destination")?;
    let prefix: u8 = prefix.parse().map_err(|_| "Invalid route prefix")?;
    if prefix > if address.is_ipv4() { 32 } else { 128 } {
        return Err("Invalid route prefix".into());
    }
    if let Some(gateway) = &r.gateway {
        let gateway = gateway
            .parse::<IpAddr>()
            .map_err(|_| "Invalid route gateway")?;
        if gateway.is_ipv4() != address.is_ipv4() {
            return Err("Route and gateway address families differ".into());
        }
    }
    Ok(())
}
pub fn displays(configs: &[OutputConfig], current: &[Output]) -> Result<()> {
    if configs.len() != current.len() || configs.len() > 32 {
        return Err("Display list changed; refresh and try again".into());
    }
    let mut names = std::collections::BTreeSet::new();
    if !configs.iter().any(|o| o.enabled) {
        return Err("At least one display must remain enabled".into());
    }
    for c in configs {
        if !names.insert(&c.name) {
            return Err("Duplicate display".into());
        }
        let o = current
            .iter()
            .find(|o| o.name == c.name)
            .ok_or("Unknown display")?;
        if !c.scale.is_finite()
            || !(0.5..=4.0).contains(&c.scale)
            || c.transform > 7
            || c.x.unsigned_abs() > 32768
            || c.y.unsigned_abs() > 32768
        {
            return Err("Invalid display scale, orientation or position".into());
        }
        if c.enabled
            && !c.mode.as_ref().is_some_and(|m| {
                o.modes
                    .iter()
                    .any(|x| x.width == m.width && x.height == m.height && x.refresh == m.refresh)
            })
        {
            return Err("Choose an advertised display mode".into());
        }
        if c.adaptive_sync && !o.adaptive_sync_control {
            return Err("VRR control is unavailable on this compositor".into());
        }
    }
    Ok(())
}
pub fn enterprise(p: &Enterprise) -> Result<()> {
    if p.ssid.is_empty() || p.ssid.len() > 32 || p.ssid.contains(['\0', '\n', '\r']) {
        return Err("SSID must contain 1–32 bytes".into());
    }
    if !["TLS", "PEAP", "TTLS"].contains(&p.method.as_str()) {
        return Err("Unsupported enterprise method".into());
    }
    if !hostname(&p.server_domain)
        || !p.ca_pem.contains("-----BEGIN CERTIFICATE-----")
        || p.ca_pem.len() > 128 * 1024
    {
        return Err("A CA certificate and authentication-server domain are required".into());
    }
    for value in [
        &p.identity,
        &p.anonymous_identity,
        &p.password,
        &p.key_password,
    ] {
        if value.len() > 1024 || value.contains(['\n', '\r', '\0']) {
            return Err("Invalid enterprise credential field".into());
        }
    }
    if p.identity.is_empty() {
        return Err("An identity is required".into());
    }
    if p.method == "TLS"
        && (!p
            .client_certificate_pem
            .contains("-----BEGIN CERTIFICATE-----")
            || !p.private_key_pem.contains("PRIVATE KEY-----"))
    {
        return Err("EAP-TLS requires a client certificate and private key".into());
    }
    if p.method != "TLS" && p.password.is_empty() {
        return Err("A password is required".into());
    }
    if p.client_certificate_pem.len() > 128 * 1024 || p.private_key_pem.len() > 64 * 1024 {
        return Err("Certificate/key is too large".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    #[test]
    fn rejects_nonfinite_and_out_of_range() {
        for n in [f64::NAN, f64::INFINITY, -1., 101.] {
            assert!(percent(n).is_err());
        }
    }
    #[test]
    fn connections_are_opaque_paths() {
        assert!(service("/net/connman/service/wifi_abc_managed_psk").is_ok());
        for s in [
            "/",
            "/net/connman/service/../../etc",
            "/net/connman/service/",
            "$(id)",
        ] {
            assert!(service(s).is_err());
        }
    }
    #[test]
    fn dns_and_masks_are_validated() {
        let mut c = ConnectionConfig::default();
        c.dns = vec!["example.com".into()];
        assert!(connection(&c).is_err());
        c.dns.clear();
        c.ipv4 = BTreeMap::from([
            ("Method".into(), "manual".into()),
            ("Address".into(), "192.0.2.5".into()),
            ("Netmask".into(), "255.0.255.0".into()),
        ]);
        assert!(connection(&c).is_err());
    }
    #[test]
    fn routes_cannot_mix_families() {
        assert!(
            route(&Route {
                destination: "192.0.2.0/24".into(),
                gateway: Some("::1".into()),
                metric: 1
            })
            .is_err()
        );
    }
    #[test]
    fn unknown_root_operations_rejected() {
        assert!(
            serde_json::from_str::<SystemAction>(r#"{"operation":"run","command":"id"}"#).is_err()
        );
    }
    #[test]
    fn hdr_preserves_other_configuration() {
        let s = "<labwc_config><!--keep--><core><gap>3</gap><hdr>no</hdr></core><keyboard/></labwc_config>";
        let x = files::set_hdr_xml(s, true).unwrap();
        assert!(x.contains("<!--keep-->"));
        assert!(x.contains("<gap>3</gap><hdr>yes</hdr>"));
        assert!(
            files::set_hdr_xml("<labwc_config><keyboard/></labwc_config>", true)
                .unwrap()
                .contains("<core><hdr>yes</hdr></core>")
        );
    }
    #[test]
    fn refusing_a_black_screen() {
        assert!(displays(&[], &[]).is_err());
    }
    #[test]
    fn hdr_empty_elements_and_attributes_preserve_other_sections() {
        for source in [
            "<labwc_config/>",
            "<labwc_config><core/></labwc_config>",
            "<labwc_config><core gap=\"4\" hdr=\"no\"/><other><hdr>untouched</hdr></other></labwc_config>",
        ] {
            let changed = files::set_hdr_xml(source, true).unwrap();
            assert_eq!(files::hdr_requested(&changed), Some(true));
            if source.contains("untouched") {
                assert!(changed.contains("<other><hdr>untouched</hdr></other>"));
                assert!(changed.contains("gap=\"4\""));
            }
        }
        assert!(files::set_hdr_xml("<wrong/>", true).is_err());
    }
    #[test]
    fn enterprise_requires_server_validation() {
        let p = Enterprise {
            ssid: "wifi".into(),
            method: "PEAP".into(),
            identity: "user".into(),
            anonymous_identity: String::new(),
            password: "password".into(),
            server_domain: String::new(),
            ca_pem: String::new(),
            client_certificate_pem: String::new(),
            private_key_pem: String::new(),
            key_password: String::new(),
        };
        assert!(enterprise(&p).is_err());
    }
}
