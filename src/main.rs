//! tplight - control the "AI lighting" LEDs of the Acer Swift 14 AI touchpad.

mod config;
mod daemon;
mod effects;
mod hid;
mod input;
mod protocol;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use crate::config::Config;
use crate::protocol::Light;

#[derive(Parser)]
#[command(
    name = "tplight",
    version,
    about = "Control the Acer Swift 14 AI touchpad lighting"
)]
struct Cli {
    /// hidraw node of the touchpad (auto-detected by default)
    #[arg(long, global = true, value_name = "PATH")]
    device: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show device, lighting state and NPU counter
    Status,
    /// Enable lighting (status command ON)
    On,
    /// Disable lighting (the device ignores effects until `on`)
    Off,
    /// Stop the running effect (lights off)
    Stop,
    /// All four lamps on, constant
    Steady {
        /// 0..100
        #[arg(short, long, default_value_t = 100, value_parser = clap::value_parser!(u8).range(0..=100))]
        brightness: u8,
    },
    /// Play a built-in or custom effect (looping effects run until `stop`)
    Play {
        effect: String,
        /// Brightness scale 0..100 (default: from config)
        #[arg(short, long, value_parser = clap::value_parser!(u8).range(0..=100))]
        brightness: Option<u8>,
        #[arg(long, value_name = "PATH", default_value = config::DEFAULT_PATH)]
        config: PathBuf,
    },
    /// List available effects
    List {
        #[arg(long, value_name = "PATH", default_value = config::DEFAULT_PATH)]
        config: PathBuf,
    },
    /// Send one raw report 0x02 payload: L1 L2 L3 L4 D F (6 hex bytes)
    Raw {
        #[arg(num_args = 1.., required = true)]
        bytes: Vec<String>,
        /// Send frames only once (by default non-command frames are sent twice,
        /// because the first upload only interrupts a running effect)
        #[arg(long)]
        once: bool,
    },
    /// Print key names as they are pressed (helps writing [[key]] combos)
    Keys,
    /// Run the background service (NPU activity + key combos)
    Daemon {
        #[arg(long, value_name = "PATH", default_value = config::DEFAULT_PATH)]
        config: PathBuf,
        /// Only validate the configuration and exit
        #[arg(long)]
        check: bool,
    },
}

fn parse_hex(parts: &[String]) -> Result<Vec<u8>, String> {
    let joined = parts.join(" ").replace([':', ','], " ");
    let mut out = Vec::new();
    for tok in joined.split_whitespace() {
        let tok = tok.trim_start_matches("0x").trim_start_matches("0X");
        if tok.len() > 2 && tok.len() % 2 == 0 && !tok.contains(' ') {
            // allow "6464646464 0e"-style concatenated bytes
            for i in (0..tok.len()).step_by(2) {
                out.push(
                    u8::from_str_radix(&tok[i..i + 2], 16)
                        .map_err(|_| format!("bad hex '{tok}'"))?,
                );
            }
        } else {
            out.push(u8::from_str_radix(tok, 16).map_err(|_| format!("bad hex byte '{tok}'"))?);
        }
    }
    Ok(out)
}

fn hex(b: &[u8]) -> String {
    b.iter()
        .map(|x| format!("{x:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn status(device: Option<PathBuf>) -> Result<(), String> {
    match Light::open(device) {
        Ok(mut l) => {
            println!("device:   {}", l.path().unwrap().display());
            match l.state_report() {
                Ok(p) => println!(
                    "lighting: {} (report 0x01 = {})",
                    if p[0] & 1 != 0 { "enabled" } else { "disabled" },
                    hex(&p)
                ),
                Err(e) => println!("lighting: unknown (cannot read report 0x01: {e})"),
            }
        }
        Err(e) => println!("device:   not available: {e}"),
    }
    let mut found = false;
    if let Ok(rd) = std::fs::read_dir("/sys/class/accel") {
        for e in rd.flatten() {
            let p = e.path().join("device/npu_busy_time_us");
            if let Ok(v) = std::fs::read_to_string(&p) {
                println!("npu:      {} = {} us", p.display(), v.trim());
                found = true;
            }
        }
    }
    if !found {
        println!("npu:      busy counter not found (intel_vpu not loaded?)");
    }
    Ok(())
}

fn keys() -> Result<(), String> {
    let w = input::KeyWatcher::new(
        |d| {
            d.supported_keys()
                .is_some_and(|k| k.iter().next().is_some())
        },
        |dev, key, value, held| {
            if value == 1 {
                let mut h: Vec<String> = held.iter().map(|k| format!("{k:?}")).collect();
                h.sort();
                println!("{dev}: {key:?} pressed   (held: {})", h.join(" + "));
            }
        },
    );
    if w.rescan() == 0 {
        return Err(
            "no readable input devices (run as root or as a member of group 'input')".into(),
        );
    }
    println!("press keys, Ctrl+C to quit");
    loop {
        std::thread::sleep(std::time::Duration::from_secs(5));
        w.rescan();
    }
}

fn run(cli: Cli) -> Result<(), String> {
    let io = |e: std::io::Error| e.to_string();
    match cli.cmd {
        Cmd::Status => status(cli.device),
        Cmd::On => Light::open(cli.device)
            .and_then(|mut l| l.enable())
            .map_err(io),
        Cmd::Off => Light::open(cli.device)
            .and_then(|mut l| l.disable())
            .map_err(io),
        Cmd::Stop => Light::open(cli.device)
            .and_then(|mut l| l.terminate())
            .map_err(io),
        Cmd::Steady { brightness } => Light::open(cli.device)
            .and_then(|mut l| l.play(&effects::steady_at(brightness), 100))
            .map_err(io),
        Cmd::Play {
            effect,
            brightness,
            config,
        } => {
            let cfg = Config::load(&config)?;
            let e = cfg
                .library()?
                .get(&effect)
                .ok_or_else(|| format!("unknown effect '{effect}' (see `tplight list`)"))?;
            let mut l = Light::open(cli.device.or(cfg.device.clone())).map_err(io)?;
            l.play(&e, brightness.unwrap_or(cfg.brightness))
                .map_err(io)?;
            if e.looping {
                println!("'{}' is looping; run `tplight stop` to end it", e.name);
            }
            Ok(())
        }
        Cmd::List { config } => {
            let cfg = Config::load(&config)?;
            for (e, custom) in cfg.library()?.all() {
                println!(
                    "{:<10} {:>2} frames  {:>5} ms  {:<8} {}{}",
                    e.name,
                    e.frames.len(),
                    e.cycle_ms(),
                    if e.looping { "loop" } else { "one-shot" },
                    e.description,
                    if custom { "  [custom]" } else { "" }
                );
            }
            Ok(())
        }
        Cmd::Raw { bytes, once } => {
            let payload = parse_hex(&bytes)?;
            let report = protocol::validate_raw_payload(&payload)?;
            let mut l = Light::open(cli.device).map_err(io)?;
            if once || protocol::is_command(&report) {
                l.command(report).map_err(io)?;
            } else {
                l.send_effect_reports(&[report]).map_err(io)?;
            }
            println!("sent: {}", hex(&report));
            Ok(())
        }
        Cmd::Keys => keys(),
        Cmd::Daemon { config, check } => {
            let mut cfg = Config::load(&config)?;
            if check {
                println!("configuration OK");
                return Ok(());
            }
            if cli.device.is_some() {
                cfg.device = cli.device;
            }
            daemon::run(cfg)
        }
    }
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("tplight: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_parsing() {
        let v = |s: &[&str]| parse_hex(&s.iter().map(|x| x.to_string()).collect::<Vec<_>>());
        assert_eq!(
            v(&["64", "0", "0", "0", "0a", "0e"]).unwrap(),
            vec![100, 0, 0, 0, 10, 14]
        );
        assert_eq!(
            v(&["64 00 00 00 0a 0e"]).unwrap(),
            vec![100, 0, 0, 0, 10, 14]
        );
        assert_eq!(v(&["640000000a0e"]).unwrap(), vec![100, 0, 0, 0, 10, 14]);
        assert_eq!(v(&["0x64,0x00"]).unwrap(), vec![100, 0]);
        assert!(v(&["zz"]).is_err());
    }

    #[test]
    fn cli_parses() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }
}
