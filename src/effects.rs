//! Built-in, procedurally generated light effects.
//!
//! Lamp order: the vendor's own "circle" animation lights the lamps one after
//! the other in the order L2 -> L4 -> L3 -> L1, so that is the ring order used
//! by every rotating effect here ([`RING`]). Lamps that are opposite on the
//! ring (L2 + L3 and L4 + L1) form the two pairs used by `twinkle`.
//! The physical corner of each lamp is not verified.

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

impl Effect {
    /// Index of the frame the firmware is showing `elapsed_ms` after the effect
    /// started (wrapping around for looping effects).
    pub fn frame_at(&self, elapsed_ms: u64) -> usize {
        let cycle = self.cycle_ms().max(1);
        let mut t = if self.looping {
            elapsed_ms % cycle
        } else {
            elapsed_ms.min(cycle - 1)
        };
        for (i, f) in self.frames.iter().enumerate() {
            let d = u64::from(crate::protocol::duration_byte(f.ms)) * 10;
            if t < d {
                return i;
            }
            t -= d;
        }
        self.frames.len() - 1
    }

    /// A non-looping "graceful ending": one more cycle of this effect, starting
    /// from `start` (the frame currently shown) and fading linearly to dark.
    pub fn fade_out(&self, start: usize) -> Effect {
        let n = self.frames.len();
        let steps = n.max(2);
        let frames = (0..steps)
            .map(|i| {
                let f = self.frames[(start + i) % n];
                let k = 1.0 - (i + 1) as f64 / steps as f64; // last frame -> 0
                Frame::new(f.levels.map(|l| (f64::from(l) * k).round() as u8), f.ms)
            })
            .collect();
        effect(&self.name, "fade-out", false, frames)
    }

    /// Durations (ms) of the dark frames at the end and at the start of the
    /// effect, i.e. the dark pause where a looping effect wraps around.
    /// `None` if the effect never goes dark there (or is dark all the time).
    pub fn dark_gap(&self) -> Option<(u64, u64)> {
        let dark = |f: &&Frame| f.levels == [0; 4];
        let ms = |f: &Frame| u64::from(crate::protocol::duration_byte(f.ms)) * 10;
        if self.frames.iter().all(|f| dark(&f)) {
            return None;
        }
        let trailing: u64 = self.frames.iter().rev().take_while(dark).map(ms).sum();
        let leading: u64 = self.frames.iter().take_while(dark).map(ms).sum();
        (trailing + leading > 0).then_some((trailing, leading))
    }

    /// How long to keep a looping effect running, `elapsed_ms` after it started,
    /// so that stopping it lands in the middle of its dark pause (a natural end,
    /// like the vendor service that stops its effect at a cycle boundary).
    /// `None` if the effect has no dark pause.
    pub fn ms_until_dark(&self, elapsed_ms: u64) -> Option<u64> {
        let (trailing, leading) = self.dark_gap()?;
        let cycle = self.cycle_ms().max(1);
        let t = elapsed_ms % cycle;
        if t < leading || t >= cycle - trailing {
            return Some(0); // already dark
        }
        Some(cycle - trailing + (trailing + leading) / 2 - t)
    }
}

/// Lamp indices (0 = L1 .. 3 = L4) in ring order: L2, L4, L3, L1.
pub const RING: [usize; 4] = [1, 3, 2, 0];

/// Position of `lamp` (0-based index) on the ring, see [`RING`].
pub fn ring_pos(lamp: usize) -> usize {
    RING.iter()
        .position(|&l| l == lamp)
        .expect("lamp index 0..4")
}

/// Names of all built-in effects.
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

/// Ease-in curve used for lamps filling up / emptying (0..1 -> 0..1).
fn ease_in(x: f64) -> f64 {
    x.clamp(0.0, 1.0).powf(1.8)
}

fn circle() -> Effect {
    // Same look as the vendor's Copilot-key animation, generated procedurally:
    // starting dark, the lamps fill up one at a time in ring order with an
    // ease-in ramp, then empty again in the same order, ending dark.
    const STEPS: usize = 5;
    const MS: u32 = 70;
    let mut levels = [0u8; 4];
    let mut frames = vec![Frame::new(levels, MS)];
    for &lamp in &RING {
        for j in 1..=STEPS {
            levels[lamp] = lvl(ease_in(j as f64 / STEPS as f64));
            frames.push(Frame::new(levels, MS));
        }
    }
    for (n, &lamp) in RING.iter().enumerate() {
        for j in (0..STEPS).rev() {
            levels[lamp] = lvl(ease_in(j as f64 / STEPS as f64));
            frames.push(Frame::new(levels, MS));
        }
        if n + 1 < RING.len() {
            frames.push(Frame::new(levels, MS)); // short pause between lamps
        }
    }
    effect(
        "circle",
        "lamps fill up one by one around the ring, then empty (3.1 s)",
        false,
        frames,
    )
}

fn spin() -> Effect {
    // A soft "beam" rotating along the ring; adjacent lamps cross-fade (cos^2 + sin^2 = 1).
    const N: usize = 32;
    let frames = (0..N)
        .map(|i| {
            let p = 4.0 * i as f64 / N as f64;
            let levels = std::array::from_fn(|k| {
                let pos = ring_pos(k) as f64;
                let d = ring_dist(pos, p).min(ring_dist(p, pos));
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
        "spin",
        "a light rotating around the four lamps (1.9 s)",
        true,
        frames,
    )
}

fn comet() -> Effect {
    // Bright head moving along the ring with an exponentially fading tail.
    const N: usize = 32;
    let frames = (0..N)
        .map(|i| {
            let head = 4.0 * i as f64 / N as f64;
            let levels = std::array::from_fn(|k| {
                // how far the lamp is behind the head
                let behind = ring_dist(ring_pos(k) as f64, head);
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
        "a comet with a fading tail circling around the lamps (1.6 s)",
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
    effect("pulse", "double flash that fades out (1 s)", false, frames)
}

fn twinkle() -> Effect {
    // Same look as the vendor's NPU animation, generated procedurally: the pair
    // L2 + L3 swells up and down, then cross-fades into the pair L4 + L1,
    // followed by a short dark pause (where the loop wraps around).
    const N: usize = 41;
    const HALF: f64 = 12.0; // frames from dark to peak
    const PEAKS: [f64; 2] = [11.0, 24.0];
    let pairs = [[RING[0], RING[2]], [RING[1], RING[3]]];
    let frames = (0..N)
        .map(|i| {
            let mut levels = [0u8; 4];
            for (pair, peak) in pairs.iter().zip(PEAKS) {
                let d = (i as f64 - peak).abs();
                let v = if d < HALF {
                    (1.0 - d / HALF).powf(1.7)
                } else {
                    0.0
                };
                for &lamp in pair {
                    levels[lamp] = lvl(v);
                }
            }
            Frame::new(levels, 80)
        })
        .collect();
    effect(
        "twinkle",
        "the two lamp pairs (L2+L3, L4+L1) swell and cross-fade (3.3 s loop)",
        true,
        frames,
    )
}

fn wave() -> Effect {
    // One half of the ring (L2, L4) -> the other half (L3, L1) and back.
    const N: usize = 36;
    let frames = (0..N)
        .map(|i| {
            let x = 0.5 - 0.5 * (2.0 * PI * i as f64 / N as f64).cos(); // 0 = first half
            let a = lvl((PI / 2.0 * x).cos().powi(2));
            let b = lvl((PI / 2.0 * (1.0 - x)).cos().powi(2));
            let mut levels = [0u8; 4];
            levels[RING[0]] = a;
            levels[RING[1]] = a;
            levels[RING[2]] = b;
            levels[RING[3]] = b;
            Frame::new(levels, 50)
        })
        .collect();
    effect(
        "wave",
        "light sweeping between the two halves of the ring (1.8 s)",
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
        "circle" => circle(),
        "spin" => spin(),
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
    fn frame_at_tracks_phase() {
        let c = builtin("spin").unwrap(); // 32 frames x 60 ms
        assert_eq!(c.frame_at(0), 0);
        assert_eq!(c.frame_at(59), 0);
        assert_eq!(c.frame_at(60), 1);
        assert_eq!(c.frame_at(c.cycle_ms() + 125), 2); // wraps around
    }

    #[test]
    fn fade_out_continues_and_ends_dark() {
        let c = builtin("spin").unwrap();
        let f = c.fade_out(5);
        assert!(!f.looping);
        assert_eq!(f.frames.len(), c.frames.len());
        assert!(f.validate().is_ok());
        // continues from the current position...
        let first = f.frames[0].levels;
        let orig = c.frames[5].levels;
        for k in 0..4 {
            assert!(first[k] <= orig[k] && (orig[k] == 0 || first[k] > 0));
        }
        // ...and ends completely dark
        assert_eq!(f.frames.last().unwrap().levels, [0; 4]);
    }

    #[test]
    fn ring_order() {
        assert_eq!(RING, [1, 3, 2, 0]); // L2, L4, L3, L1
        assert_eq!((0..4).map(ring_pos).collect::<Vec<_>>(), vec![3, 0, 2, 1]);
    }

    #[test]
    fn circle_fills_in_ring_order_and_ends_dark() {
        let c = builtin("circle").unwrap();
        assert!(!c.looping);
        assert!(c.frames.len() <= 48);
        assert!(c.frames.iter().all(|f| f.ms == 70)); // D = 7, as at 14 fps
        assert_eq!(c.frames[0].levels, [0; 4]);
        assert_eq!(c.frames.last().unwrap().levels, [0; 4]);
        // order in which each lamp first reaches full brightness
        let mut order: Vec<usize> = (0..4).collect();
        let first_full = |k: usize| c.frames.iter().position(|f| f.levels[k] == 100).unwrap();
        order.sort_by_key(|&k| first_full(k));
        assert_eq!(order, RING);
        // ... and the order in which they go dark again
        let last_lit = |k: usize| c.frames.iter().rposition(|f| f.levels[k] > 0).unwrap();
        order.sort_by_key(|&k| last_lit(k));
        assert_eq!(order, RING);
        assert!(c.frames.iter().any(|f| f.levels == [100; 4]));
        // ease-in: the ramp of the first lamp is monotonic and starts slow
        let ramp: Vec<u8> = c.frames[..6].iter().map(|f| f.levels[RING[0]]).collect();
        assert!(ramp.windows(2).all(|w| w[0] < w[1]), "{ramp:?}");
        assert!(ramp[1] < 20 && ramp[5] == 100, "{ramp:?}");
    }

    #[test]
    fn twinkle_cross_fades_pairs() {
        let t = builtin("twinkle").unwrap();
        assert!(t.frames.len() <= 48);
        assert!(t.frames.iter().all(|f| f.ms == 80)); // D = 8, as at 12 fps
        for f in &t.frames {
            let [l1, l2, l3, l4] = f.levels;
            assert_eq!(l2, l3, "{f:?}");
            assert_eq!(l1, l4, "{f:?}");
        }
        let peak = |k: usize| {
            t.frames
                .iter()
                .position(|f| f.levels[k] == 100)
                .unwrap_or_else(|| panic!("lamp {k} never at 100"))
        };
        assert!(peak(1) < peak(0)); // L2+L3 first, then L4+L1
        // they overlap (cross-fade) somewhere
        assert!(
            t.frames
                .iter()
                .any(|f| f.levels[0] > 10 && f.levels[1] > 10)
        );
        assert_eq!(t.frames.last().unwrap().levels, [0; 4]);
    }

    #[test]
    fn stopping_at_the_dark_pause() {
        let t = builtin("twinkle").unwrap();
        let cycle = t.cycle_ms();
        let (trailing, leading) = t.dark_gap().unwrap();
        assert!(trailing >= 300 && leading == 0);
        assert_eq!(t.ms_until_dark(0), Some(cycle - trailing / 2));
        assert_eq!(t.ms_until_dark(cycle - 10), Some(0));
        assert_eq!(
            t.ms_until_dark(cycle + 1000),
            Some(cycle - trailing / 2 - 1000)
        );
        assert_eq!(t.frames[t.frame_at(cycle - trailing / 2)].levels, [0; 4]);
        assert!(builtin("steady").unwrap().dark_gap().is_none());
        assert!(builtin("comet").unwrap().ms_until_dark(5).is_none());
    }

    #[test]
    fn spin_walks_the_ring() {
        let c = builtin("spin").unwrap();
        // beam on L2, then L4, then L3, then L1 (ring order)
        assert_eq!(c.frames[0].levels, [0, 100, 0, 0]);
        assert_eq!(c.frames[8].levels, [0, 0, 0, 100]);
        assert_eq!(c.frames[16].levels, [0, 0, 100, 0]);
        assert_eq!(c.frames[24].levels, [100, 0, 0, 0]);
        // halfway between L2 and L4 both are at 50 %
        assert_eq!(c.frames[4].levels, [0, 50, 0, 50]);
    }

    #[test]
    fn comet_has_tail_behind_head() {
        let c = builtin("comet").unwrap();
        let [l1, l2, l3, l4] = c.frames[8].levels; // head on L4 (ring position 1)
        assert_eq!(l4, 100);
        assert!(l2 > l1 && l1 > l3, "{:?}", c.frames[8].levels);
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
