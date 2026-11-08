use crate::{Result, SESSION_NAME, SESSION_PATH};
use glib::variant::ToVariant;
use std::collections::HashMap;

pub const SESSION_XML: &str = r#"<node><interface name="org.uflinux.Settings1">
<method name="GetState"><arg type="s" direction="out"/></method>
<method name="Execute"><arg type="s" direction="in"/><arg type="s" direction="out"/></method>
<signal name="Changed"><arg type="s"/></signal>
</interface></node>"#;
pub const SYSTEM_XML: &str = r#"<node><interface name="org.uflinux.Settings.System1">
<method name="Execute"><arg type="s" direction="in"/><arg type="s" direction="out"/></method>
</interface></node>"#;

pub async fn call(
    connection: &gio::DBusConnection,
    name: &str,
    path: &str,
    interface: &str,
    method: &str,
    parameters: Option<&glib::Variant>,
) -> Result<glib::Variant> {
    connection
        .call_future(
            Some(name),
            path,
            interface,
            method,
            parameters,
            None,
            gio::DBusCallFlags::NONE,
            120_000,
        )
        .await
        .map_err(|e| e.message().to_string())
}
pub async fn session(method: &str, request: Option<&str>) -> Result<String> {
    let bus = gio::bus_get_future(gio::BusType::Session)
        .await
        .map_err(|e| e.to_string())?;
    let args = request.map(|s| (s,).to_variant());
    let reply = call(
        &bus,
        SESSION_NAME,
        SESSION_PATH,
        SESSION_NAME,
        method,
        args.as_ref(),
    )
    .await?;
    reply
        .get::<(String,)>()
        .map(|v| v.0)
        .ok_or_else(|| "Invalid Settings reply".into())
}
pub async fn execute(action: &crate::Action) -> Result<serde_json::Value> {
    let raw = serde_json::to_string(action).map_err(|e| e.to_string())?;
    let result = session("Execute", Some(&raw)).await?;
    serde_json::from_str(&result).map_err(|e| e.to_string())
}
pub type Properties = HashMap<String, glib::Variant>;
pub fn properties(v: &glib::Variant) -> Properties {
    v.get().unwrap_or_default()
}
pub fn text(p: &Properties, key: &str) -> String {
    p.get(key).and_then(|v| v.get()).unwrap_or_default()
}
pub fn boolean(p: &Properties, key: &str) -> bool {
    p.get(key).and_then(|v| v.get()).unwrap_or(false)
}
pub fn strings(p: &Properties, key: &str) -> Vec<String> {
    p.get(key).and_then(|v| v.get()).unwrap_or_default()
}
pub fn dict(p: &Properties, key: &str) -> Properties {
    p.get(key).map(properties).unwrap_or_default()
}
pub fn text_dict(p: &Properties, key: &str) -> std::collections::BTreeMap<String, String> {
    dict(p, key)
        .into_iter()
        .filter_map(|(k, v)| {
            v.get::<String>()
                .or_else(|| v.get::<u8>().map(|n| n.to_string()))
                .map(|s| (k, s))
        })
        .collect()
}
pub fn request_name(connection: &gio::DBusConnection, name: &str) -> Result<()> {
    let v = connection
        .call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "RequestName",
            Some(&(name, 4u32).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            5000,
            gio::Cancellable::NONE,
        )
        .map_err(|e| e.to_string())?;
    if v.get::<(u32,)>() != Some((1,)) {
        return Err("Settings service is already running".into());
    }
    Ok(())
}
