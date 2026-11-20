//! HID protocol of the 04F3:026F receiver (Titorion YDX-GM009).
//! Pure code (no I/O), tested against frames captured from the official software.
//! See PROTOCOL.md for the origin and confidence level of each field.

pub const VID: u16 = 0x04F3;
pub const PID: u16 = 0x026F;
pub const CONTROL_INTERFACE: u8 = 1;
pub const EXPECTED_RDESC_SIZE: u32 = 252;
pub const REPORT_LEN: usize = 32;

pub const CMD_REPORT_ID: u8 = 0x06;
pub const STATUS_REPORT_ID: u8 = 0x04;
pub const INFO_REPORT_ID: u8 = 0x05;

pub const DPI_STAGES: usize = 7;

/// First sequence number. The official software initializes its counter to 1 (`0x40a51c`);
/// the firmware ignores a frame whose sequence byte is 0, so 0 is never used.
pub const SEQ_START: u8 = 1;

fn next_seq(seq: u8) -> u8 {
    match seq.wrapping_add(1) {
        0 => SEQ_START,
        n => n,
    }
}
pub const PAW3311: u8 = 0x11;
pub const PAW3311_DPI_MIN: u16 = 100;
pub const PAW3311_DPI_MAX: u16 = 12000;
pub const PAW3311_DPI_STEP: u16 = 100;

/// Supported rates and their codes (the same code appears in status byte 3).
pub const POLLING: [(u16, u8); 4] = [(125, 0x08), (250, 0x04), (500, 0x02), (1000, 0x01)];
/// Rates already tested on the user's hardware.
pub const POLLING_TESTED: [u16; 2] = [250, 500];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtoError {
    InvalidPolling(u16),
    InvalidDpi(u16),
    NoStageEnabled,
    ActiveStageDisabled,
    ActiveStageOutOfRange,
}

impl std::fmt::Display for ProtoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPolling(hz) => write!(f, "unsupported polling rate: {hz} Hz"),
            Self::InvalidDpi(d) => write!(f, "invalid DPI for the PAW3311: {d} (100–12000, step 100)"),
            Self::NoStageEnabled => write!(f, "enable at least one DPI stage"),
            Self::ActiveStageDisabled => write!(f, "the active stage must be enabled"),
            Self::ActiveStageOutOfRange => write!(f, "active stage out of range 1–7"),
        }
    }
}
impl std::error::Error for ProtoError {}

pub fn polling_code(hz: u16) -> Result<u8, ProtoError> {
    POLLING.iter().find(|(h, _)| *h == hz).map(|(_, c)| *c).ok_or(ProtoError::InvalidPolling(hz))
}

pub fn polling_hz(code: u8) -> Option<u16> {
    POLLING.iter().find(|(_, c)| *c == code).map(|(h, _)| *h)
}

pub fn paw3311_dpi_code(dpi: u16) -> Result<u16, ProtoError> {
    if !(PAW3311_DPI_MIN..=PAW3311_DPI_MAX).contains(&dpi) || !dpi.is_multiple_of(PAW3311_DPI_STEP) {
        return Err(ProtoError::InvalidDpi(dpi));
    }
    Ok(dpi / 100 - 1)
}

/// The only commands this program can build. By construction there is no way
/// to build any other report (such as the firmware ones, 0xBC/0xBF).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Polling(u16),
    Dpi(DpiConfig),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DpiConfig {
    /// DPI of each stage (same value for the X and Y axes).
    pub stages: [u16; DPI_STAGES],
    /// Bit i = stage i enabled. The mouse DPI button only cycles through these stages.
    pub enabled_mask: u8,
    /// Active stage, zero-based.
    pub active: u8,
}

impl Default for DpiConfig {
    fn default() -> Self {
        Self { stages: [400, 800, 1600, 2400, 3200, 6400, 12000], enabled_mask: 0x7F, active: 1 }
    }
}

impl DpiConfig {
    pub fn validate(&self) -> Result<(), ProtoError> {
        for d in self.stages {
            paw3311_dpi_code(d)?;
        }
        if self.enabled_mask & 0x7F == 0 {
            return Err(ProtoError::NoStageEnabled);
        }
        if self.active as usize >= DPI_STAGES {
            return Err(ProtoError::ActiveStageOutOfRange);
        }
        if self.enabled_mask & (1 << self.active) == 0 {
            return Err(ProtoError::ActiveStageDisabled);
        }
        Ok(())
    }
}

pub fn checksum(buf: &[u8; REPORT_LEN]) -> u8 {
    buf.iter().fold(0u8, |a, b| a.wrapping_add(*b))
}

/// Report 0x06: [0]=0x06 [1]=cmd [2]=seq [3]=(part<<4)|total [4]=checksum [5..]=data.
fn frame(cmd: u8, seq: u8, data: &[u8], part: u8, parts: u8) -> [u8; REPORT_LEN] {
    debug_assert!(data.len() <= REPORT_LEN - 5);
    let mut b = [0u8; REPORT_LEN];
    b[0] = CMD_REPORT_ID;
    b[1] = cmd;
    b[2] = seq;
    b[3] = ((part & 0x0F) << 4) | (parts & 0x0F);
    b[5..5 + data.len()].copy_from_slice(data);
    b[4] = checksum(&b);
    b
}

/// Builds the frames for a command. `seq` is the counter; each frame consumes one value.
pub fn build(cmd: &Command, seq: &mut u8) -> Result<Vec<[u8; REPORT_LEN]>, ProtoError> {
    let mut next = || {
        if *seq == 0 {
            *seq = SEQ_START;
        }
        let s = *seq;
        *seq = next_seq(s);
        s
    };
    match cmd {
        Command::Polling(hz) => {
            let code = polling_code(*hz)?;
            Ok(vec![frame(0x04, next(), &[code], 0, 1)])
        }
        Command::Dpi(cfg) => {
            cfg.validate()?;
            // 18 bytes: [active][mask][7 x LE code][00 00]. Part 0 = X axis, part 1 = Y axis.
            let mut block = [0u8; 18];
            block[0] = cfg.active;
            block[1] = cfg.enabled_mask & 0x7F;
            for (i, d) in cfg.stages.iter().enumerate() {
                let c = paw3311_dpi_code(*d)?;
                block[2 + 2 * i] = (c & 0xFF) as u8;
                block[3 + 2 * i] = (c >> 8) as u8;
            }
            Ok(vec![frame(0x02, next(), &block, 0, 2), frame(0x02, next(), &block, 1, 2)])
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Status {
    pub raw: [u8; 10],
}

impl Status {
    /// Accepts only 0x04 input reports with at least 10 bytes.
    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < 10 || data[0] != STATUS_REPORT_ID {
            return None;
        }
        let mut raw = [0u8; 10];
        raw.copy_from_slice(&data[..10]);
        Some(Self { raw })
    }
    pub fn dpi_stage(&self) -> u8 { self.raw[2] }
    pub fn polling_code(&self) -> u8 { self.raw[3] }
    pub fn polling_hz(&self) -> Option<u16> { polling_hz(self.raw[3]) }
    pub fn battery(&self) -> u8 { if self.raw[5] == 2 { 100 } else { self.raw[4].min(100) } }
    pub fn sensor_code(&self) -> u8 { self.raw[9] }

    /// 0 = on battery, 1 = charging, 2 = fully charged.
    pub fn charge_state(&self) -> u8 { self.raw[5] }
    pub fn charge_text(&self) -> &'static str {
        match self.raw[5] {
            0 => "on battery",
            1 => "charging",
            2 => "fully charged",
            _ => "unknown",
        }
    }
    pub fn max_polling(&self) -> Option<u16> {
        match self.raw[8] {
            0x01 | 0x02 => Some(1000),
            0x04 => Some(2000),
            0x08 => Some(4000),
            _ => None,
        }
    }
    pub fn sensor_name(&self) -> String {
        let n = match self.raw[9] {
            0x04 => "PAW3104", 0x11 => "PAW3311", 0x25 => "PMW3325", 0x27 => "PAW3327",
            0x12 => "PMW3212", 0x35 => "PAW3335", 0x60 => "PMW3360", 0x70 => "PAW3370",
            0x89 => "PMW3389", 0x95 => "PAW3395", 0x50 => "PAW3950", 0x99 => "PAW3399",
            c => return format!("unknown (0x{c:02x})"),
        };
        n.to_string()
    }
}

/// Report 0x05: bytes 1..5 as ASCII (likely the firmware version, "1010").
pub fn parse_version(raw: &[u8; REPORT_LEN]) -> String {
    raw[1..5].iter().map(|&b| if (0x20..0x7f).contains(&b) { b as char } else { '.' }).collect()
}

pub fn hex(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polling_frames_match_captures() {
        // Real frames from the official software (Wireshark).
        let mut seq = 0x09;
        let f = build(&Command::Polling(250), &mut seq).unwrap();
        assert_eq!(&f[0][..6], &[0x06, 0x04, 0x09, 0x01, 0x18, 0x04]);
        assert!(f[0][6..].iter().all(|&b| b == 0));
        let f = build(&Command::Polling(500), &mut seq).unwrap();
        assert_eq!(&f[0][..6], &[0x06, 0x04, 0x0a, 0x01, 0x17, 0x02]);
        assert_eq!(seq, 0x0b);
    }

    #[test]
    fn dpi_frames_layout() {
        let cfg = DpiConfig { stages: [400, 800, 1600, 2400, 3200, 6400, 12000], enabled_mask: 0x0F, active: 1 };
        let mut seq = 1;
        let f = build(&Command::Dpi(cfg), &mut seq).unwrap();
        assert_eq!(f.len(), 2);
        // Same bytes the Python version sent and the user validated on the hardware.
        assert_eq!(hex(&f[0][..23]), "06 02 01 02 20 01 0f 03 00 07 00 0f 00 17 00 1f 00 3f 00 77 00 00 00");
        assert_eq!(hex(&f[1][..23]), "06 02 02 12 31 01 0f 03 00 07 00 0f 00 17 00 1f 00 3f 00 77 00 00 00");
    }

    #[test]
    fn rejects_bad_input() {
        let mut seq = SEQ_START;
        assert!(build(&Command::Polling(333), &mut seq).is_err());
        for d in [0, 50, 150, 12100, u16::MAX] {
            assert!(paw3311_dpi_code(d).is_err(), "{d}");
        }
        let c = DpiConfig { enabled_mask: 0, ..Default::default() };
        assert_eq!(c.validate(), Err(ProtoError::NoStageEnabled));
        let c = DpiConfig { enabled_mask: 0b1, active: 3, ..Default::default() };
        assert_eq!(c.validate(), Err(ProtoError::ActiveStageDisabled));
        assert_eq!(seq, SEQ_START, "invalid commands must not consume a sequence number");
    }

    #[test]
    fn sequence_never_zero() {
        // Starts at 1 like the official software and skips 0 when it wraps.
        let mut seq = 0;
        let f = build(&Command::Polling(500), &mut seq).unwrap();
        assert_eq!(f[0][2], 1);
        let mut seq = 254;
        let f = build(&Command::Dpi(DpiConfig::default()), &mut seq).unwrap();
        assert_eq!((f[0][2], f[1][2]), (254, 255));
        let f = build(&Command::Polling(500), &mut seq).unwrap();
        assert_eq!(f[0][2], 1, "wraps to 1, never 0");
    }

    #[test]
    fn status_decode() {
        let s = Status::parse(&[0x04, 0x01, 0x03, 0x02, 0x1e, 0x00, 0x00, 0x00, 0x02, 0x11, 0, 0]).unwrap();
        assert_eq!(s.polling_hz(), Some(500));
        assert_eq!(s.battery(), 30);
        assert_eq!(s.dpi_stage(), 3);
        assert_eq!(s.sensor_name(), "PAW3311");
        assert_eq!(s.max_polling(), Some(1000));
        assert!(Status::parse(&[0x05; 32]).is_none());
        assert!(Status::parse(&[0x04, 1, 2]).is_none());
        let full = Status::parse(&[0x04, 1, 0, 2, 0x28, 2, 0, 0, 2, 0x11]).unwrap();
        assert_eq!(full.battery(), 100);
    }
}
