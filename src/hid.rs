//! Low-level access to the touchpad's hidraw node (feature reports).

use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

/// HID_ID of the PixArt PIXA3838 touchpad (bus I2C = 0x18, VID 093A, PID 3838).
const KNOWN_HID_IDS: &[&str] = &["0018:0000093A:00003838"];

/// Vendor collection that carries the lighting reports: Usage Page 0xFF58, Usage 0x05.
const LIGHT_COLLECTION: &[u8] = &[0x06, 0x58, 0xFF, 0x09, 0x05];

/// The only feature report this program is allowed to write.
const WRITABLE_REPORT_ID: u8 = 0x02;

const IOC_WRITE: u32 = 1;
const IOC_READ: u32 = 2;

const fn ioc(dir: u32, ty: u8, nr: u8, size: usize) -> u32 {
    (dir << 30) | ((size as u32) << 16) | ((ty as u32) << 8) | nr as u32
}

/// HIDIOCSFEATURE(len)
const fn hidiocsfeature(len: usize) -> u32 {
    ioc(IOC_WRITE | IOC_READ, b'H', 0x06, len)
}

/// HIDIOCGFEATURE(len)
const fn hidiocgfeature(len: usize) -> u32 {
    ioc(IOC_WRITE | IOC_READ, b'H', 0x07, len)
}

fn has_light_collection(hidraw_sysdir: &Path) -> bool {
    fs::read(hidraw_sysdir.join("device/report_descriptor"))
        .map(|d| {
            d.windows(LIGHT_COLLECTION.len())
                .any(|w| w == LIGHT_COLLECTION)
        })
        .unwrap_or(false)
}

/// Finds the hidraw node of a touchpad exposing the lighting collection.
///
/// Prefers known HID IDs, then falls back to any hidraw whose report descriptor
/// contains the 0xFF58/0x05 vendor collection.
pub fn find_hidraw() -> io::Result<PathBuf> {
    let mut fallback = None;
    let mut entries: Vec<_> = fs::read_dir("/sys/class/hidraw")?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .collect();
    entries.sort();
    for sys in entries {
        if !has_light_collection(&sys) {
            continue;
        }
        let name = sys.file_name().unwrap().to_owned();
        let dev = Path::new("/dev").join(name);
        let uevent = fs::read_to_string(sys.join("device/uevent")).unwrap_or_default();
        let known = uevent.lines().any(|l| {
            l.strip_prefix("HID_ID=")
                .is_some_and(|id| KNOWN_HID_IDS.iter().any(|k| k.eq_ignore_ascii_case(id)))
        });
        if known {
            return Ok(dev);
        }
        fallback.get_or_insert(dev);
    }
    fallback.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "no touchpad with the Acer lighting collection (0xFF58/0x05) found",
        )
    })
}

pub struct HidRaw {
    file: File,
    pub path: PathBuf,
}

impl HidRaw {
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|e| {
                if e.kind() == io::ErrorKind::PermissionDenied {
                    io::Error::new(
                        e.kind(),
                        format!(
                            "permission denied on {} (run as root or install the udev rule)",
                            path.display()
                        ),
                    )
                } else {
                    e
                }
            })?;
        Ok(Self {
            file,
            path: path.to_owned(),
        })
    }

    /// Sends a feature report. `buf[0]` must be the report ID.
    ///
    /// Only report 0x02 (lighting) may be written; anything else (e.g. the firmware
    /// update / diagnostic reports 0x41..0x43) is refused here as a last line of defence.
    pub fn set_feature(&self, buf: &[u8]) -> io::Result<()> {
        if buf.first() != Some(&WRITABLE_REPORT_ID) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "refusing to write a report other than 0x02",
            ));
        }
        let mut tmp = buf.to_vec();
        // SAFETY: tmp is a valid, writable buffer of the length encoded in the request.
        let r = unsafe {
            libc::ioctl(
                self.file.as_raw_fd(),
                hidiocsfeature(tmp.len()) as _,
                tmp.as_mut_ptr(),
            )
        };
        if r < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Reads a feature report of `len` payload bytes. Returns the payload (without ID).
    pub fn get_feature(&self, id: u8, len: usize) -> io::Result<Vec<u8>> {
        let mut buf = vec![0u8; len + 1];
        buf[0] = id;
        // SAFETY: buf is a valid, writable buffer of the length encoded in the request.
        let r = unsafe {
            libc::ioctl(
                self.file.as_raw_fd(),
                hidiocgfeature(buf.len()) as _,
                buf.as_mut_ptr(),
            )
        };
        if r < 0 {
            return Err(io::Error::last_os_error());
        }
        let n = (r as usize).min(buf.len());
        if n <= 1 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "empty feature report",
            ));
        }
        Ok(buf[1..n].to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ioctl_numbers_match_linux_headers() {
        // values from <linux/hidraw.h> for len = 3 and 7
        assert_eq!(hidiocgfeature(3), 0xc003_4807);
        assert_eq!(hidiocsfeature(7), 0xc007_4806);
    }
}
