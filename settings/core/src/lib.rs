//! Shared wire types and validation. This crate deliberately has no GTK dependency.
pub mod bus;
pub mod clock;
pub mod files;
pub mod layout;
pub mod model;
pub mod power;
pub mod validate;

pub use model::*;
pub type Result<T> = std::result::Result<T, String>;
pub const SESSION_NAME: &str = "org.uflinux.Settings1";
pub const SESSION_PATH: &str = "/org/uflinux/Settings";
pub const SYSTEM_NAME: &str = "org.uflinux.Settings.System1";
pub const SYSTEM_PATH: &str = "/org/uflinux/Settings/System";
pub const MAX_REQUEST: usize = 256 * 1024;
