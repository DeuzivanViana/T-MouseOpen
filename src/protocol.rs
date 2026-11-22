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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtoError {
    InvalidPolling(u16),
    InvalidDpi(u16),
    NoStageEnabled,
    ActiveStageDisabled,
    ActiveStageOutOfRange,
    NoLeftClick,
    InvalidLiftOff(u8),
    InvalidLighting,
    InvalidSleep(u8),
    InvalidDebounce(u8),
}

impl std::fmt::Display for ProtoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPolling(hz) => write!(f, "unsupported polling rate: {hz} Hz"),
            Self::InvalidDpi(d) => write!(f, "invalid DPI for the PAW3311: {d} (100–12000, step 100)"),
            Self::NoStageEnabled => write!(f, "enable at least one DPI stage"),
            Self::ActiveStageDisabled => write!(f, "the active stage must be enabled"),
            Self::ActiveStageOutOfRange => write!(f, "active stage out of range 1–7"),
            Self::NoLeftClick => write!(f, "keep at least one button as left click"),
            Self::InvalidLiftOff(v) => write!(f, "invalid lift-off value: {v} (1 or 2 mm)"),
            Self::InvalidLighting => write!(f, "invalid lighting settings"),
            Self::InvalidSleep(v) => write!(f, "invalid sleep time: {v} (1–{SLEEP_MAX_MIN} min)"),
            Self::InvalidDebounce(v) => write!(f, "invalid debounce: {v} ms (0–{DEBOUNCE_MAX_MS} ms)"),
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
/// Every value is checked against the range the official software (GamingMS v2.1.5)
/// itself offers before a frame is built; see PROTOCOL.md for where each one comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// 0x04 [HW]
    Polling(u16),
    /// 0x02 [HW]
    Dpi(DpiConfig),
    /// 0x01 [RE + HW] button assignment
    Buttons(ButtonMap),
    /// 0x03 [RE + HW] LED color of each DPI stage
    DpiColors(DpiColors),
    /// 0x05 [RE + HW] lift-off distance in mm (1 or 2)
    LiftOff(u8),
    /// 0x06 [RE + HW] lighting effect
    Lighting(LightingConfig),
    /// 0x07 [RE + HW] sleep timer and "LED off while moving"
    Power(PowerConfig),
    /// 0x08 [RE + HW] angle snap, motion sync, ripple control
    Sensor(SensorConfig),
    /// 0x09 [RE + HW] button debounce in ms
    Debounce(u8),
    /// 0x0F [RE + HW] restore the mouse's factory settings
    FactoryReset,
}

/// Command codes this program may ever write (checked again in the device layer).
pub const ALLOWED_COMMANDS: [u8; 10] = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0F];

impl Command {
    pub fn code(&self) -> u8 {
        match self {
            Self::Buttons(_) => 0x01,
            Self::Dpi(_) => 0x02,
            Self::DpiColors(_) => 0x03,
            Self::Polling(_) => 0x04,
            Self::LiftOff(_) => 0x05,
            Self::Lighting(_) => 0x06,
            Self::Power(_) => 0x07,
            Self::Sensor(_) => 0x08,
            Self::Debounce(_) => 0x09,
            Self::FactoryReset => 0x0F,
        }
    }

    /// Short name for logs and messages.
    pub fn label(&self) -> String {
        match self {
            Self::Polling(hz) => format!("polling {hz} Hz"),
            Self::Dpi(_) => "DPI".into(),
            Self::Buttons(_) => "button assignment".into(),
            Self::DpiColors(_) => "DPI stage colors".into(),
            Self::LiftOff(mm) => format!("lift-off {mm} mm"),
            Self::Lighting(_) => "lighting".into(),
            Self::Power(_) => "power settings".into(),
            Self::Sensor(_) => "sensor settings".into(),
            Self::Debounce(ms) => format!("debounce {ms} ms"),
            Self::FactoryReset => "factory reset".into(),
        }
    }
}

// ------------------------------------------------------------------ buttons (0x01)

/// Button functions whose codes are confirmed by the official profiles and by the
/// default key table embedded in the firmware image. Nothing else can be assigned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ButtonAction {
    Left,
    Right,
    Middle,
    Back,
    Forward,
    DpiLoop,
}

impl ButtonAction {
    pub const ALL: [ButtonAction; 6] = [Self::Left, Self::Right, Self::Middle, Self::Back, Self::Forward, Self::DpiLoop];
    pub fn code(self) -> [u8; 3] {
        match self {
            Self::Left => [0x10, 0x01, 0x00],
            Self::Right => [0x10, 0x02, 0x00],
            Self::Middle => [0x10, 0x04, 0x00],
            Self::Back => [0x10, 0x08, 0x00],
            Self::Forward => [0x10, 0x10, 0x00],
            Self::DpiLoop => [0x40, 0x01, 0x00],
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Left => "Left click",
            Self::Right => "Right click",
            Self::Middle => "Middle click",
            Self::Back => "Back",
            Self::Forward => "Forward",
            Self::DpiLoop => "DPI loop",
        }
    }
}

/// Physical buttons, in the order the firmware stores them.
pub const BUTTON_NAMES: [&str; 6] = ["Left button", "Right button", "Middle button", "Back (side)", "Forward (side)", "DPI button"];

/// Function of each physical button, in firmware order (see `BUTTON_NAMES`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ButtonMap {
    pub buttons: [ButtonAction; 6],
}

impl Default for ButtonMap {
    fn default() -> Self {
        use ButtonAction::*;
        Self { buttons: [Left, Right, Middle, Back, Forward, DpiLoop] }
    }
}

impl ButtonMap {
    pub fn validate(&self) -> Result<(), ProtoError> {
        // Same rule as the official software ("Please have at least one L Key!"):
        // without a left click the mouse could not even click "Restore".
        if !self.buttons.contains(&ButtonAction::Left) {
            return Err(ProtoError::NoLeftClick);
        }
        Ok(())
    }
}

/// 7th key slot the official software always sends (its built-in default, `0x00436d30`).
const BUTTON_SLOT7: [u8; 3] = [0x40, 0x03, 0x00];

// ------------------------------------------------------------------ DPI colors (0x03)

/// RGB color of the LED for each DPI stage (8 slots, like the official software).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DpiColors {
    pub colors: [[u8; 3]; 8],
}

impl Default for DpiColors {
    fn default() -> Self {
        // Official defaults (DPILevelIndicationN in default0.psf).
        Self {
            colors: [
                [0xFF, 0x00, 0x00],
                [0x00, 0xFF, 0x00],
                [0x00, 0x00, 0xFF],
                [0x00, 0xFF, 0xFF],
                [0xFF, 0xFF, 0x00],
                [0xFF, 0x00, 0xFF],
                [0xFF, 0xFF, 0xFF],
                [0xFF, 0xFF, 0xFF],
            ],
        }
    }
}

// ------------------------------------------------------------------ lighting (0x06)

/// Effect codes, from the status-report handler of the official software (`0x00422050`).
pub const LIGHTING_MODES: [(u8, &str); 8] = [
    (0, "Off"),
    (1, "Colorful streaming"),
    (2, "Breathing"),
    (3, "Steady"),
    (4, "Neon"),
    (5, "Snake"),
    (6, "Colorful steady"),
    (7, "Wave"),
];
/// The official sliders have 3 speed and 4 brightness positions.
pub const LIGHTING_SPEED_MAX: u8 = 2;
pub const LIGHTING_BRIGHTNESS_MAX: u8 = 3;

pub fn lighting_mode_name(code: u8) -> Option<&'static str> {
    LIGHTING_MODES.iter().find(|(c, _)| *c == code).map(|(_, n)| *n)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LightingConfig {
    pub mode: u8,
    /// 0..=2
    pub speed: u8,
    /// 0..=3
    pub brightness: u8,
    /// 0 or 1
    pub direction: u8,
}

impl Default for LightingConfig {
    fn default() -> Self {
        // Official default profile: mode 1, speed 1, brightness 1.
        Self { mode: 1, speed: 1, brightness: 1, direction: 0 }
    }
}

impl LightingConfig {
    pub fn validate(&self) -> Result<(), ProtoError> {
        if lighting_mode_name(self.mode).is_none()
            || self.speed > LIGHTING_SPEED_MAX
            || self.brightness > LIGHTING_BRIGHTNESS_MAX
            || self.direction > 1
        {
            return Err(ProtoError::InvalidLighting);
        }
        Ok(())
    }
}

// ------------------------------------------------------------------ power (0x07)

/// The official power control has 8 positions, labeled "power down time: N minutes".
pub const SLEEP_MAX_MIN: u8 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PowerConfig {
    /// 1..=8
    pub sleep_minutes: u8,
    pub led_off_while_moving: bool,
}

impl Default for PowerConfig {
    fn default() -> Self {
        // Official defaults: PowerDownTime=1, MoveLightingOnOffMode=0.
        Self { sleep_minutes: 1, led_off_while_moving: false }
    }
}

impl PowerConfig {
    pub fn validate(&self) -> Result<(), ProtoError> {
        if !(1..=SLEEP_MAX_MIN).contains(&self.sleep_minutes) {
            return Err(ProtoError::InvalidSleep(self.sleep_minutes));
        }
        Ok(())
    }
}

// ------------------------------------------------------------------ sensor (0x08)

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SensorConfig {
    pub angle_snap: bool,
    pub motion_sync: bool,
    pub ripple_control: bool,
}

impl Default for SensorConfig {
    fn default() -> Self {
        // Official defaults: AngleSnap=0, MotionSync=1, RippleControl=0.
        Self { angle_snap: false, motion_sync: true, ripple_control: false }
    }
}

// ------------------------------------------------------------------ lift-off (0x05), debounce (0x09)

pub const LIFT_OFF_DEFAULT_MM: u8 = 2;
/// The official debounce slider has 31 positions, labeled "0ms" to "30ms".
pub const DEBOUNCE_MAX_MS: u8 = 30;
pub const DEBOUNCE_DEFAULT_MS: u8 = 2;

pub fn validate_lift_off(mm: u8) -> Result<(), ProtoError> {
    if mm == 1 || mm == 2 { Ok(()) } else { Err(ProtoError::InvalidLiftOff(mm)) }
}

pub fn validate_debounce(ms: u8) -> Result<(), ProtoError> {
    if ms <= DEBOUNCE_MAX_MS { Ok(()) } else { Err(ProtoError::InvalidDebounce(ms)) }
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
        Command::Buttons(map) => {
            map.validate()?;
            // 7 slots x 3 bytes, firmware order (see BUTTON_NAMES), then the fixed 7th slot.
            let mut data = [0u8; 21];
            for (i, a) in map.buttons.iter().enumerate() {
                data[3 * i..3 * i + 3].copy_from_slice(&a.code());
            }
            data[18..21].copy_from_slice(&BUTTON_SLOT7);
            Ok(vec![frame(0x01, next(), &data, 0, 1)])
        }
        Command::DpiColors(c) => {
            let mut data = [0u8; 24];
            for (i, rgb) in c.colors.iter().enumerate() {
                data[3 * i..3 * i + 3].copy_from_slice(rgb);
            }
            Ok(vec![frame(0x03, next(), &data, 0, 1)])
        }
        Command::LiftOff(mm) => {
            validate_lift_off(*mm)?;
            Ok(vec![frame(0x05, next(), &[*mm], 0, 1)])
        }
        Command::Lighting(l) => {
            l.validate()?;
            let data = [0x00, l.mode, ((l.brightness + 1) << 4) | ((l.speed + 1) & 0x0F), l.direction];
            Ok(vec![frame(0x06, next(), &data, 0, 1)])
        }
        Command::Power(pw) => {
            pw.validate()?;
            Ok(vec![frame(0x07, next(), &[pw.sleep_minutes, pw.led_off_while_moving as u8], 0, 1)])
        }
        Command::Sensor(se) => {
            let data = [se.angle_snap as u8, se.motion_sync as u8, se.ripple_control as u8];
            Ok(vec![frame(0x08, next(), &data, 0, 1)])
        }
        Command::Debounce(ms) => {
            validate_debounce(*ms)?;
            Ok(vec![frame(0x09, next(), &[*ms], 0, 1)])
        }
        Command::FactoryReset => Ok(vec![frame(0x0F, next(), &[0xFF], 0, 1)]),
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
    /// Current lighting effect (byte 6, read the same way by the official software).
    pub fn lighting_mode(&self) -> u8 { self.raw[6] }

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

    fn data(f: &[u8; REPORT_LEN], n: usize) -> String {
        hex(&f[5..5 + n])
    }

    fn assert_frame_ok(f: &[u8; REPORT_LEN], cmd: u8) {
        assert_eq!(f[0], CMD_REPORT_ID);
        assert_eq!(f[1], cmd);
        assert_ne!(f[2], 0);
        assert_eq!(f[3], 0x01, "single-part command");
        let mut z = *f;
        z[4] = 0;
        assert_eq!(f[4], checksum(&z));
    }

    #[test]
    fn new_commands_layout() {
        let mut seq = 1;
        let one = |c: Command, seq: &mut u8| {
            let f = build(&c, seq).unwrap();
            assert_eq!(f.len(), 1);
            assert_frame_ok(&f[0], c.code());
            f[0]
        };
        // Default button map = the firmware's own default table.
        let f = one(Command::Buttons(ButtonMap::default()), &mut seq);
        assert_eq!(data(&f, 21), "10 01 00 10 02 00 10 04 00 10 08 00 10 10 00 40 01 00 40 03 00");
        assert!(f[26..].iter().all(|&b| b == 0));
        let f = one(Command::DpiColors(DpiColors::default()), &mut seq);
        assert_eq!(data(&f, 24), "ff 00 00 00 ff 00 00 00 ff 00 ff ff ff ff 00 ff 00 ff ff ff ff ff ff ff");
        let f = one(Command::LiftOff(1), &mut seq);
        assert_eq!(data(&f, 2), "01 00");
        let f = one(Command::Lighting(LightingConfig { mode: 3, speed: 2, brightness: 3, direction: 1 }), &mut seq);
        assert_eq!(data(&f, 5), "00 03 43 01 00");
        let f = one(Command::Lighting(LightingConfig::default()), &mut seq);
        assert_eq!(data(&f, 4), "00 01 22 00");
        let f = one(Command::Power(PowerConfig { sleep_minutes: 5, led_off_while_moving: true }), &mut seq);
        assert_eq!(data(&f, 3), "05 01 00");
        let f = one(Command::Sensor(SensorConfig { angle_snap: true, motion_sync: false, ripple_control: true }), &mut seq);
        assert_eq!(data(&f, 4), "01 00 01 00");
        let f = one(Command::Debounce(12), &mut seq);
        assert_eq!(data(&f, 2), "0c 00");
        let f = one(Command::FactoryReset, &mut seq);
        assert_eq!(data(&f, 2), "ff 00");
    }

    #[test]
    fn new_commands_reject_out_of_range() {
        let mut seq = SEQ_START;
        let bad = [
            Command::LiftOff(0),
            Command::LiftOff(3),
            Command::Debounce(31),
            Command::Debounce(255),
            Command::Power(PowerConfig { sleep_minutes: 0, led_off_while_moving: false }),
            Command::Power(PowerConfig { sleep_minutes: 9, led_off_while_moving: false }),
            Command::Lighting(LightingConfig { mode: 8, ..Default::default() }),
            Command::Lighting(LightingConfig { speed: 3, ..Default::default() }),
            Command::Lighting(LightingConfig { brightness: 4, ..Default::default() }),
            Command::Lighting(LightingConfig { direction: 2, ..Default::default() }),
            Command::Buttons(ButtonMap { buttons: [ButtonAction::Right; 6] }),
        ];
        for c in bad {
            assert!(build(&c, &mut seq).is_err(), "{c:?}");
        }
        assert_eq!(seq, SEQ_START, "invalid commands must not consume a sequence number");
        // Swapping left and right is allowed (a left click still exists).
        let mut m = ButtonMap::default();
        m.buttons.swap(0, 1);
        assert!(build(&Command::Buttons(m), &mut seq).is_ok());
    }

    #[test]
    fn only_allowed_codes() {
        let all = [
            Command::Polling(500),
            Command::Dpi(DpiConfig::default()),
            Command::Buttons(ButtonMap::default()),
            Command::DpiColors(DpiColors::default()),
            Command::LiftOff(2),
            Command::Lighting(LightingConfig::default()),
            Command::Power(PowerConfig::default()),
            Command::Sensor(SensorConfig::default()),
            Command::Debounce(2),
            Command::FactoryReset,
        ];
        let mut seq = 1;
        for c in all {
            for f in build(&c, &mut seq).unwrap() {
                assert!(ALLOWED_COMMANDS.contains(&f[1]));
                assert_eq!(f[0], CMD_REPORT_ID);
            }
        }
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
