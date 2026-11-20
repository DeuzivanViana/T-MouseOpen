//! Access to /dev/hidrawN.
//!
//! Safeguards:
//! - discovery via sysfs only (nothing is opened until a candidate matches VID:PID + interface 1);
//! - only `/dev/hidraw<N>` paths are accepted;
//! - after opening, the file descriptor itself is checked (fstat + HIDIOCGRAWINFO + HIDIOCGRDESCSIZE)
//!   to confirm it is the right receiver and the 252-byte vendor interface. This prevents the node
//!   from being swapped between the check and the open (TOCTOU) and talking to another device;
//! - O_CLOEXEC, fixed-size buffers and reads with a timeout.

use crate::protocol::{self as p, REPORT_LEN};
use std::fs;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};

/// What the worker needs from a device. Implemented by the real hidraw and by demo mode.
pub trait Backend: Send {
    fn name(&self) -> String;
    fn set_feature(&mut self, frame: &[u8; REPORT_LEN]) -> io::Result<()>;
    fn get_feature(&mut self, report_id: u8) -> io::Result<[u8; REPORT_LEN]>;
    /// Waits up to `timeout_ms` for an input report. Ok(None) = nothing arrived.
    fn read_input(&mut self, timeout_ms: i32) -> io::Result<Option<Vec<u8>>>;
}

// ------------------------------------------------------------------ discovery

#[derive(Debug, Clone)]
pub struct Candidate {
    pub node: PathBuf,
    pub interface: u8,
    pub rdesc_len: Option<usize>,
}

fn read_small(path: &Path) -> Option<String> {
    let s = fs::read_to_string(path).ok()?;
    (s.len() < 4096).then(|| s.trim().to_string())
}

fn parse_hid_id(uevent: &str) -> Option<(u16, u16)> {
    let line = uevent.lines().find(|l| l.starts_with("HID_ID="))?;
    let mut it = line["HID_ID=".len()..].split(':');
    let _bus = it.next()?;
    let vid = u32::from_str_radix(it.next()?, 16).ok()?;
    let pid = u32::from_str_radix(it.next()?, 16).ok()?;
    Some((vid as u16, pid as u16))
}

fn valid_node_name(name: &str) -> bool {
    name.strip_prefix("hidraw").is_some_and(|n| !n.is_empty() && n.len() <= 4 && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Lists the receiver's hidraw nodes (both interfaces). Does not open any device.
pub fn candidates() -> Vec<Candidate> {
    let mut out = Vec::new();
    let Ok(dir) = fs::read_dir("/sys/class/hidraw") else { return out };
    for e in dir.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !valid_node_name(&name) {
            continue;
        }
        let Ok(dev) = fs::canonicalize(e.path().join("device")) else { continue };
        let Some((vid, pid)) = read_small(&dev.join("uevent")).as_deref().and_then(parse_hid_id) else { continue };
        if (vid, pid) != (p::VID, p::PID) {
            continue;
        }
        let Some(iface) = dev.parent()
            .and_then(|d| read_small(&d.join("bInterfaceNumber")))
            .and_then(|s| u8::from_str_radix(&s, 16).ok()) else { continue };
        let rdesc_len = fs::read(dev.join("report_descriptor")).ok().map(|v| v.len());
        out.push(Candidate { node: PathBuf::from("/dev").join(&name), interface: iface, rdesc_len });
    }
    out.sort_by(|a, b| a.node.cmp(&b.node));
    out
}

pub fn control_candidates() -> Vec<Candidate> {
    candidates().into_iter().filter(|c| c.interface == p::CONTROL_INTERFACE).collect()
}

// ------------------------------------------------------------------ ioctl

const fn ioc(dir: u64, nr: u64, size: u64) -> u64 {
    (dir << 30) | (size << 16) | ((b'H' as u64) << 8) | nr
}
const IOC_READ: u64 = 2;
const IOC_RW: u64 = 3;
const HIDIOCGRDESCSIZE: u64 = ioc(IOC_READ, 0x01, 4);
const HIDIOCGRAWINFO: u64 = ioc(IOC_READ, 0x03, 8);
const HIDIOCSFEATURE_32: u64 = ioc(IOC_RW, 0x06, REPORT_LEN as u64);
const HIDIOCGFEATURE_32: u64 = ioc(IOC_RW, 0x07, REPORT_LEN as u64);

#[repr(C)]
#[derive(Default)]
struct HidrawDevinfo {
    bustype: u32,
    vendor: i16,
    product: i16,
}

fn check(ret: libc::c_int) -> io::Result<libc::c_int> {
    if ret < 0 { Err(io::Error::last_os_error()) } else { Ok(ret) }
}

// ------------------------------------------------------------------ real hidraw

pub struct Hidraw {
    fd: OwnedFd,
    node: PathBuf,
}

#[derive(Debug)]
pub enum OpenError {
    BadPath,
    Permission(PathBuf),
    NotCharDevice,
    WrongDevice(String),
    Io(io::Error),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadPath => write!(f, "invalid path (expected /dev/hidrawN)"),
            Self::Permission(p) => write!(f, "no permission on {} — install the udev rule (see README)", p.display()),
            Self::NotCharDevice => write!(f, "the node is not a character device"),
            Self::WrongDevice(s) => write!(f, "unexpected device: {s}"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl Hidraw {
    pub fn open(node: &Path) -> Result<Self, OpenError> {
        let name = node.strip_prefix("/dev").ok().and_then(|n| n.to_str()).ok_or(OpenError::BadPath)?;
        if !valid_node_name(name) {
            return Err(OpenError::BadPath);
        }
        let c = std::ffi::CString::new(node.as_os_str().as_encoded_bytes()).map_err(|_| OpenError::BadPath)?;
        // SAFETY: the path is a valid CString; plain flags.
        let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC | libc::O_NOCTTY) };
        if fd < 0 {
            let e = io::Error::last_os_error();
            return Err(if e.kind() == io::ErrorKind::PermissionDenied {
                OpenError::Permission(node.to_path_buf())
            } else {
                OpenError::Io(e)
            });
        }
        // SAFETY: fd was just opened and is now exclusively owned by the OwnedFd.
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };

        // Is it really a character device?
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        // SAFETY: st is a valid buffer for fstat.
        check(unsafe { libc::fstat(fd.as_raw_fd(), &mut st) }).map_err(OpenError::Io)?;
        if st.st_mode & libc::S_IFMT != libc::S_IFCHR {
            return Err(OpenError::NotCharDevice);
        }
        // VID:PID checked on the file descriptor itself.
        let mut info = HidrawDevinfo::default();
        // SAFETY: the ioctl writes exactly sizeof(HidrawDevinfo) bytes.
        check(unsafe { libc::ioctl(fd.as_raw_fd(), HIDIOCGRAWINFO as _, &mut info) }).map_err(OpenError::Io)?;
        let (vid, pid) = (info.vendor as u16, info.product as u16);
        if (vid, pid) != (p::VID, p::PID) {
            return Err(OpenError::WrongDevice(format!("{vid:04x}:{pid:04x}")));
        }
        // Vendor interface (252-byte descriptor), not the regular mouse interface.
        let mut size: libc::c_int = 0;
        // SAFETY: the ioctl writes one int.
        check(unsafe { libc::ioctl(fd.as_raw_fd(), HIDIOCGRDESCSIZE as _, &mut size) }).map_err(OpenError::Io)?;
        if size as u32 != p::EXPECTED_RDESC_SIZE {
            return Err(OpenError::WrongDevice(format!("report descriptor is {size} bytes (expected {})", p::EXPECTED_RDESC_SIZE)));
        }
        Ok(Self { fd, node: node.to_path_buf() })
    }
}

impl Backend for Hidraw {
    fn name(&self) -> String {
        self.node.display().to_string()
    }

    fn set_feature(&mut self, frame: &[u8; REPORT_LEN]) -> io::Result<()> {
        if frame[0] != p::CMD_REPORT_ID {
            // Defense in depth: this program only ever writes the command report.
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "report not allowed"));
        }
        let mut buf = *frame;
        // SAFETY: buf is exactly REPORT_LEN bytes, the size encoded in the ioctl.
        check(unsafe { libc::ioctl(self.fd.as_raw_fd(), HIDIOCSFEATURE_32 as _, buf.as_mut_ptr()) })?;
        Ok(())
    }

    fn get_feature(&mut self, report_id: u8) -> io::Result<[u8; REPORT_LEN]> {
        if report_id != p::CMD_REPORT_ID && report_id != p::INFO_REPORT_ID {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "report not allowed"));
        }
        let mut buf = [0u8; REPORT_LEN];
        buf[0] = report_id;
        // SAFETY: same as above.
        check(unsafe { libc::ioctl(self.fd.as_raw_fd(), HIDIOCGFEATURE_32 as _, buf.as_mut_ptr()) })?;
        Ok(buf)
    }

    fn read_input(&mut self, timeout_ms: i32) -> io::Result<Option<Vec<u8>>> {
        let mut pfd = libc::pollfd { fd: self.fd.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        // SAFETY: one valid pollfd.
        let n = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
        if n < 0 {
            let e = io::Error::last_os_error();
            return if e.kind() == io::ErrorKind::Interrupted { Ok(None) } else { Err(e) };
        }
        if n == 0 {
            return Ok(None);
        }
        if pfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "receiver disconnected"));
        }
        let mut buf = [0u8; 64];
        // SAFETY: buf is 64 bytes; read never writes more than that.
        let r = unsafe { libc::read(self.fd.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
        if r < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Some(buf[..r as usize].to_vec()))
    }
}

// ------------------------------------------------------------------ demo mode

/// Simulated device (TMOUSEOPEN_DEMO=1) to preview the UI without the mouse.
pub struct Demo {
    status: [u8; 10],
    pending: Option<std::time::Instant>,
    last_periodic: std::time::Instant,
}

impl Demo {
    pub fn new() -> Self {
        Self {
            status: [0x04, 0x01, 0x01, 0x02, 0x1e, 0x00, 0x00, 0x00, 0x02, 0x11],
            pending: Some(std::time::Instant::now()),
            last_periodic: std::time::Instant::now(),
        }
    }
}

impl Backend for Demo {
    fn name(&self) -> String {
        "demo (simulated)".into()
    }
    fn set_feature(&mut self, f: &[u8; REPORT_LEN]) -> io::Result<()> {
        match f[1] {
            0x04 => self.status[3] = f[5],
            0x02 => self.status[2] = f[5],
            _ => {}
        }
        self.pending = Some(std::time::Instant::now());
        Ok(())
    }
    fn get_feature(&mut self, id: u8) -> io::Result<[u8; REPORT_LEN]> {
        let mut b = [0u8; REPORT_LEN];
        b[0] = id;
        if id == p::INFO_REPORT_ID {
            b[1..7].copy_from_slice(b"1010\x01\x01");
        } else {
            b[1] = 1;
        }
        Ok(b)
    }
    fn read_input(&mut self, timeout_ms: i32) -> io::Result<Option<Vec<u8>>> {
        let now = std::time::Instant::now();
        if self.pending.take().is_some() || now.duration_since(self.last_periodic).as_secs() >= 5 {
            self.last_periodic = now;
            let mut v = self.status.to_vec();
            v.resize(32, 0);
            return Ok(Some(v));
        }
        std::thread::sleep(std::time::Duration::from_millis(timeout_ms.clamp(0, 250) as u64));
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn node_names() {
        assert!(valid_node_name("hidraw3"));
        assert!(!valid_node_name("hidraw"));
        assert!(!valid_node_name("hidraw3/../../etc"));
        assert!(!valid_node_name("sda"));
    }
    #[test]
    fn hid_id() {
        assert_eq!(parse_hid_id("DRIVER=x\nHID_ID=0003:000004F3:0000026F\n"), Some((0x04f3, 0x026f)));
        assert_eq!(parse_hid_id("HID_ID=garbage"), None);
    }
    #[test]
    fn ioctl_numbers() {
        assert_eq!(HIDIOCSFEATURE_32, 0xC020_4806);
        assert_eq!(HIDIOCGFEATURE_32, 0xC020_4807);
        assert_eq!(HIDIOCGRAWINFO, 0x8008_4803);
        assert_eq!(HIDIOCGRDESCSIZE, 0x8004_4801);
    }
}
