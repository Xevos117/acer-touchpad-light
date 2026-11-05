//! Daemon configuration (TOML).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use evdev::Key;
use serde::Deserialize;

use crate::effects::{Effect, Library};
use crate::protocol::Frame;

pub const DEFAULT_PATH: &str = "/etc/acer-touchpad-light/config.toml";

fn default_true() -> bool {
    true
}
fn default_brightness() -> u8 {
    100
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Global brightness scale in percent (0..=100) applied to every effect.
    #[serde(default = "default_brightness")]
    pub brightness: u8,
    /// Explicit hidraw node; auto-detected when absent.
    #[serde(default)]
    pub device: Option<PathBuf>,
    /// Re-enable lighting when report 0x01 says it is disabled.
    #[serde(default = "default_true")]
    pub reenable: bool,
    #[serde(default)]
    pub npu: NpuConfig,
    #[serde(default = "default_keys", rename = "key")]
    pub keys: Vec<KeyBinding>,
    #[serde(default)]
    pub effects: BTreeMap<String, CustomEffect>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NpuConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_npu_effect")]
    pub effect: String,
    #[serde(default = "default_poll_ms")]
    pub poll_ms: u64,
    #[serde(default = "default_idle_ms")]
    pub idle_ms: u64,
    /// Override of the busy-time counter file (default: first
    /// /sys/class/accel/accel*/device/npu_busy_time_us).
    #[serde(default)]
    pub sysfs: Option<PathBuf>,
}

fn default_npu_effect() -> String {
    "twinkle".into()
}
fn default_poll_ms() -> u64 {
    250
}
fn default_idle_ms() -> u64 {
    2000
}

impl Default for NpuConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            effect: default_npu_effect(),
            poll_ms: default_poll_ms(),
            idle_ms: default_idle_ms(),
            sysfs: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyBinding {
    /// evdev key names, e.g. ["KEY_LEFTMETA", "KEY_LEFTSHIFT", "KEY_F23"].
    /// The `KEY_` prefix is optional and names are case-insensitive.
    pub keys: Vec<String>,
    #[serde(default = "default_key_effect")]
    pub effect: String,
    /// How long the effect is shown. One-shot effects always play to the end
    /// (a longer value holds their last frame); looping ones run for this long,
    /// 3 s when 0/absent.
    #[serde(default)]
    pub duration_ms: u64,
}

fn default_key_effect() -> String {
    "circle".into()
}

fn default_keys() -> Vec<KeyBinding> {
    vec![KeyBinding {
        keys: vec![
            "KEY_LEFTMETA".into(),
            "KEY_LEFTSHIFT".into(),
            "KEY_F23".into(),
        ],
        effect: default_key_effect(),
        duration_ms: 0,
    }]
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomEffect {
    #[serde(default, rename = "loop")]
    pub looping: bool,
    #[serde(default)]
    pub description: String,
    /// `[l1, l2, l3, l4, ms]` per frame.
    pub frames: Vec<[u32; 5]>,
}

impl Default for Config {
    fn default() -> Self {
        toml::from_str("").expect("empty config is valid")
    }
}

/// Parses a key name such as `KEY_F23`, `f23` or `leftmeta`.
pub fn parse_key(name: &str) -> Result<Key, String> {
    let up = name.trim().to_ascii_uppercase();
    let full = if up.starts_with("KEY_") || up.starts_with("BTN_") {
        up
    } else {
        format!("KEY_{up}")
    };
    Key::from_str(&full).map_err(|_| format!("unknown key name '{name}'"))
}

impl Config {
    pub fn parse(text: &str) -> Result<Self, String> {
        let c: Config = toml::from_str(text).map_err(|e| e.to_string())?;
        c.validate()?;
        Ok(c)
    }

    /// Loads `path`. If `path` is the default path and does not exist, built-in defaults are used.
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(t) => Self::parse(&t).map_err(|e| format!("{}: {e}", path.display())),
            Err(e)
                if e.kind() == std::io::ErrorKind::NotFound && path == Path::new(DEFAULT_PATH) =>
            {
                Ok(Self::default())
            }
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    pub fn library(&self) -> Result<Library, String> {
        let mut lib = Library::default();
        for (name, c) in &self.effects {
            let frames = c
                .frames
                .iter()
                .enumerate()
                .map(|(i, f)| {
                    if f[..4].iter().any(|&l| l > 100) {
                        return Err(format!(
                            "effect '{name}' frame {}: levels must be 0..=100",
                            i + 1
                        ));
                    }
                    Ok(Frame::new(
                        [f[0] as u8, f[1] as u8, f[2] as u8, f[3] as u8],
                        f[4],
                    ))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let e = Effect {
                name: name.clone(),
                description: if c.description.is_empty() {
                    "custom effect".into()
                } else {
                    c.description.clone()
                },
                looping: c.looping,
                frames,
            };
            e.validate()?;
            lib.custom.insert(name.clone(), e);
        }
        Ok(lib)
    }

    /// Key combos resolved to evdev keys.
    pub fn key_combos(&self) -> Result<Vec<Vec<Key>>, String> {
        self.keys
            .iter()
            .map(|b| {
                if b.keys.is_empty() {
                    return Err("[[key]] entry with empty 'keys'".to_string());
                }
                b.keys.iter().map(|k| parse_key(k)).collect()
            })
            .collect()
    }

    fn validate(&self) -> Result<(), String> {
        if self.brightness > 100 {
            return Err("brightness must be 0..=100".into());
        }
        if self.npu.poll_ms < 20 {
            return Err("npu.poll_ms must be >= 20".into());
        }
        let lib = self.library()?;
        self.key_combos()?;
        let mut used: Vec<&str> = self.keys.iter().map(|k| k.effect.as_str()).collect();
        if self.npu.enabled {
            used.push(&self.npu.effect);
        }
        for e in used {
            if lib.get(e).is_none() {
                return Err(format!("unknown effect '{e}'"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let c = Config::default();
        assert_eq!(c.brightness, 100);
        assert!(c.npu.enabled);
        assert_eq!(c.npu.effect, "twinkle");
        assert_eq!(c.keys.len(), 1);
        assert_eq!(
            c.key_combos().unwrap(),
            vec![vec![Key::KEY_LEFTMETA, Key::KEY_LEFTSHIFT, Key::KEY_F23]]
        );
    }

    #[test]
    fn example_config_parses() {
        let text = include_str!("../config/config.toml");
        let c = Config::parse(text).unwrap();
        assert!(!c.keys.is_empty());
        assert!(c.library().unwrap().get("heartbeat").is_some());
    }

    #[test]
    fn full_config() {
        let c = Config::parse(
            r#"
            brightness = 60
            [npu]
            effect = "spin"
            idle_ms = 1500
            [[key]]
            keys = ["leftctrl", "KEY_LEFTALT", "l"]
            effect = "mine"
            duration_ms = 2000
            [effects.mine]
            loop = true
            frames = [[100, 0, 0, 0, 200], [0, 100, 0, 0, 200]]
            "#,
        )
        .unwrap();
        assert_eq!(c.brightness, 60);
        assert_eq!(c.npu.poll_ms, 250);
        assert_eq!(
            c.key_combos().unwrap()[0],
            vec![Key::KEY_LEFTCTRL, Key::KEY_LEFTALT, Key::KEY_L]
        );
        let e = c.library().unwrap().get("mine").unwrap();
        assert!(e.looping);
        assert_eq!(e.frames[1], Frame::new([0, 100, 0, 0], 200));
    }

    #[test]
    fn rejects_bad_configs() {
        assert!(Config::parse("brightness = 101").is_err());
        assert!(Config::parse("[npu]\neffect = \"nope\"").is_err());
        assert!(Config::parse("[[key]]\nkeys = [\"KEY_NOPE\"]").is_err());
        assert!(Config::parse("[[key]]\nkeys = []").is_err());
        assert!(Config::parse("[effects.x]\nframes = [[101,0,0,0,100]]").is_err());
        assert!(Config::parse("[effects.x]\nframes = [[1,0,0,0,5]]").is_err());
        assert!(Config::parse("[effects.x]\nframes = []").is_err());
        assert!(Config::parse("bogus = 1").is_err());
    }

    #[test]
    fn key_names() {
        assert_eq!(parse_key("f23").unwrap(), Key::KEY_F23);
        assert_eq!(parse_key("KEY_LEFTMETA").unwrap(), Key::KEY_LEFTMETA);
        assert!(parse_key("whatever").is_err());
    }
}
