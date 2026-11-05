//! Keyboard monitoring via evdev (read-only, devices are never grabbed).

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;

use evdev::{Device, InputEventKind, Key};

/// Callback `(device_name, key, value, held_keys)`.
type Handler = dyn Fn(&str, Key, i32, &HashSet<Key>) + Send + Sync;

/// Keeps one reader thread per matching input device and spawns new ones on rescan
/// (hotplug). Threads end by themselves when their device disappears.
pub struct KeyWatcher {
    active: Arc<Mutex<HashSet<PathBuf>>>,
    filter: Box<dyn Fn(&Device) -> bool + Send>,
    handler: Arc<Handler>,
}

impl KeyWatcher {
    /// `filter` selects the devices to watch, `handler(device_name, key, value, held_keys)` is
    /// called for every key event (value 1 = press, 0 = release, 2 = autorepeat); `held_keys`
    /// are the keys currently held on that device, including the event's key on press.
    pub fn new(
        filter: impl Fn(&Device) -> bool + Send + 'static,
        handler: impl Fn(&str, Key, i32, &HashSet<Key>) + Send + Sync + 'static,
    ) -> Self {
        Self {
            active: Arc::default(),
            filter: Box::new(filter),
            handler: Arc::new(handler),
        }
    }

    /// Looks for new matching devices. Returns the number of newly watched devices.
    pub fn rescan(&self) -> usize {
        let mut added = 0;
        for (path, dev) in evdev::enumerate() {
            if self.active.lock().unwrap().contains(&path) || !(self.filter)(&dev) {
                continue;
            }
            let name = dev.name().unwrap_or("?").to_string();
            eprintln!("watching keys on {} ({name})", path.display());
            self.active.lock().unwrap().insert(path.clone());
            added += 1;
            let active = Arc::clone(&self.active);
            let handler = Arc::clone(&self.handler);
            thread::spawn(move || {
                read_loop(dev, &name, handler.as_ref());
                eprintln!("stopped watching {} ({name})", path.display());
                active.lock().unwrap().remove(&path);
            });
        }
        added
    }
}

fn read_loop(mut dev: Device, name: &str, handler: &Handler) {
    let mut held = HashSet::new();
    loop {
        let events = match dev.fetch_events() {
            Ok(ev) => ev,
            Err(_) => return,
        };
        for ev in events {
            if let InputEventKind::Key(k) = ev.kind() {
                match ev.value() {
                    0 => {
                        held.remove(&k);
                    }
                    1 => {
                        held.insert(k);
                    }
                    _ => {}
                }
                handler(name, k, ev.value(), &held);
            }
        }
    }
}

/// True when `key` was just pressed and completes `combo` (all combo keys held).
pub fn combo_triggered(combo: &[Key], key: Key, value: i32, held: &HashSet<Key>) -> bool {
    value == 1 && combo.contains(&key) && combo.iter().all(|k| held.contains(k))
}

/// Device filter: supports every key of at least one combo.
pub fn supports_any_combo(dev: &Device, combos: &[Vec<Key>]) -> bool {
    dev.supported_keys()
        .is_some_and(|s| combos.iter().any(|c| c.iter().all(|k| s.contains(*k))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combo_logic() {
        let combo = [Key::KEY_LEFTMETA, Key::KEY_LEFTSHIFT, Key::KEY_F23];
        let mut held = HashSet::new();
        held.insert(Key::KEY_LEFTMETA);
        held.insert(Key::KEY_LEFTSHIFT);
        assert!(!combo_triggered(&combo, Key::KEY_LEFTSHIFT, 1, &held));
        held.insert(Key::KEY_F23);
        assert!(combo_triggered(&combo, Key::KEY_F23, 1, &held));
        assert!(!combo_triggered(&combo, Key::KEY_F23, 2, &held)); // autorepeat
        assert!(!combo_triggered(&combo, Key::KEY_A, 1, &held));
    }
}
