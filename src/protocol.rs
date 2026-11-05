//! Encoding of the lighting feature report (ID 0x02) and the high-level device handle.
//!
//! Report 0x02 is 7 bytes: `02 L1 L2 L3 L4 D F`
//! * `L1..L4` – brightness of the four lamps, 0..=100
//! * `D`      – frame duration in 10 ms units (1..=255)
//! * `F`      – flags: 0x02 first frame, 0x04 last frame, 0x08 loop,
//!   0x10 status command (bit 0 = on/off)
//!
//! See `docs/PROTOCOL.md` for details.

use std::io;
use std::path::PathBuf;

use crate::effects::Effect;
use crate::hid::{self, HidRaw};

/// The only report ID this program ever writes.
pub const LIGHT_REPORT_ID: u8 = 0x02;
/// Read-only report carrying the "lighting enabled" bit.
pub const STATE_REPORT_ID: u8 = 0x01;
/// Payload length of report 0x01 (without the ID byte).
const STATE_REPORT_LEN: usize = 2;

pub const FLAG_FIRST: u8 = 0x02;
pub const FLAG_LAST: u8 = 0x04;
pub const FLAG_LOOP: u8 = 0x08;
pub const FLAG_STATUS: u8 = 0x10;
/// Bits that are defined in the report descriptor (bits 5..7 are padding).
pub const FLAG_MASK: u8 = 0x1F;

/// Maximum brightness value of a lamp.
pub const MAX_LEVEL: u8 = 100;
/// Upper bound for the number of frames of one effect (the vendor software uses up to ~44).
pub const MAX_FRAMES: usize = 48;
/// Longest frame that can be encoded (255 * 10 ms).
pub const MAX_FRAME_MS: u32 = 2550;

/// Raw 7-byte report (ID included).
pub type Report = [u8; 7];

/// Enable lighting (status command, bit0 = 1).
pub const CMD_ENABLE: Report = [LIGHT_REPORT_ID, 0, 0, 0, 0, 1, FLAG_STATUS | 0x01];
/// Disable lighting. While disabled the device ignores frames until enabled again.
pub const CMD_DISABLE: Report = [LIGHT_REPORT_ID, 0, 0, 0, 0, 1, FLAG_STATUS];
/// Terminate the effect that is currently playing (lights off).
pub const CMD_TERMINATE: Report = [LIGHT_REPORT_ID, 0, 0, 0, 0, 1, FLAG_LAST | FLAG_FIRST];

/// One animation frame: brightness of the four lamps (L1..L4) and its duration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    pub levels: [u8; 4],
    pub ms: u32,
}

impl Frame {
    pub const fn new(levels: [u8; 4], ms: u32) -> Self {
        Self { levels, ms }
    }

    pub const fn all(level: u8, ms: u32) -> Self {
        Self {
            levels: [level; 4],
            ms,
        }
    }
}

/// Converts a duration in milliseconds to the D byte (10 ms units, 1..=255).
pub fn duration_byte(ms: u32) -> u8 {
    (ms / 10).clamp(1, 255) as u8
}

/// Scales a 0..=100 level by a 0..=100 percentage, rounding to nearest.
pub fn scale_level(level: u8, percent: u8) -> u8 {
    let l = u32::from(level.min(MAX_LEVEL));
    let p = u32::from(percent.min(100));
    ((l * p + 50) / 100) as u8
}

/// Encodes a whole effect into the sequence of reports that must be sent back-to-back.
///
/// `brightness` (0..=100) scales every lamp level.
pub fn encode_effect(frames: &[Frame], looping: bool, brightness: u8) -> Vec<Report> {
    let n = frames.len();
    frames
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let mut flags = 0;
            if i == 0 {
                flags |= FLAG_FIRST;
            }
            if i + 1 == n {
                flags |= FLAG_LAST;
                if looping {
                    flags |= FLAG_LOOP;
                }
            }
            let l = f.levels.map(|v| scale_level(v, brightness));
            [
                LIGHT_REPORT_ID,
                l[0],
                l[1],
                l[2],
                l[3],
                duration_byte(f.ms),
                flags,
            ]
        })
        .collect()
}

/// Validates a user-supplied report 0x02 payload (6 bytes, without the ID).
pub fn validate_raw_payload(p: &[u8]) -> Result<Report, String> {
    if p.len() != 6 {
        return Err(format!(
            "report 0x02 needs exactly 6 payload bytes, got {}",
            p.len()
        ));
    }
    if let Some(v) = p[..4].iter().find(|&&v| v > MAX_LEVEL) {
        return Err(format!("lamp levels must be 0..=100 (got {v})"));
    }
    if p[4] == 0 {
        return Err("duration byte must be 1..=255".into());
    }
    if p[5] & !FLAG_MASK != 0 {
        return Err(format!("flags 0x{:02x}: only bits 0..4 are defined", p[5]));
    }
    if p[5] & FLAG_STATUS != 0 && p[5] & !(FLAG_STATUS | 0x01) != 0 {
        return Err("status command (0x10) may only be combined with bit 0".into());
    }
    Ok([LIGHT_REPORT_ID, p[0], p[1], p[2], p[3], p[4], p[5]])
}

/// Is this report a single status/terminate command (sent once) rather than an effect frame?
pub fn is_command(r: &Report) -> bool {
    r[6] & FLAG_STATUS != 0 || *r == CMD_TERMINATE
}

/// High-level handle to the touchpad light. Re-discovers and reopens the hidraw
/// node transparently after I/O errors (e.g. after resume or a driver rebind).
pub struct Light {
    fixed_path: Option<PathBuf>,
    dev: Option<HidRaw>,
}

impl Light {
    /// `path = None` means auto-detect.
    pub fn new(path: Option<PathBuf>) -> Self {
        Self {
            fixed_path: path,
            dev: None,
        }
    }

    /// Opens the device now (to report errors early).
    pub fn open(path: Option<PathBuf>) -> io::Result<Self> {
        let mut l = Self::new(path);
        l.device()?;
        Ok(l)
    }

    pub fn path(&self) -> Option<&std::path::Path> {
        self.dev.as_ref().map(|d| d.path.as_path())
    }

    fn device(&mut self) -> io::Result<&HidRaw> {
        if self.dev.is_none() {
            let p = match &self.fixed_path {
                Some(p) => p.clone(),
                None => hid::find_hidraw()?,
            };
            self.dev = Some(HidRaw::open(&p)?);
        }
        Ok(self.dev.as_ref().unwrap())
    }

    /// Runs `f` on the device; on failure drops the handle and retries once with a fresh one.
    fn with_retry<T>(&mut self, mut f: impl FnMut(&HidRaw) -> io::Result<T>) -> io::Result<T> {
        match self.device().and_then(&mut f) {
            Ok(v) => Ok(v),
            Err(_) => {
                self.dev = None;
                let r = self.device().and_then(&mut f);
                if r.is_err() {
                    self.dev = None;
                }
                r
            }
        }
    }

    fn send_reports(&mut self, reports: &[Report]) -> io::Result<()> {
        // Whitelist: we only ever write report 0x02.
        assert!(reports.iter().all(|r| r[0] == LIGHT_REPORT_ID));
        self.with_retry(|d| reports.iter().try_for_each(|r| d.set_feature(r)))
    }

    /// Sends a single status/terminate command (once).
    pub fn command(&mut self, cmd: Report) -> io::Result<()> {
        self.send_reports(&[cmd])
    }

    pub fn enable(&mut self) -> io::Result<()> {
        self.command(CMD_ENABLE)
    }

    pub fn disable(&mut self) -> io::Result<()> {
        self.command(CMD_DISABLE)
    }

    pub fn terminate(&mut self) -> io::Result<()> {
        self.command(CMD_TERMINATE)
    }

    /// Uploads an encoded effect. The firmware uses the first upload to interrupt a running
    /// effect, so the sequence is sent twice in a row.
    pub fn send_effect_reports(&mut self, reports: &[Report]) -> io::Result<()> {
        let mut twice = reports.to_vec();
        twice.extend_from_slice(reports);
        self.send_reports(&twice)
    }

    pub fn play(&mut self, effect: &Effect, brightness: u8) -> io::Result<()> {
        self.send_effect_reports(&encode_effect(&effect.frames, effect.looping, brightness))
    }

    /// Reads report 0x01 and returns the "lighting enabled" bit.
    pub fn is_enabled(&mut self) -> io::Result<bool> {
        self.with_retry(|d| d.get_feature(STATE_REPORT_ID, STATE_REPORT_LEN))
            .map(|p| p[0] & 0x01 != 0)
    }

    /// Raw content of report 0x01 (payload only).
    pub fn state_report(&mut self) -> io::Result<Vec<u8>> {
        self.with_retry(|d| d.get_feature(STATE_REPORT_ID, STATE_REPORT_LEN))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_match_known_bytes() {
        assert_eq!(CMD_ENABLE, [0x02, 0, 0, 0, 0, 0x01, 0x11]);
        assert_eq!(CMD_DISABLE, [0x02, 0, 0, 0, 0, 0x01, 0x10]);
        assert_eq!(CMD_TERMINATE, [0x02, 0, 0, 0, 0, 0x01, 0x06]);
        assert!(is_command(&CMD_ENABLE) && is_command(&CMD_DISABLE) && is_command(&CMD_TERMINATE));
    }

    #[test]
    fn duration_encoding() {
        assert_eq!(duration_byte(0), 1);
        assert_eq!(duration_byte(5), 1);
        assert_eq!(duration_byte(70), 7);
        assert_eq!(duration_byte(1000), 100);
        assert_eq!(duration_byte(2550), 255);
        assert_eq!(duration_byte(100_000), 255);
    }

    #[test]
    fn scaling() {
        assert_eq!(scale_level(100, 100), 100);
        assert_eq!(scale_level(100, 50), 50);
        assert_eq!(scale_level(33, 50), 17);
        assert_eq!(scale_level(200, 100), 100);
        assert_eq!(scale_level(80, 0), 0);
    }

    #[test]
    fn single_steady_frame_has_0e_flags() {
        let r = encode_effect(&[Frame::all(100, 1000)], true, 100);
        assert_eq!(r, vec![[0x02, 100, 100, 100, 100, 100, 0x0E]]);
        let r = encode_effect(&[Frame::all(100, 1000)], false, 100);
        assert_eq!(r[0][6], 0x06);
    }

    #[test]
    fn multi_frame_flags() {
        let f = [Frame::all(10, 30), Frame::all(20, 30), Frame::all(30, 30)];
        let r = encode_effect(&f, true, 50);
        assert_eq!(r.len(), 3);
        assert_eq!(r[0], [0x02, 5, 5, 5, 5, 3, FLAG_FIRST]);
        assert_eq!(r[1][6], 0);
        assert_eq!(r[2][6], FLAG_LAST | FLAG_LOOP);
        let r = encode_effect(&f, false, 100);
        assert_eq!(r[2][6], FLAG_LAST);
    }

    #[test]
    fn raw_validation() {
        assert_eq!(
            validate_raw_payload(&[100, 0, 0, 0, 10, 0x0E]).unwrap()[0],
            0x02
        );
        assert!(validate_raw_payload(&[0, 0, 0, 0, 1, 0x11]).is_ok());
        assert!(validate_raw_payload(&[101, 0, 0, 0, 10, 0x0E]).is_err());
        assert!(validate_raw_payload(&[0, 0, 0, 0, 0, 0x0E]).is_err());
        assert!(validate_raw_payload(&[0, 0, 0, 0, 1, 0x20]).is_err());
        assert!(validate_raw_payload(&[0, 0, 0, 0, 1, 0x18]).is_err());
        assert!(validate_raw_payload(&[0, 0, 0, 0, 1]).is_err());
    }
}
