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
    #[serde(default)]
    pub screen_capture: ScreenCapture,
    /// Probed from the capture backend at startup; runtime-only and never persisted.
    #[serde(default)]
    pub capture_capabilities: CaptureCapabilities,
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
                file: "/usr/share/backgrounds/uflinux/Dark_Ivy.jpg".into(),
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
/// User preferences for uflinux-screencapture. The capture tool reads this section
/// over D-Bus and never writes it, so the tool itself stays daemonless.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScreenCapture {
    pub image_format: CaptureFormat,
    /// Empty means "$XDG_PICTURES_DIR/Screenshots".
    pub screenshot_dir: String,
    pub copy_to_clipboard: bool,
    pub flash: bool,
    pub delay_seconds: u8,
    pub include_cursor: bool,
    pub target: CaptureTarget,
    pub advanced_view: bool,
    pub quality: CaptureQuality,
    /// Only used when quality is ConstantBitrate, mirroring the backend frontend.
    pub bitrate_kbps: u32,
    pub fps: u8,
    pub audio_source: AudioSource,
    /// None means "let the backend choose".
    pub video_codec: Option<String>,
    pub container: Option<String>,
    pub audio_codec: Option<AudioCodec>,
    pub framerate_mode: FramerateMode,
    pub color_range: ColorRange,
}

/// What the installed gpu-screen-recorder actually supports. The settings page
/// filters its option lists with this so a user cannot choose an unavailable codec.
#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureCapabilities {
    pub backend_available: bool,
    pub image_formats: Vec<String>,
    pub video_codecs: Vec<String>,
    pub containers: Vec<String>,
}
impl CaptureCapabilities {
    /// True when the backend can write this image format (or is unknown, in which
    /// case the UI keeps every option available rather than hiding the default).
    pub fn supports_image(&self, id: &str) -> bool {
        !self.backend_available || self.image_formats.iter().any(|f| f == id)
    }
    /// Known codecs the backend reports it can encode, in the fixed display order.
    pub fn video_codec_list(&self) -> Vec<String> {
        self.filter(&VIDEO_CODECS, &self.video_codecs)
    }
    /// Known containers the backend reports it can write, in the fixed display order.
    pub fn container_list(&self) -> Vec<String> {
        self.filter(&CONTAINERS, &self.containers)
    }
    fn filter(&self, known: &[&str], reported: &[String]) -> Vec<String> {
        if !self.backend_available {
            return known.iter().map(|k| (*k).to_string()).collect();
        }
        known
            .iter()
            .filter(|k| reported.iter().any(|r| r == *k))
            .map(|k| (*k).to_string())
            .collect()
    }
}

impl Default for ScreenCapture {
    fn default() -> Self {
        Self {
            image_format: CaptureFormat::default(),
            screenshot_dir: String::new(),
            copy_to_clipboard: true,
            flash: true,
            delay_seconds: 0,
            include_cursor: true,
            target: CaptureTarget::default(),
            advanced_view: false,
            quality: CaptureQuality::default(),
            bitrate_kbps: DEFAULT_BITRATE_KBPS,
            fps: DEFAULT_FPS,
            audio_source: AudioSource::default(),
            video_codec: None,
            container: None,
            audio_codec: None,
            framerate_mode: FramerateMode::default(),
            color_range: ColorRange::default(),
        }
    }
}

pub const DEFAULT_FPS: u8 = 60;
pub const DEFAULT_BITRATE_KBPS: u32 = 15000;
pub const MAX_BITRATE_KBPS: u32 = 500_000;
pub const MAX_DELAY_SECONDS: u8 = 30;
pub const MAX_FPS: u8 = 240;

/// Video codec names accepted by the backend's -k option.
pub const VIDEO_CODECS: [&str; 19] = [
    "auto",
    "h264",
    "hevc",
    "av1",
    "vp8",
    "vp9",
    "hevc_hdr",
    "av1_hdr",
    "hevc_10bit",
    "av1_10bit",
    "h264_vulkan",
    "hevc_vulkan",
    "hevc_10bit_vulkan",
    "av1_vulkan",
    "av1_hdr_vulkan",
    "av1_10bit_vulkan",
    "h264_v4l2",
    "hevc_v4l2",
    "h264_software",
];

/// Container names accepted by the backend's -c option.
pub const CONTAINERS: [&str; 5] = ["mp4", "mkv", "flv", "webm", "mp3"];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CaptureFormat {
    Png,
    Jpeg,
    #[default]
    Webp,
}
impl CaptureFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Webp => "webp",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Png => "PNG",
            Self::Jpeg => "JPEG",
            Self::Webp => "WebP",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CaptureTarget {
    #[default]
    Screen,
    Window,
    Area,
}
impl CaptureTarget {
    pub fn label(self) -> &'static str {
        match self {
            Self::Screen => "Entire screen",
            Self::Window => "Window",
            Self::Area => "Selection",
        }
    }
}

/// Mirrors the quality presets of the backend's own GTK frontend.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CaptureQuality {
    Medium,
    High,
    #[default]
    VeryHigh,
    Ultra,
    ConstantBitrate,
}
impl CaptureQuality {
    pub fn id(self) -> &'static str {
        match self {
            Self::Medium => "medium",
            Self::High => "high",
            Self::VeryHigh => "very_high",
            Self::Ultra => "ultra",
            Self::ConstantBitrate => "cbr",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Medium => "Medium",
            Self::High => "High",
            Self::VeryHigh => "Very high",
            Self::Ultra => "Ultra",
            Self::ConstantBitrate => "Constant bitrate",
        }
    }
    pub fn hint(self) -> &'static str {
        match self {
            Self::VeryHigh => "Recommended for recording",
            Self::ConstantBitrate => "Recommended for live streaming",
            _ => "",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AudioSource {
    #[default]
    None,
    Output,
    Input,
    OutputAndInput,
}
impl AudioSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "No audio",
            Self::Output => "System output",
            Self::Input => "Microphone",
            Self::OutputAndInput => "System output and microphone",
        }
    }
    /// The value for the backend's -a option, or None when nothing is captured.
    pub fn backend(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Output => Some("default_output"),
            Self::Input => Some("default_input"),
            Self::OutputAndInput => Some("default_output|default_input"),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FramerateMode {
    #[default]
    Auto,
    Constant,
    Variable,
    Content,
}
impl FramerateMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Automatic",
            Self::Constant => "Constant",
            Self::Variable => "Variable",
            Self::Content => "Sync to content",
        }
    }
    /// The value for the backend's -fm option; Auto leaves the flag out entirely.
    pub fn backend(self) -> Option<&'static str> {
        match self {
            Self::Auto => None,
            Self::Constant => Some("cfr"),
            Self::Variable => Some("vfr"),
            Self::Content => Some("content"),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ColorRange {
    #[default]
    Limited,
    Full,
}
impl ColorRange {
    pub fn label(self) -> &'static str {
        match self {
            Self::Limited => "Limited",
            Self::Full => "Full",
        }
    }
    /// Limited is the backend default, so only Full produces a flag.
    pub fn backend(self) -> Option<&'static str> {
        match self {
            Self::Limited => None,
            Self::Full => Some("full"),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AudioCodec {
    #[default]
    Opus,
    Aac,
}
impl AudioCodec {
    pub fn id(self) -> &'static str {
        match self {
            Self::Opus => "opus",
            Self::Aac => "aac",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Opus => "Opus",
            Self::Aac => "AAC",
        }
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct DateTime {
    pub timezone: String,
    pub time_sync_enabled: bool,
    pub sntp_running: bool,
    pub synchronized: Option<bool>,
    pub timeservers: Vec<String>,
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
    SetScreenCapture {
        screen_capture: ScreenCapture,
    },
    ResetScreenCapture,
    ScreenCaptureCapabilities,
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
    SetTimeSync {
        enabled: bool,
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
    #[serde(default)]
    pub batteries: Vec<Battery>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Battery {
    pub name: String,
    pub percent: Option<u8>,
    pub status: String,
    pub energy_full: Option<u64>,
}
