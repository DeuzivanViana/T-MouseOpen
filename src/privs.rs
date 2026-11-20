//! Running under `sudo -E`.
//!
//! When started as root, TMouseOpen does exactly three things with root privileges:
//!   1. reads sysfs to find the receiver,
//!   2. opens the receiver's control node (`/dev/hidrawN`, verified as in `device::Hidraw::open`),
//!   3. permanently drops to the invoking user (SUDO_UID/SUDO_GID).
//!
//! Only then is any window, file or config touched. Nothing is ever written as root, and after
//! the drop the process cannot regain root (all uids/gids are reset and NO_NEW_PRIVS is set).
//!
//! Without root (udev rule installed), none of this runs.

use crate::device::{self, Backend, Hidraw};
use std::ffi::CStr;
use std::path::PathBuf;

pub enum Mode {
    /// Regular user (udev rule). Reconnecting after unplug works.
    User,
    /// Started via sudo; privileges were dropped to this user.
    Dropped { home: Option<PathBuf> },
}

pub struct Prepared {
    pub mode: Mode,
    /// Device opened while still root (sudo mode only).
    pub device: Option<Box<dyn Backend>>,
    /// Message to show in the UI log.
    pub note: Option<String>,
}

fn env_id(name: &str) -> Option<u32> {
    let v = std::env::var(name).ok()?;
    if v.is_empty() || v.len() > 10 || !v.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    v.parse().ok()
}

/// Home directory of `uid` from the password database (not from $HOME, which sudo may change).
fn home_of(uid: u32) -> Option<PathBuf> {
    let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
    let mut out: *mut libc::passwd = std::ptr::null_mut();
    let mut buf = vec![0 as libc::c_char; 16 * 1024];
    // SAFETY: valid buffers; getpwuid_r is the thread-safe variant.
    let rc = unsafe { libc::getpwuid_r(uid, &mut pw, buf.as_mut_ptr(), buf.len(), &mut out) };
    if rc != 0 || out.is_null() || pw.pw_dir.is_null() {
        return None;
    }
    // SAFETY: pw_dir points into `buf`, NUL-terminated by libc.
    let dir = unsafe { CStr::from_ptr(pw.pw_dir) }.to_str().ok()?;
    let p = PathBuf::from(dir);
    p.is_absolute().then_some(p)
}

fn drop_to(uid: u32, gid: u32) -> Result<(), String> {
    // SAFETY: plain syscalls with integer arguments; order matters (groups, gid, then uid).
    unsafe {
        if libc::setgroups(0, std::ptr::null()) != 0 {
            return Err(format!("setgroups failed: {}", std::io::Error::last_os_error()));
        }
        if libc::setresgid(gid, gid, gid) != 0 {
            return Err(format!("setresgid failed: {}", std::io::Error::last_os_error()));
        }
        if libc::setresuid(uid, uid, uid) != 0 {
            return Err(format!("setresuid failed: {}", std::io::Error::last_os_error()));
        }
        // Must not be able to get root back.
        if libc::setuid(0) == 0 || libc::geteuid() == 0 || libc::getegid() == 0 && gid != 0 {
            return Err("privileges could not be dropped".into());
        }
        libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0);
        libc::umask(0o077);
    }
    Ok(())
}

/// Call first thing in `main`, before any window or file is created.
pub fn prepare(demo: bool) -> Result<Prepared, String> {
    // SAFETY: geteuid has no preconditions.
    if unsafe { libc::geteuid() } != 0 {
        return Ok(Prepared { mode: Mode::User, device: None, note: None });
    }
    let (Some(uid), Some(gid)) = (env_id("SUDO_UID"), env_id("SUDO_GID")) else {
        return Err("Running as root is only supported through sudo (sudo -E tmouseopen). \
                    Or install the udev rule and run it as your user."
            .into());
    };
    if uid == 0 {
        return Err("sudo was invoked by root itself; run it from your regular user account.".into());
    }

    // 1-2. Still root: find and open the receiver. Nothing else.
    let mut device: Option<Box<dyn Backend>> = None;
    let mut note = None;
    if !demo {
        for c in device::control_candidates() {
            match Hidraw::open(&c.node) {
                Ok(h) => {
                    device = Some(Box::new(h));
                    break;
                }
                Err(e) => note = Some(format!("could not open {}: {e}", c.node.display())),
            }
        }
        if device.is_none() && note.is_none() {
            note = Some("receiver not found while starting under sudo".into());
        }
    }
    let home = home_of(uid);

    // 3. Drop for good.
    drop_to(uid, gid)?;

    // Point HOME and XDG dirs at the user's own home so no library (GL, fonts) tries root paths.
    // Still single-threaded here, so changing the environment is sound.
    if let Some(h) = &home {
        std::env::set_var("HOME", h);
        for var in ["XDG_CONFIG_HOME", "XDG_CACHE_HOME", "XDG_DATA_HOME", "XDG_STATE_HOME"] {
            let inside = std::env::var_os(var).map(PathBuf::from).is_some_and(|p| p.is_absolute() && p.starts_with(h));
            if !inside {
                std::env::remove_var(var);
            }
        }
    }

    let msg = format!("started via sudo: receiver opened, privileges dropped to uid {uid}");
    Ok(Prepared {
        mode: Mode::Dropped { home },
        device,
        note: Some(match note {
            Some(n) => format!("{msg}; {n}"),
            None => msg,
        }),
    })
}
