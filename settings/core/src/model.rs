use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Snapshot {
    pub network: Network,
    pub audio: Audio,
    pub displays: Displays,
    pub appearance: Appearance,
    pub datetime: DateTime,
    #[serde(default)]
    pub power: Power,
    pub about: BTreeMap<String, String>,
    pub error: Option<String>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Network {
    pub available: bool,
    pub state: String,
    pub offline: bool,
    pub technologies: Vec<Technology>,
    pub services: Vec<Service>,
    pub authentication: Option<AuthRequest>,
    pub portal: Option<String>,
    pub wifi_hardware_blocked: bool,
    pub wifi_software_blocked: bool,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Technology {
    pub path: String,
    pub kind: String,
    pub powered: bool,
    pub connected: bool,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Service {
    pub path: String,
    pub name: String,
    pub kind: String,
    pub state: String,
    pub security: Vec<String>,
    pub strength: u8,
    pub favorite: bool,
    pub autoconnect: bool,
    pub interface: String,
    pub address: String,
    pub ipv4: BTreeMap<String, String>,
    pub ipv6: BTreeMap<String, String>,
    pub dns: Vec<String>,
    pub domains: Vec<String>,
    pub proxy: Proxy,
    pub config: ConnectionConfig,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct AuthRequest {
    pub id: u64,
    pub service: String,
    pub name: String,
    pub fields: Vec<AuthField>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct AuthField {
    pub name: String,
    pub secret: bool,
    pub required: bool,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Audio {
    pub available: bool,
    pub outputs: Vec<AudioDevice>,
    pub inputs: Vec<AudioDevice>,
    pub streams: Vec<AudioStream>,
    pub cards: Vec<AudioCard>,
    pub default_output: String,
    pub default_input: String,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct AudioDevice {
    pub index: u32,
    pub name: String,
    pub description: String,
    pub volume: f64,
    pub muted: bool,
    pub balance: f64,
    pub port: String,
    pub ports: Vec<(String, String)>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct AudioStream {
    pub index: u32,
    pub name: String,
    pub input: bool,
    pub volume: f64,
    pub muted: bool,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct AudioCard {
    pub index: u32,
    pub description: String,
    pub profile: String,
    pub profiles: Vec<(String, String)>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Displays {
    pub available: bool,
    pub outputs: Vec<Output>,
    pub brightness: Vec<Brightness>,
    pub pending: Option<u64>,
    pub hdr_requested: bool,
    pub renderer: String,
    pub hdr_status: String,
    pub brightness_status: String,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Output {
    pub name: String,
    pub make: String,
    pub model: String,
    pub serial: String,
    pub enabled: bool,
    pub x: i32,
    pub y: i32,
    pub scale: f64,
    pub transform: u32,
    pub adaptive_sync: bool,
    pub adaptive_sync_control: bool,
    pub current: Option<Mode>,
    pub modes: Vec<Mode>,
}
impl Output {
    pub fn identity(&self) -> String {
        if self.serial.is_empty() {
            format!("{}|{}|{}", self.make, self.model, self.name)
        } else {
            format!("{}|{}|{}", self.make, self.model, self.serial)
        }
    }
    pub fn config(&self) -> OutputConfig {
        OutputConfig {
            name: self.name.clone(),
            enabled: self.enabled,
            mode: self.current.clone(),
            x: self.x,
            y: self.y,
            scale: self.scale,
            transform: self.transform,
            adaptive_sync: self.adaptive_sync,
        }
    }
}
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mode {
    pub width: i32,
    pub height: i32,
    pub refresh: i32,
    pub preferred: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputConfig {
    pub name: String,
    pub enabled: bool,
    pub mode: Option<Mode>,
    pub x: i32,
    pub y: i32,
    pub scale: f64,
    pub transform: u32,
    pub adaptive_sync: bool,
}
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Brightness {
    pub id: String,
    pub name: String,
    pub percent: f64,
    pub ddc: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Appearance {
    pub wallpapers: Vec<Wallpaper>,
    pub color: String,
}
impl Default for Appearance {
    fn default() -> Self {
        Self {
            wallpapers: vec![Wallpaper {
                output: "*".into(),
                file: "/usr/share/backgrounds/uflinux.svg".into(),
                fit: "fill".into(),
            }],
            color: "#102c46".into(),
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Wallpaper {
    pub output: String,
    pub file: String,
    pub fit: String,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct DateTime {
    pub timezone: String,
    pub ntp_enabled: bool,
    pub ntp_running: bool,
    pub synchronized: Option<bool>,
    pub servers: Vec<String>,
    pub utc_seconds: i64,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionConfig {
    pub ipv4: BTreeMap<String, String>,
    pub ipv6: BTreeMap<String, String>,
    pub dns: Vec<String>,
    pub domains: Vec<String>,
    pub proxy: Proxy,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proxy {
    pub method: String,
    pub url: String,
    pub servers: Vec<String>,
    pub excludes: Vec<String>,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Route {
    pub destination: String,
    pub gateway: Option<String>,
    pub metric: u32,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Enterprise {
    pub ssid: String,
    pub method: String,
    pub identity: String,
    pub anonymous_identity: String,
    pub password: String,
    pub server_domain: String,
    pub ca_pem: String,
    pub client_certificate_pem: String,
    pub private_key_pem: String,
    pub key_password: String,
}

/// Exhaustive actions, not an arbitrary D-Bus proxy or command runner.
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Action {
    Refresh,
    SetPowerProfile {
        profile: PowerProfile,
    },
    Networking {
        enabled: bool,
    },
    Wifi {
        enabled: bool,
    },
    Scan,
    Connect {
        service: String,
    },
    Disconnect {
        service: String,
    },
    Forget {
        service: String,
    },
    ConnectHidden {
        ssid: String,
        passphrase: String,
    },
    AutoConnect {
        service: String,
        enabled: bool,
    },
    ConfigureConnection {
        service: String,
        config: ConnectionConfig,
    },
    AuthReply {
        id: u64,
        values: BTreeMap<String, String>,
    },
    AuthCancel {
        id: u64,
    },
    SetVolume {
        input: bool,
        device: String,
        percent: f64,
    },
    SetMute {
        input: bool,
        device: String,
        muted: bool,
    },
    SetDefault {
        input: bool,
        device: String,
    },
    SetBalance {
        input: bool,
        device: String,
        balance: f64,
    },
    SetPort {
        input: bool,
        device: String,
        port: String,
    },
    SetProfile {
        card: u32,
        profile: String,
    },
    SetStream {
        input: bool,
        index: u32,
        percent: f64,
        muted: bool,
    },
    ApplyDisplays {
        outputs: Vec<OutputConfig>,
        hdr: bool,
    },
    ConfirmDisplays {
        id: u64,
    },
    RevertDisplays,
    Brightness {
        id: String,
        percent: f64,
    },
    AdjustBrightness {
        delta: f64,
    },
    AdjustVolume {
        delta: f64,
    },
    ToggleMute,
    SetAppearance {
        appearance: Appearance,
    },
    System {
        operation: SystemAction,
    },
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "kebab-case", deny_unknown_fields)]
pub enum SystemAction {
    Status,
    PowerStatus,
    SetPowerProfile {
        profile: PowerProfile,
    },
    SetPowerButtons {
        on_ac: PowerButtonAction,
        on_battery: PowerButtonAction,
    },
    SetTimezone {
        timezone: String,
    },
    SetTime {
        utc_seconds: i64,
    },
    SetNtp {
        enabled: bool,
        servers: Vec<String>,
    },
    DdcDiscover,
    DdcBrightness {
        id: String,
        percent: f64,
    },
    GetRoutes {
        service: String,
    },
    SetRoutes {
        service: String,
        routes: Vec<Route>,
    },
    Enterprise {
        profile: Enterprise,
    },
    HiddenProfile {
        ssid: String,
        passphrase: String,
    },
    ForgetCredentials {
        ssid: String,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PowerProfile {
    Performance,
    #[default]
    Balanced,
    PowerSaver,
}
impl PowerProfile {
    pub fn name(self) -> &'static str {
        match self {
            Self::Performance => "performance",
            Self::Balanced => "balanced",
            Self::PowerSaver => "power-saver",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Performance => "Performance",
            Self::Balanced => "Balanced",
            Self::PowerSaver => "Power saver",
        }
    }
    pub fn code(self) -> &'static str {
        match self {
            Self::Performance => "PRF",
            Self::Balanced => "BAL",
            Self::PowerSaver => "SAV",
        }
    }
    pub fn parse(s: &str) -> crate::Result<Self> {
        match s {
            "performance" => Ok(Self::Performance),
            "balanced" => Ok(Self::Balanced),
            "power-saver" => Ok(Self::PowerSaver),
            _ => Err("Choose power-saver, balanced or performance".into()),
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PowerButtonAction {
    #[default]
    Poweroff,
    Suspend,
    Nothing,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PowerPolicy {
    pub profile: PowerProfile,
    pub on_ac: PowerButtonAction,
    pub on_battery: PowerButtonAction,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Power {
    pub available: bool,
    pub profile: Option<PowerProfile>,
    pub policy: PowerPolicy,
    pub on_battery: bool,
    pub button_handler_running: bool,
    pub can_suspend: bool,
}
