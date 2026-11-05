//! Import of the animation files used by the Acer lighting service on Windows.
//!
//! The service keeps its animations in a `Frames` folder: `<Name>.ini` holds
//! `[Frame] FrameAmount`, `[Speed] fps` and `[Duration] LifeTime`, and each
//! frame is a separate `<Name>_<NN>.ini` with `[lamp1]`..`[lamp4]`
//! `brightness = 0..100`. The files are UTF-16 or UTF-8 (often with a BOM) and
//! use CRLF line endings. Nothing from those files is shipped with this
//! project: users convert the copy from their own Windows installation.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::effects::Effect;
use crate::protocol::{Frame, MAX_FRAMES, MAX_LEVEL};

/// Files larger than this are not animation files.
const MAX_FILE_LEN: u64 = 64 * 1024;

/// One converted animation.
#[derive(Debug, Clone, PartialEq)]
pub struct Imported {
    /// Name of the source set, e.g. `Circle_R`.
    pub source: String,
    pub fps: u32,
    pub effect: Effect,
}

type Ini = BTreeMap<String, BTreeMap<String, String>>;

/// Minimal INI parser: sections and keys are case-insensitive, `;`/`#` start
/// a comment line. Handles a UTF-8 BOM and CRLF line endings.
pub fn parse_ini(text: &str) -> Ini {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut ini = Ini::new();
    let mut section = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = name.trim().to_ascii_lowercase();
        } else if let Some((k, v)) = line.split_once('=') {
            ini.entry(section.clone())
                .or_default()
                .insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    ini
}

/// Reads a small text file, accepting UTF-8 (with or without BOM) and UTF-16LE.
fn read_text(path: &Path) -> Result<String, String> {
    let err = |e: std::io::Error| format!("{}: {e}", path.display());
    let len = fs::metadata(path).map_err(err)?.len();
    if len > MAX_FILE_LEN {
        return Err(format!("{}: file too large", path.display()));
    }
    let bytes = fs::read(path).map_err(err)?;
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let units: Vec<u16> = rest
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        return String::from_utf16(&units).map_err(|_| format!("{}: bad UTF-16", path.display()));
    }
    String::from_utf8(bytes).map_err(|_| format!("{}: not UTF-8 text", path.display()))
}

fn get<'a>(ini: &'a Ini, section: &str, key: &str) -> Option<&'a str> {
    ini.get(section)?.get(key).map(String::as_str)
}

fn get_num(ini: &Ini, section: &str, key: &str, file: &Path) -> Result<u32, String> {
    let v = get(ini, section, key)
        .ok_or_else(|| format!("{}: missing [{section}] {key}", file.display()))?;
    v.parse().map_err(|_| {
        format!(
            "{}: [{section}] {key} = '{v}' is not a number",
            file.display()
        )
    })
}

/// Effect name for a set: `Circle_R` -> `acer-circle`.
pub fn effect_name(stem: &str) -> String {
    let lower = stem.to_ascii_lowercase();
    let base = lower.strip_suffix("_r").unwrap_or(&lower);
    let clean: String = base
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    format!("acer-{}", clean.trim_matches('-'))
}

/// Frame duration for a frame rate: D = round(1000 / fps / 10), in ms.
pub fn frame_ms(fps: u32) -> Result<u32, String> {
    if fps == 0 || fps > 100 {
        return Err(format!("fps = {fps} out of range (1..=100)"));
    }
    let d = (1000.0 / f64::from(fps) / 10.0).round().clamp(1.0, 255.0) as u32;
    Ok(d * 10)
}

/// Directory listing: lowercase file name -> path.
fn listing(dir: &Path) -> Result<BTreeMap<String, PathBuf>, String> {
    let rd = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(rd
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            Some((name.to_ascii_lowercase(), e.path()))
        })
        .collect())
}

fn load_set(
    files: &BTreeMap<String, PathBuf>,
    stem: &str,
    header: &Path,
    ini: &Ini,
) -> Result<Imported, String> {
    let count = get_num(ini, "frame", "frameamount", header)? as usize;
    let fps = get_num(ini, "speed", "fps", header)?;
    if count == 0 || count > MAX_FRAMES {
        return Err(format!(
            "{}: FrameAmount = {count}, the touchpad supports 1..={MAX_FRAMES} frames",
            header.display()
        ));
    }
    let ms = frame_ms(fps).map_err(|e| format!("{}: {e}", header.display()))?;
    let lower = stem.to_ascii_lowercase();
    let mut frames = Vec::with_capacity(count);
    for i in 1..=count {
        // the service uses "%s_%02d.ini"; accept "<Name>_<n>.ini" too
        let path = [format!("{lower}_{i:02}.ini"), format!("{lower}_{i}.ini")]
            .iter()
            .find_map(|n| files.get(n))
            .ok_or_else(|| {
                format!(
                    "{}: frame {i} of {count} is missing ({stem}_{i:02}.ini)",
                    header.display()
                )
            })?;
        let f = parse_ini(&read_text(path)?);
        let mut levels = [0u8; 4];
        for (k, l) in levels.iter_mut().enumerate() {
            let v = get_num(&f, &format!("lamp{}", k + 1), "brightness", path)?;
            if v > u32::from(MAX_LEVEL) {
                return Err(format!(
                    "{}: lamp{} brightness {v} > {MAX_LEVEL}",
                    path.display(),
                    k + 1
                ));
            }
            *l = v as u8;
        }
        frames.push(Frame::new(levels, ms));
    }
    let effect = Effect {
        name: effect_name(stem),
        description: format!("{stem} from the Acer Light Service ({fps} fps)"),
        looping: false,
        frames,
    };
    effect.validate()?;
    Ok(Imported {
        source: stem.to_string(),
        fps,
        effect,
    })
}

/// Converts every animation set (`<Name>.ini` with a `[Frame]` section) in `dir`.
pub fn import_dir(dir: &Path) -> Result<Vec<Imported>, String> {
    let files = listing(dir)?;
    let mut out: Vec<Imported> = Vec::new();
    for path in files.values() {
        let Some(stem) = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(".ini").or_else(|| n.strip_suffix(".INI")))
        else {
            continue;
        };
        // only keep names that are safe to put in a TOML string
        if stem.is_empty()
            || !stem
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            continue;
        }
        let ini = parse_ini(&read_text(path)?);
        if !ini.contains_key("frame") {
            continue; // a frame file, or something else
        }
        let imp = load_set(&files, stem, path, &ini)?;
        if out.iter().any(|o| o.effect.name == imp.effect.name) {
            return Err(format!(
                "two animation sets map to the same name '{}'",
                imp.effect.name
            ));
        }
        out.push(imp);
    }
    if out.is_empty() {
        return Err(format!(
            "{}: no animation sets found (expected e.g. Circle_R.ini and Circle_R_01.ini ...)",
            dir.display()
        ));
    }
    Ok(out)
}

/// TOML snippet with `[effects.<name>]` tables for the imported animations.
pub fn to_toml(imported: &[Imported], dir: &Path) -> String {
    let mut s = format!(
        "\n# Imported by `tplight import-acer` from {}.\n\
         # Animation data of the Acer Light Service (Acer's copyright): for personal use,\n\
         # do not redistribute.\n",
        dir.display()
    );
    for imp in imported {
        let e = &imp.effect;
        s += &format!(
            "\n[effects.{}]\ndescription = \"{}\"\nloop = {}\nframes = [\n",
            e.name, e.description, e.looping
        );
        for f in &e.frames {
            let [a, b, c, d] = f.levels;
            s += &format!("  [{a:>3}, {b:>3}, {c:>3}, {d:>3}, {}],\n", f.ms);
        }
        s += "]\n";
    }
    s
}

/// Merges imported effects into the text of a config file.
///
/// Existing content is kept as it is; the new tables are appended. Effects
/// with the same name are an error unless `force` is set, in which case the
/// old definitions are removed. With `activate`, `[npu] effect` is set to
/// `acer-twinkle` and the Copilot key binding (a combo containing F23) to
/// `acer-circle`, when those effects were imported.
/// Returns the new text and a list of notes for the user.
pub fn merge(
    existing: &str,
    imported: &[Imported],
    snippet: &str,
    force: bool,
    activate: bool,
) -> Result<(String, Vec<String>), String> {
    use toml_edit::{Array, DocumentMut, Item, Table, value};

    let mut doc: DocumentMut = existing
        .parse()
        .map_err(|e| format!("existing config is not valid TOML: {e}"))?;
    let mut notes = Vec::new();

    let names: Vec<&str> = imported.iter().map(|i| i.effect.name.as_str()).collect();
    if let Some(effects) = doc.get_mut("effects").and_then(Item::as_table_like_mut) {
        let clashes: Vec<&str> = names
            .iter()
            .copied()
            .filter(|n| effects.contains_key(n))
            .collect();
        if !clashes.is_empty() && !force {
            return Err(format!(
                "effects already defined: {} (use --force to replace them)",
                clashes.join(", ")
            ));
        }
        for n in clashes {
            effects.remove(n);
            notes.push(format!("replaced existing effect '{n}'"));
        }
    }

    if activate {
        let has = |n: &str| names.contains(&n);
        if has("acer-twinkle") {
            let npu = doc.entry("npu").or_insert(Item::Table(Table::new()));
            let npu = npu
                .as_table_like_mut()
                .ok_or("[npu] is not a table in the existing config")?;
            npu.insert("effect", value("acer-twinkle"));
            notes.push("[npu] effect = \"acer-twinkle\"".into());
        }
        if has("acer-circle") {
            let is_copilot = |t: &Table| {
                t.get("keys").and_then(Item::as_array).is_some_and(|a| {
                    a.iter().any(|k| {
                        k.as_str()
                            .is_some_and(|k| k.to_ascii_uppercase().ends_with("F23"))
                    })
                })
            };
            match doc.get_mut("key") {
                None => {
                    // no [[key]] at all: the built-in default binding is used, make it explicit
                    let mut t = Table::new();
                    let mut keys = Array::new();
                    for k in ["KEY_LEFTMETA", "KEY_LEFTSHIFT", "KEY_F23"] {
                        keys.push(k);
                    }
                    t.insert("keys", value(keys));
                    t.insert("effect", value("acer-circle"));
                    let mut a = toml_edit::ArrayOfTables::new();
                    a.push(t);
                    doc.insert("key", Item::ArrayOfTables(a));
                    notes.push("added Copilot [[key]] with effect \"acer-circle\"".into());
                }
                Some(item) => {
                    let a = item.as_array_of_tables_mut().ok_or(
                        "'key' is not an array of tables ([[key]]) in the existing config",
                    )?;
                    let mut found = false;
                    for t in a.iter_mut().filter(|t| is_copilot(t)) {
                        t.insert("effect", value("acer-circle"));
                        t.remove("duration_ms");
                        found = true;
                    }
                    notes.push(if found {
                        "Copilot [[key]] effect = \"acer-circle\"".into()
                    } else {
                        "no [[key]] with KEY_F23 found, key bindings left unchanged".into()
                    });
                }
            }
        }
    }

    let mut text = doc.to_string();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text += snippet;
    Config::parse(&text).map_err(|e| format!("merged config would be invalid: {e}"))?;
    Ok((text, notes))
}

/// Writes `text` to `path` atomically, keeping a `.bak` copy of the old file
/// and its permissions.
pub fn write_config(path: &Path, text: &str) -> Result<(), String> {
    let err = |e: std::io::Error| format!("{}: {e}", path.display());
    let tmp = path.with_extension("toml.tplight-tmp");
    fs::write(&tmp, text).map_err(err)?;
    if let Ok(meta) = fs::metadata(path) {
        fs::set_permissions(&tmp, meta.permissions()).map_err(err)?;
        fs::copy(path, path.with_extension("toml.bak")).map_err(err)?;
    }
    fs::rename(&tmp, path).map_err(err)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temporary folder with a small synthetic animation set (not vendor data).
    fn fixture(name: &str, frames: &[[u8; 4]], skip: Option<usize>) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tplight-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let header = format!(
            "\u{feff}[DeviceType]\r\nDeviceType = 1\r\n\r\n[Frame]\r\nFrameAmount = {}\r\n\r\n\
             [Duration]\r\nLifeTime = 1\r\n\r\n[Speed]\r\nfps = 20\r\n",
            frames.len()
        );
        fs::write(dir.join("Test_R.ini"), header).unwrap();
        for (i, f) in frames.iter().enumerate() {
            if skip == Some(i + 1) {
                continue;
            }
            let body: String = f
                .iter()
                .enumerate()
                .map(|(k, v)| format!("[lamp{}]\r\nbrightness = {v}\r\n", k + 1))
                .collect();
            fs::write(
                dir.join(format!("Test_R_{:02}.ini", i + 1)),
                format!("\u{feff}{body}"),
            )
            .unwrap();
        }
        fs::write(dir.join("Other.json"), "{}").unwrap();
        dir
    }

    const FRAMES: [[u8; 4]; 3] = [[0, 0, 0, 0], [10, 50, 0, 100], [0, 0, 0, 0]];

    #[test]
    fn ini_parsing() {
        let ini =
            parse_ini("\u{feff}[Frame]\r\nFrameAmount = 7\r\n; note\r\n[Speed]\r\n fps=12 \r\n");
        assert_eq!(get(&ini, "frame", "frameamount"), Some("7"));
        assert_eq!(get(&ini, "speed", "fps"), Some("12"));
    }

    #[test]
    fn names_and_timing() {
        assert_eq!(effect_name("Circle_R"), "acer-circle");
        assert_eq!(effect_name("Twinkle_R"), "acer-twinkle");
        assert_eq!(effect_name("My Effect"), "acer-my-effect");
        assert_eq!(frame_ms(14).unwrap(), 70);
        assert_eq!(frame_ms(12).unwrap(), 80);
        assert_eq!(frame_ms(1).unwrap(), 1000);
        assert!(frame_ms(0).is_err());
    }

    #[test]
    fn imports_a_set() {
        let dir = fixture("ok", &FRAMES, None);
        let v = import_dir(&dir).unwrap();
        assert_eq!(v.len(), 1);
        let e = &v[0].effect;
        assert_eq!(e.name, "acer-test");
        assert!(!e.looping);
        assert_eq!(v[0].fps, 20);
        let got: Vec<[u8; 4]> = e.frames.iter().map(|f| f.levels).collect();
        assert_eq!(got, FRAMES);
        assert!(e.frames.iter().all(|f| f.ms == 50));

        let snippet = to_toml(&v, &dir);
        let cfg = Config::parse(&snippet).unwrap();
        assert_eq!(cfg.library().unwrap().get("acer-test").as_ref(), Some(e));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_frame_is_an_error() {
        let dir = fixture("missing", &FRAMES, Some(2));
        let err = import_dir(&dir).unwrap_err();
        assert!(err.contains("frame 2 of 3"), "{err}");
        fs::remove_dir_all(dir).unwrap();
        let empty = fixture("empty", &[], None);
        assert!(import_dir(&empty).is_err());
        fs::remove_dir_all(empty).unwrap();
    }

    fn imported(names: &[&str]) -> Vec<Imported> {
        names
            .iter()
            .map(|n| Imported {
                source: n.to_string(),
                fps: 10,
                effect: Effect {
                    name: n.to_string(),
                    description: "x".into(),
                    looping: false,
                    frames: vec![Frame::new([0, 100, 0, 0], 100), Frame::all(0, 100)],
                },
            })
            .collect()
    }

    #[test]
    fn merge_keeps_content_and_refuses_clashes() {
        let existing = "# my settings\nbrightness = 50\n\n[npu]\neffect = \"breath\" # keep\n\n\
                        [[key]]\nkeys = [\"KEY_LEFTMETA\", \"KEY_LEFTSHIFT\", \"KEY_F23\"]\n\
                        effect = \"circle\"\nduration_ms = 3000\n";
        let imp = imported(&["acer-circle", "acer-twinkle"]);
        let snippet = to_toml(&imp, Path::new("/x"));

        let (text, _) = merge(existing, &imp, &snippet, false, false).unwrap();
        assert!(text.starts_with(existing));
        let cfg = Config::parse(&text).unwrap();
        assert_eq!(cfg.brightness, 50);
        assert_eq!(cfg.npu.effect, "breath");

        // importing again clashes...
        let err = merge(&text, &imp, &snippet, false, false).unwrap_err();
        assert!(err.contains("--force"), "{err}");
        // ...unless forced, and --activate switches the defaults
        let (text2, notes) = merge(&text, &imp, &snippet, true, true).unwrap();
        assert!(text2.contains("# my settings"));
        let cfg = Config::parse(&text2).unwrap();
        assert_eq!(cfg.npu.effect, "acer-twinkle");
        assert_eq!(cfg.keys[0].effect, "acer-circle");
        assert_eq!(cfg.keys[0].duration_ms, 0);
        assert_eq!(cfg.effects.len(), 2);
        assert!(notes.iter().any(|n| n.contains("replaced")));
    }

    #[test]
    fn merge_into_empty_config_with_activate() {
        let imp = imported(&["acer-circle"]);
        let snippet = to_toml(&imp, Path::new("/x"));
        let (text, _) = merge("", &imp, &snippet, false, true).unwrap();
        let cfg = Config::parse(&text).unwrap();
        assert_eq!(cfg.keys.len(), 1);
        assert_eq!(cfg.keys[0].effect, "acer-circle");
        assert_eq!(cfg.npu.effect, "twinkle");
    }
}
