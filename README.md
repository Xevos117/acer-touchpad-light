# acer-touchpad-light

[![CI](https://github.com/Xevos117/acer-touchpad-light/actions/workflows/ci.yml/badge.svg)](https://github.com/Xevos117/acer-touchpad-light/actions/workflows/ci.yml)

Linux control of the touchpad **"AI lighting"** LEDs of the **Acer Swift 14 AI
(SF14-51)**, plus a small daemon that reproduces what the Acer Light Service does
on Windows: the touchpad lights up while the **NPU is busy** and when you press
the **Copilot key** (or any other key combination you configure).

> Not affiliated with, endorsed by or supported by Acer. "Acer", "Swift" and
> "Copilot" are trademarks of their respective owners. Use at your own risk.

## Features

- Single binary, `tplight`, written in Rust; no runtime dependencies.
- Talks directly to the touchpad over hidraw (feature report `0x02` only).
- Built-in effects designed from scratch: `steady`, `breath`, `blink`,
  `circle` (alias `spin`), `comet`, `pulse`, `twinkle`, `wave`.
- Custom effects defined as frames in a TOML file.
- Daemon (`tplight daemon`):
  - **NPU activity**: watches the `intel_vpu` busy counter and plays an effect
    while the NPU is working;
  - **key combos**: plays an effect when a combination is pressed (Copilot key
    = `Meta + Shift + F23` by default). Keyboards are read via evdev and never
    grabbed;
  - priority: key effect > NPU effect > off;
  - always stops the running effect when it is no longer needed and on
    shutdown (SIGTERM/SIGINT);
  - survives suspend/resume and driver rebinds (reopens the device, re-applies
    the state), re-enables lighting if it was disabled.
- systemd unit, optional udev rule, install script.

## Supported hardware

| Laptop                     | Touchpad                     | Status |
|----------------------------|------------------------------|--------|
| Acer Swift 14 AI (SF14-51) | PixArt PIXA3838 (`093A:3838`) | developed on Ubuntu 26.04, kernel 7.0 |

Other Acer models with the same touchpad lighting may work: `tplight` also
accepts any hidraw device exposing the same vendor collection (`0xFF58/0x05`).
Reports are welcome.

The NPU integration requires the `intel_vpu` driver
(`/sys/class/accel/accel*/device/npu_busy_time_us`).

## Install

Requires a Rust toolchain (<https://rustup.rs>).

```sh
git clone https://github.com/Xevos117/acer-touchpad-light
cd acer-touchpad-light
./install.sh            # run as your normal user, it uses sudo for the install step
```

This builds the release binary as your user, then installs:

| File                                       | Purpose            |
|--------------------------------------------|--------------------|
| `/usr/local/bin/tplight`                   | the binary         |
| `/etc/acer-touchpad-light/config.toml`     | config (kept if it already exists) |
| `/etc/systemd/system/tplight.service`      | systemd unit, enabled and started |

`./install.sh --udev` additionally creates the group `tplight`, adds you to it
and installs a udev rule so the CLI works without `sudo` (the daemon runs as
root and does not need it).

Logs: `journalctl -u tplight -f`.

### Uninstall

```sh
./install.sh --uninstall           # keeps /etc/acer-touchpad-light
./install.sh --uninstall --purge   # removes the config too
```

Manual equivalent:

```sh
sudo systemctl disable --now tplight.service
sudo tplight stop
sudo rm -f /usr/local/bin/tplight /etc/systemd/system/tplight.service \
           /etc/udev/rules.d/70-acer-touchpad-light.rules
sudo rm -rf /etc/acer-touchpad-light
sudo systemctl daemon-reload
```

## Command line

```
tplight status                 device, lighting state (report 0x01), NPU counter
tplight on | off               enable / disable lighting (while off, effects are ignored)
tplight stop                   stop the running effect
tplight steady [-b 0..100]     all lamps on
tplight play <effect> [-b N]   play a built-in or custom effect
tplight list                   list effects
tplight raw L1 L2 L3 L4 D F    send one raw report 0x02 (hex, validated)
tplight keys                   print key names as you press them
tplight daemon [--config P] [--check]
```

All commands except `list`, `keys` and `daemon --check` need access to the
hidraw node (root, or the udev rule). `--device /dev/hidrawN` overrides
auto-detection. Looping effects keep running in the touchpad firmware until
`tplight stop`.

## Configuration

`/etc/acer-touchpad-light/config.toml` (full commented example in
[`config/config.toml`](config/config.toml)). Without a config file the daemon
uses the defaults shown here.

```toml
brightness = 100          # global scale, percent
reenable = true           # re-enable lighting if it was disabled

[npu]
enabled = true
effect = "breath"
poll_ms = 250             # how often the busy counter is read
idle_ms = 2000            # keep the effect until the NPU has been idle this long

[[key]]                   # Copilot key
keys = ["KEY_LEFTMETA", "KEY_LEFTSHIFT", "KEY_F23"]
effect = "pulse"
# duration_ms = 0         # 0: one playthrough (non-looping) or 3 s (looping)

[effects.heartbeat]       # custom effect: [l1, l2, l3, l4, ms] per frame
loop = true
frames = [[100,100,100,100,80], [20,20,20,20,120], [80,80,80,80,80], [0,0,0,0,700]]
```

- Key names are evdev names (`KEY_` prefix optional, case-insensitive). Use
  `sudo tplight keys` to see what your keys send.
- Lamps: L1 top-right, L2 bottom-right, L3 bottom-left, L4 top-left
  (probable layout). Levels `0..100`, frame duration `10..2550` ms, at most
  48 frames.
- Check a config with `tplight daemon --check --config FILE`, then
  `sudo systemctl restart tplight`.

## Protocol

Short version (details in [docs/PROTOCOL.md](docs/PROTOCOL.md)):

- feature report `0x02`: `02 L1 L2 L3 L4 D F` – four lamp levels (0..100),
  duration in 10 ms units, flags `0x02` first / `0x04` last / `0x08` loop /
  `0x10` status command (bit 0 = on/off);
- an effect is all its frames sent back-to-back; the firmware plays them;
- a new effect has to be sent twice while another is playing;
- feature report `0x01` (read-only): bit 0 = lighting enabled.

## Safety notes

- `tplight` only ever **writes report `0x02`**; this is enforced in the
  lowest-level HID function. The same device has firmware update/diagnostic
  reports that are never touched.
- Looping effects run in the touchpad firmware even after `tplight` exits.
  The daemon always sends *terminate* when an effect is no longer wanted and
  on shutdown; after manual experiments run `sudo tplight stop`.
- `tplight off` disables lighting entirely; the daemon re-enables it unless
  `reenable = false`.
- The udev rule gives the `tplight` group write access to the whole touchpad
  hidraw node; only install it on single-user machines you trust.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

## In italiano

`tplight` controlla i LED "AI lighting" del touchpad dell'Acer Swift 14 AI
(SF14-51) su Linux e, come il servizio Acer su Windows, accende il touchpad
quando la NPU lavora e quando premi il tasto Copilot (o altre combinazioni
configurabili).

- Installazione: `./install.sh` da utente normale (usa `sudo` solo per
  copiare i file). Disinstallazione: `./install.sh --uninstall`.
- Configurazione: `/etc/acer-touchpad-light/config.toml`, poi
  `sudo systemctl restart tplight`.
- Prove manuali: `sudo tplight play circle`, `sudo tplight stop`,
  `sudo tplight list`.
- Gli effetti in loop continuano a girare nel firmware del touchpad finché
  non si esegue `sudo tplight stop` (il demone lo fa da solo).
- Progetto non affiliato ad Acer.

## License

MIT – see [LICENSE](LICENSE).
