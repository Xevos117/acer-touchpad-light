//! Built-in, procedurally generated light effects.
//!
//! Lamp order follows the circular model used by the vendor software:
//! L1 = top-right, L2 = bottom-right, L3 = bottom-left, L4 = top-left
//! (i.e. clockwise starting at the top-right corner; physical layout is probable,
//! not verified). Rotating effects therefore just walk the index 0..4.

use std::collections::BTreeMap;
use std::f64::consts::PI;

use crate::protocol::{Frame, MAX_FRAME_MS, MAX_FRAMES, MAX_LEVEL};

#[derive(Debug, Clone, PartialEq)]
pub struct Effect {
    pub name: String,
    pub description: String,
    pub looping: bool,
    pub frames: Vec<Frame>,
}

impl Effect {
    /// Duration of one playthrough in milliseconds (as the firmware will play it,
    /// i.e. after quantisation to 10 ms units).
    pub fn cycle_ms(&self) -> u64 {
        self.frames
            .iter()
            .map(|f| u64::from(crate::protocol::duration_byte(f.ms)) * 10)
            .sum()
    }

    /// Checks the limits of the protocol.
    pub fn validate(&self) -> Result<(), String> {
        if self.frames.is_empty() {
            return Err(format!("effect '{}' has no frames", self.name));
        }
        if self.frames.len() > MAX_FRAMES {
            return Err(format!(
                "effect '{}' has {} frames (max {MAX_FRAMES})",
                self.name,
                self.frames.len()
            ));
        }
        for (i, f) in self.frames.iter().enumerate() {
            if f.levels.iter().any(|&l| l > MAX_LEVEL) {
                return Err(format!(
                    "effect '{}' frame {}: levels must be 0..=100",
                    self.name,
                    i + 1
                ));
            }
            if !(10..=MAX_FRAME_MS).contains(&f.ms) {
                return Err(format!(
                    "effect '{}' frame {}: duration must be 10..={MAX_FRAME_MS} ms",
                    self.name,
                    i + 1
                ));
            }
        }
        Ok(())
    }
}

/// Names of all built-in effects (`spin` is an alias of `circle`).
pub const BUILTIN_NAMES: &[&str] = &[
    "steady", "breath", "blink", "circle", "spin", "comet", "pulse", "twinkle", "wave",
];

fn lvl(x: f64) -> u8 {
    (x.clamp(0.0, 1.0) * f64::from(MAX_LEVEL)).round() as u8
}

fn effect(name: &str, description: &str, looping: bool, frames: Vec<Frame>) -> Effect {
    Effect {
        name: name.into(),
        description: description.into(),
        looping,
        frames,
    }
}

/// Forward circular distance from `from` to `to` on a ring of 4 lamps, in [0, 4).
fn ring_dist(from: f64, to: f64) -> f64 {
    (to - from).rem_euclid(4.0)
}

fn steady() -> Effect {
    effect(
        "steady",
        "all lamps on, constant",
        true,
        vec![Frame::all(100, 1000)],
    )
}

fn breath() -> Effect {
    const N: usize = 40;
    let frames = (0..N)
        .map(|i| {
            let t = i as f64 / N as f64;
            let v = (0.5 - 0.5 * (2.0 * PI * t).cos()).powf(1.8);
            Frame::all(lvl(v), 60)
        })
        .collect();
    effect(
        "breath",
        "all lamps slowly fade in and out (2.4 s)",
        true,
        frames,
    )
}

fn blink() -> Effect {
    effect(
        "blink",
        "all lamps blink on/off (0.4 s each)",
        true,
        vec![Frame::all(100, 400), Frame::all(0, 400)],
    )
}

fn circle(name: &str) -> Effect {
    // A soft "beam" rotating clockwise; adjacent lamps cross-fade (cos^2 + sin^2 = 1).
    const N: usize = 32;
    let frames = (0..N)
        .map(|i| {
            let p = 4.0 * i as f64 / N as f64;
            let levels = std::array::from_fn(|k| {
                let d = ring_dist(k as f64, p).min(ring_dist(p, k as f64));
                if d >= 1.0 {
                    0
                } else {
                    lvl((PI / 2.0 * d).cos().powi(2))
                }
            });
            Frame::new(levels, 60)
        })
        .collect();
    effect(
        name,
        "a light rotating clockwise around the four lamps (1.9 s)",
        true,
        frames,
    )
}

fn comet() -> Effect {
    // Bright head moving clockwise with an exponentially fading tail.
    const N: usize = 32;
    let frames = (0..N)
        .map(|i| {
            let head = 4.0 * i as f64 / N as f64;
            let levels = std::array::from_fn(|k| {
                let behind = ring_dist(k as f64, head); // how far the lamp is behind the head
                let v = if behind <= 3.0 {
                    (-1.3 * behind).exp()
                } else {
                    behind - 3.0
                };
                lvl(v)
            });
            Frame::new(levels, 50)
        })
        .collect();
    effect(
        "comet",
        "a comet with a fading tail circling clockwise (1.6 s)",
        true,
        frames,
    )
}

fn pulse() -> Effect {
    // One-shot double pulse, ends dark. Good feedback for a key press.
    let mut frames = Vec::new();
    for peak in [1.0, 0.6] {
        for x in [0.3, 0.65, 1.0] {
            frames.push(Frame::all(lvl(x * peak), 30));
        }
        for i in 1..=9 {
            frames.push(Frame::all(lvl(peak * (-0.45 * i as f64).exp()), 40));
        }
    }
    frames.push(Frame::all(0, 100));
    effect(
        "pulse",
        "one-shot double flash that fades out (~1 s, not looping)",
        false,
        frames,
    )
}

fn twinkle() -> Effect {
    // Pseudo-random sparkles, deterministic so the effect is reproducible and loops seamlessly.
    const N: usize = 48;
    const LEN: usize = 10;
    let mut seed: u32 = 0x5EED_1234;
    let mut rnd = |m: usize| {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 8) as usize % m
    };
    let mut acc = [[0.0f64; N]; 4];
    for (lamp, track) in acc.iter_mut().enumerate() {
        for s in 0..3 {
            let start = (s * N / 3 + rnd(N / 3) + lamp * 5) % N;
            let peak = 0.55 + rnd(46) as f64 / 100.0;
            for j in 0..LEN {
                let env = (PI * (j as f64 + 0.5) / LEN as f64).sin();
                let cell = &mut track[(start + j) % N];
                *cell = cell.max(peak * env);
            }
        }
    }
    let frames = (0..N)
        .map(|i| Frame::new(std::array::from_fn(|k| lvl(0.04 + acc[k][i])), 60))
        .collect();
    effect(
        "twinkle",
        "random-looking sparkles on each lamp (2.9 s loop)",
        true,
        frames,
    )
}

fn wave() -> Effect {
    // Left column (L4, L3) -> right column (L1, L2) and back.
    const N: usize = 36;
    let frames = (0..N)
        .map(|i| {
            let x = 0.5 - 0.5 * (2.0 * PI * i as f64 / N as f64).cos(); // 0 = left, 1 = right
            let right = lvl((PI / 2.0 * (1.0 - x)).cos().powi(2));
            let left = lvl((PI / 2.0 * x).cos().powi(2));
            Frame::new([right, right, left, left], 50)
        })
        .collect();
    effect(
        "wave",
        "light sweeping left <-> right (1.8 s)",
        true,
        frames,
    )
}

/// Returns a built-in effect by name.
pub fn builtin(name: &str) -> Option<Effect> {
    Some(match name {
        "steady" => steady(),
        "breath" => breath(),
        "blink" => blink(),
        "circle" | "spin" => circle(name),
        "comet" => comet(),
        "pulse" => pulse(),
        "twinkle" => twinkle(),
        "wave" => wave(),
        _ => return None,
    })
}

/// Steady effect with a custom level.
pub fn steady_at(level: u8) -> Effect {
    let mut e = steady();
    e.frames = vec![Frame::all(level.min(MAX_LEVEL), 1000)];
    e
}

/// Built-ins plus custom effects from the configuration (custom ones take precedence).
#[derive(Debug, Clone, Default)]
pub struct Library {
    pub custom: BTreeMap<String, Effect>,
}

impl Library {
    pub fn get(&self, name: &str) -> Option<Effect> {
        self.custom.get(name).cloned().or_else(|| builtin(name))
    }

    pub fn all(&self) -> Vec<(Effect, bool)> {
        let mut v: Vec<(Effect, bool)> = BUILTIN_NAMES
            .iter()
            .filter(|n| !self.custom.contains_key(**n))
            .filter_map(|n| builtin(n))
            .map(|e| (e, false))
            .collect();
        v.extend(self.custom.values().cloned().map(|e| (e, true)));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{FLAG_FIRST, FLAG_LAST, FLAG_LOOP, encode_effect};

    #[test]
    fn all_builtins_are_valid() {
        for n in BUILTIN_NAMES {
            let e = builtin(n).unwrap_or_else(|| panic!("missing {n}"));
            e.validate().unwrap();
            assert!(e.frames.len() <= 48, "{n}");
        }
        assert!(builtin("nope").is_none());
    }

    #[test]
    fn looping_flags() {
        for n in BUILTIN_NAMES {
            let e = builtin(n).unwrap();
            let r = encode_effect(&e.frames, e.looping, 100);
            assert_eq!(r[0][6] & FLAG_FIRST, FLAG_FIRST);
            let last = r.last().unwrap()[6];
            assert_eq!(last & FLAG_LAST, FLAG_LAST);
            assert_eq!(last & FLAG_LOOP != 0, e.looping, "{n}");
        }
        assert!(!builtin("pulse").unwrap().looping);
    }

    #[test]
    fn pulse_ends_dark() {
        let p = builtin("pulse").unwrap();
        assert_eq!(p.frames.last().unwrap().levels, [0; 4]);
        assert!(p.frames.iter().any(|f| f.levels == [100; 4]));
        assert!(p.cycle_ms() > 500 && p.cycle_ms() < 2000);
    }

    #[test]
    fn twinkle_lights_every_lamp() {
        let t = builtin("twinkle").unwrap();
        for k in 0..4 {
            let max = t.frames.iter().map(|f| f.levels[k]).max().unwrap();
            let min = t.frames.iter().map(|f| f.levels[k]).min().unwrap();
            assert!(max >= 50 && min <= 10, "lamp {k}: {min}..{max}");
        }
    }

    #[test]
    fn circle_rotates_clockwise() {
        let c = builtin("circle").unwrap();
        // frame 0: beam on L1; after a quarter turn: on L2
        assert_eq!(c.frames[0].levels, [100, 0, 0, 0]);
        assert_eq!(c.frames[8].levels, [0, 100, 0, 0]);
        assert_eq!(c.frames[16].levels, [0, 0, 100, 0]);
        // halfway between L1 and L2 both are at 50 %
        assert_eq!(c.frames[4].levels, [50, 50, 0, 0]);
    }

    #[test]
    fn comet_has_tail_behind_head() {
        let c = builtin("comet").unwrap();
        let f = c.frames[8].levels; // head on L2
        assert_eq!(f[1], 100);
        assert!(f[0] > f[3] && f[3] > f[2], "{f:?}");
    }

    #[test]
    fn library_custom_overrides() {
        let mut lib = Library::default();
        let e = Effect {
            name: "breath".into(),
            description: String::new(),
            looping: false,
            frames: vec![Frame::all(1, 10)],
        };
        lib.custom.insert("breath".into(), e.clone());
        assert_eq!(lib.get("breath"), Some(e));
        assert_eq!(
            lib.all().iter().filter(|(e, _)| e.name == "breath").count(),
            1
        );
    }
}
