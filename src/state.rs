//! Last applied settings. The mouse does not report the DPI of each stage
//! (the official software also keeps this on the PC), so we store it here.
//! The file is treated as untrusted input: size-limited and fully validated.

use crate::protocol::{self as p, ButtonMap, DpiColors, DpiConfig, LightingConfig, PowerConfig, SensorConfig};
use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::sync::OnceLock;

const MAX_FILE: u64 = 16 * 1024;

/// Under sudo, the user's home from the password database (ignoring $HOME/$XDG_CONFIG_HOME).
static HOME_OVERRIDE: OnceLock<Option<PathBuf>> = OnceLock::new();

pub fn set_home_override(home: Option<PathBuf>) {
    let _ = HOME_OVERRIDE.set(home);
}

#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct Saved {
    pub polling_hz: Option<u16>,
    pub dpi: Option<DpiConfig>,
    // Settings below are stored only after the mouse acknowledged them
    // (the mouse has no command to read them back).
    #[serde(default)]
    pub buttons: Option<ButtonMap>,
    #[serde(default)]
    pub dpi_colors: Option<DpiColors>,
    #[serde(default)]
    pub lift_off_mm: Option<u8>,
    #[serde(default)]
    pub lighting: Option<LightingConfig>,
    #[serde(default)]
    pub power: Option<PowerConfig>,
    #[serde(default)]
    pub sensor: Option<SensorConfig>,
    #[serde(default)]
    pub debounce_ms: Option<u8>,
}

fn config_base() -> Option<PathBuf> {
    if let Some(over) = HOME_OVERRIDE.get() {
        return over.as_ref().map(|h| h.join(".config"));
    }
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
}

fn path() -> Option<PathBuf> {
    Some(config_base()?.join("tmouseopen").join("state.json"))
}

/// Path used by the previous version (YDX Mouse); read only if the new one does not exist yet.
fn legacy_path() -> Option<PathBuf> {
    Some(config_base()?.join("ydx-mouse").join("state.json"))
}

impl Saved {
    pub fn load() -> Self {
        let Some(mut path) = path() else { return Self::default() };
        if fs::symlink_metadata(&path).is_err() {
            match legacy_path() {
                Some(old) => path = old,
                None => return Self::default(),
            }
        }
        let Ok(meta) = fs::symlink_metadata(&path) else { return Self::default() };
        if !meta.is_file() || meta.len() > MAX_FILE {
            return Self::default();
        }
        let Ok(text) = fs::read_to_string(&path) else { return Self::default() };
        let mut s: Saved = serde_json::from_str(&text).unwrap_or_default();
        if s.polling_hz.is_some_and(|hz| p::polling_code(hz).is_err()) {
            s.polling_hz = None;
        }
        if s.dpi.is_some_and(|d| d.validate().is_err()) {
            s.dpi = None;
        }
        if s.buttons.is_some_and(|b| b.validate().is_err()) {
            s.buttons = None;
        }
        if s.lift_off_mm.is_some_and(|v| p::validate_lift_off(v).is_err()) {
            s.lift_off_mm = None;
        }
        if s.lighting.is_some_and(|l| l.validate().is_err()) {
            s.lighting = None;
        }
        if s.power.is_some_and(|pw| pw.validate().is_err()) {
            s.power = None;
        }
        if s.debounce_ms.is_some_and(|v| p::validate_debounce(v).is_err()) {
            s.debounce_ms = None;
        }
        s
    }

    /// Writes atomically (temporary file + rename) with mode 0600.
    pub fn store(&self) {
        // Never write anything as root.
        // SAFETY: geteuid has no preconditions.
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let Some(path) = path() else { return };
        let Some(dir) = path.parent() else { return };
        if fs::create_dir_all(dir).is_err() {
            return;
        }
        let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
        let tmp = dir.join(format!(".state.{}.tmp", std::process::id()));
        let Ok(json) = serde_json::to_vec_pretty(self) else { return };
        let res = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&tmp)
            .and_then(|mut f| f.write_all(&json).and_then(|_| f.sync_all()));
        if res.is_ok() {
            let _ = fs::rename(&tmp, &path);
        } else {
            let _ = fs::remove_file(&tmp);
        }
    }
}
