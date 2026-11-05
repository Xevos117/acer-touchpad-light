//! The background service: NPU activity + key combos -> light effects.
//!
//! Priority: key effect (for its duration) > NPU effect (while the NPU is busy) > idle (off).
//!
//! Mirrors the vendor service: the NPU effect is played *looping* while the NPU
//! is busy and stopped at a natural point (the dark pause of its cycle); a key
//! effect is a one-shot that is always played to its end.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant, SystemTime};

use crate::config::Config;
use crate::effects::{Effect, Library};
use crate::input::{self, KeyWatcher};
use crate::protocol::Light;

const TICK: Duration = Duration::from_millis(50);
const RESCAN_EVERY: Duration = Duration::from_secs(5);
const RETRY_AFTER: Duration = Duration::from_secs(2);
const DEFAULT_LOOP_KEY_MS: u64 = 3000;

/// What should be shown right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Want {
    Idle,
    Npu,
    /// Index into the key bindings.
    Key(usize),
}

/// Pure decision logic, kept separate for testing.
pub fn decide(key: Option<(usize, Instant)>, npu_active: bool, now: Instant) -> Want {
    match key {
        Some((i, until)) if now < until => Want::Key(i),
        _ if npu_active => Want::Npu,
        _ => Want::Idle,
    }
}

/// How long a key-triggered effect is shown. One-shot effects are never cut
/// short: `duration_ms` can only extend them (keeping the last frame).
pub fn key_duration(effect: &Effect, duration_ms: u64) -> Duration {
    Duration::from_millis(match duration_ms {
        0 if effect.looping => DEFAULT_LOOP_KEY_MS,
        d if effect.looping => d,
        d => d.max(effect.cycle_ms()),
    })
}

/// The NPU effect is always repeated while the NPU is busy (the vendor service
/// plays its NPU animation with the loop flag), whatever its own `loop` setting.
pub fn npu_effect(mut e: Effect) -> Effect {
    e.looping = true;
    e
}

/// Tracks the intel_vpu busy-time counter.
struct NpuMonitor {
    path: Option<PathBuf>,
    explicit: bool,
    last_value: Option<u64>,
    last_active: Option<Instant>,
    warned: bool,
}

fn find_npu_counter() -> Option<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir("/sys/class/accel")
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path().join("device/npu_busy_time_us"))
        .filter(|p| p.exists())
        .collect();
    v.sort();
    v.into_iter().next()
}

impl NpuMonitor {
    fn new(path: Option<PathBuf>) -> Self {
        Self {
            explicit: path.is_some(),
            path,
            last_value: None,
            last_active: None,
            warned: false,
        }
    }

    fn poll(&mut self, now: Instant) {
        if self.path.is_none() {
            self.path = find_npu_counter();
        }
        let Some(p) = &self.path else {
            if !self.warned {
                eprintln!("NPU busy counter not found (intel_vpu loaded?); NPU effect inactive");
                self.warned = true;
            }
            return;
        };
        match fs::read_to_string(p)
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
        {
            Some(v) => {
                if self.last_value.is_some_and(|old| v > old) {
                    self.last_active = Some(now);
                }
                self.last_value = Some(v);
                self.warned = false;
            }
            None => {
                if !self.warned {
                    eprintln!("cannot read {}", p.display());
                    self.warned = true;
                }
                if !self.explicit {
                    self.path = None;
                }
                self.last_value = None;
            }
        }
    }

    fn active(&self, now: Instant, idle: Duration) -> bool {
        self.last_active
            .is_some_and(|t| now.duration_since(t) < idle)
    }
}

struct Player {
    light: Light,
    brightness: u8,
    reenable: bool,
    /// Effect currently playing and when it started.
    playing: Option<(Effect, Instant)>,
    /// The effect is being ended: send terminate at this time.
    terminate_at: Option<Instant>,
}

impl Player {
    fn show(&mut self, effect: Option<&Effect>) -> std::io::Result<()> {
        match effect {
            None => self.end(),
            Some(e) => {
                if self.reenable && matches!(self.light.is_enabled(), Ok(false)) {
                    eprintln!("lighting was disabled, enabling it");
                    self.light.enable()?;
                }
                self.terminate_at = None;
                self.light.play(e, self.brightness)?;
                self.playing = Some((e.clone(), Instant::now()));
                Ok(())
            }
        }
    }

    /// Ends the current effect without cutting it off:
    /// * a one-shot effect still running is left to finish, then terminated;
    /// * a looping effect with a dark pause keeps running until it reaches
    ///   that pause, then it is terminated (no visible jump);
    /// * any other looping effect fades out from where it is.
    fn end(&mut self) -> std::io::Result<()> {
        let now = Instant::now();
        let Some((e, started)) = self.playing.take() else {
            self.terminate_at = None;
            return self.light.terminate();
        };
        let elapsed = now.duration_since(started).as_millis() as u64;
        let wait = if !e.looping {
            e.cycle_ms().saturating_sub(elapsed)
        } else if let Some(ms) = e.ms_until_dark(elapsed) {
            ms
        } else {
            let fade = e.fade_out(e.frame_at(elapsed));
            self.light.play(&fade, self.brightness)?;
            fade.cycle_ms()
        };
        if wait == 0 {
            self.terminate_at = None;
            self.light.terminate()
        } else {
            self.terminate_at = Some(now + Duration::from_millis(wait));
            Ok(())
        }
    }

    /// Sends the pending terminate once the effect has reached its end.
    fn tick(&mut self, now: Instant) -> std::io::Result<()> {
        if self.terminate_at.is_some_and(|t| now >= t) {
            self.terminate_at = None;
            self.light.terminate()?;
        }
        Ok(())
    }
}

pub fn run(cfg: Config) -> Result<(), String> {
    let lib: Library = cfg.library()?;
    let combos = cfg.key_combos()?;
    let key_effects: Vec<Effect> = cfg
        .keys
        .iter()
        .map(|k| {
            lib.get(&k.effect)
                .ok_or_else(|| format!("unknown effect '{}'", k.effect))
        })
        .collect::<Result<_, _>>()?;
    let npu_effect = if cfg.npu.enabled {
        Some(
            lib.get(&cfg.npu.effect)
                .map(npu_effect)
                .ok_or_else(|| format!("unknown effect '{}'", cfg.npu.effect))?,
        )
    } else {
        None
    };

    let term = Arc::new(AtomicBool::new(false));
    for sig in [
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGHUP,
    ] {
        signal_hook::flag::register(sig, Arc::clone(&term)).map_err(|e| e.to_string())?;
    }

    // Key watcher: reader threads send the index of the triggered binding.
    let (tx, rx) = mpsc::channel::<usize>();
    let watcher = if combos.is_empty() {
        None
    } else {
        let filter_combos = combos.clone();
        let tx = Mutex::new(tx);
        Some(KeyWatcher::new(
            move |d| input::supports_any_combo(d, &filter_combos),
            move |_, key, value, held| {
                for (i, c) in combos.iter().enumerate() {
                    if input::combo_triggered(c, key, value, held) {
                        let _ = tx.lock().unwrap().send(i);
                    }
                }
            },
        ))
    };

    let mut player = Player {
        light: Light::new(cfg.device.clone()),
        brightness: cfg.brightness,
        reenable: cfg.reenable,
        playing: None,
        terminate_at: None,
    };
    let mut npu = NpuMonitor::new(cfg.npu.sysfs.clone());
    let poll = Duration::from_millis(cfg.npu.poll_ms);
    let idle = Duration::from_millis(cfg.npu.idle_ms);

    // Start from a clean state (terminate any leftover effect).
    let mut current: Option<Want> = match player.show(None) {
        Ok(()) => {
            eprintln!(
                "touchpad light: {}",
                player
                    .light
                    .path()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            );
            Some(Want::Idle)
        }
        Err(e) => {
            eprintln!("touchpad light not available yet: {e}");
            None
        }
    };
    let mut retry_at = Instant::now();
    let mut key_until: Option<(usize, Instant)> = None;
    let mut next_poll = Instant::now();
    let mut next_rescan = Instant::now();
    let mut last_mono = Instant::now();
    let mut last_wall = SystemTime::now();
    eprintln!("tplight daemon running");

    while !term.load(Ordering::Relaxed) {
        let mut force = false;
        match rx.recv_timeout(TICK) {
            Ok(i) => {
                let until = Instant::now() + key_duration(&key_effects[i], cfg.keys[i].duration_ms);
                key_until = Some((i, until));
                force = true; // restart the effect on every press
                eprintln!("key combo {:?} -> {}", cfg.keys[i].keys, cfg.keys[i].effect);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            // no key bindings: nothing will ever be sent
            Err(mpsc::RecvTimeoutError::Disconnected) => std::thread::sleep(TICK),
        }
        let now = Instant::now();

        // Resume from suspend: the monotonic clock stops, the wall clock does not.
        let wall = SystemTime::now();
        let wall_dt = wall.duration_since(last_wall).unwrap_or_default();
        if wall_dt > now.duration_since(last_mono) + Duration::from_secs(3) {
            eprintln!("resume detected, re-applying state");
            force = true;
        }
        last_wall = wall;
        last_mono = now;

        if let Some(w) = &watcher
            && now >= next_rescan
        {
            w.rescan();
            next_rescan = now + RESCAN_EVERY;
        }
        if npu_effect.is_some() && now >= next_poll {
            npu.poll(now);
            next_poll = now + poll;
        }

        if let Err(e) = player.tick(now) {
            eprintln!("terminate at end of effect failed: {e}");
        }

        let want = decide(
            key_until,
            npu_effect.is_some() && npu.active(now, idle),
            now,
        );
        if !matches!(want, Want::Key(_)) {
            key_until = None;
        }
        // After a failure (current == None) only retry every RETRY_AFTER.
        let may_send = current.is_some() || now >= retry_at;
        if may_send && (Some(want) != current || force) {
            let effect = match want {
                Want::Idle => None,
                Want::Npu => npu_effect.as_ref(),
                Want::Key(i) => Some(&key_effects[i]),
            };
            match player.show(effect) {
                Ok(()) => {
                    if Some(want) != current {
                        eprintln!(
                            "state: {want:?}{}",
                            effect.map(|e| format!(" ({})", e.name)).unwrap_or_default()
                        );
                    }
                    current = Some(want);
                }
                Err(e) => {
                    if current.is_some() {
                        eprintln!("cannot drive touchpad light: {e} (retrying)");
                    }
                    current = None;
                    retry_at = now + RETRY_AFTER;
                }
            }
        }
    }

    eprintln!("shutting down, stopping effect");
    if let Err(e) = player.show(None) {
        eprintln!("stop failed: {e}");
    }
    // let the effect reach its end (bounded), then make sure everything is off
    if let Some(t) = player.terminate_at.take() {
        let wait = t.saturating_duration_since(Instant::now());
        std::thread::sleep(wait.min(Duration::from_secs(4)));
    }
    if let Err(e) = player.light.terminate() {
        eprintln!("terminate failed: {e}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::builtin;

    #[test]
    fn priorities() {
        let now = Instant::now();
        let later = now + Duration::from_secs(1);
        assert_eq!(decide(None, false, now), Want::Idle);
        assert_eq!(decide(None, true, now), Want::Npu);
        assert_eq!(decide(Some((2, later)), true, now), Want::Key(2));
        assert_eq!(decide(Some((2, now)), true, later), Want::Npu);
        assert_eq!(decide(Some((2, now)), false, later), Want::Idle);
    }

    #[test]
    fn key_durations() {
        let pulse = builtin("pulse").unwrap();
        assert_eq!(
            key_duration(&pulse, 0),
            Duration::from_millis(pulse.cycle_ms())
        );
        assert_eq!(
            key_duration(&builtin("comet").unwrap(), 0),
            Duration::from_millis(3000)
        );
        // one-shot effects are never cut short...
        assert_eq!(
            key_duration(&pulse, 10),
            Duration::from_millis(pulse.cycle_ms())
        );
        // ...but can be held longer
        assert_eq!(key_duration(&pulse, 9000), Duration::from_millis(9000));
        let circle = builtin("circle").unwrap();
        assert_eq!(
            key_duration(&circle, 3000),
            Duration::from_millis(circle.cycle_ms())
        );
        let comet = builtin("comet").unwrap();
        assert_eq!(key_duration(&comet, 1234), Duration::from_millis(1234));
    }

    #[test]
    fn npu_effect_always_loops() {
        assert!(npu_effect(builtin("circle").unwrap()).looping);
        assert!(npu_effect(builtin("twinkle").unwrap()).looping);
    }
}
