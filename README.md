# acer-touchpad-light

[![CI](https://github.com/Xevos117/acer-touchpad-light/actions/workflows/ci.yml/badge.svg)](https://github.com/Xevos117/acer-touchpad-light/actions/workflows/ci.yml)

Linux control of the touchpad "AI lighting" LEDs of the Acer Swift 14 AI
(SF14-51), plus a small daemon that does what the Acer Light Service does on
Windows: the touchpad lights up while the NPU is busy and when you press the
Copilot key (or any other key combination you configure).

> Not affiliated with, endorsed by or supported by Acer. "Acer", "Swift" and
> "Copilot" are trademarks of their respective owners. Use at your own risk.

## Features

`tplight` is a single Rust binary with no runtime dependencies. It talks to the
touchpad over hidraw and only ever writes feature report `0x02`.

The driver itself knows nothing about animations: it sends frames of four lamp
levels. Effects are data. A few built-in effects are included as defaults
(`steady`, `breath`, `blink`, `circle`, `spin`, `comet`, `pulse`, `twinkle`,
`wave`), and you can add your own in the config file without touching the code.
You can also import the original Windows animations from your own Windows
installation.

The daemon (`tplight daemon`):

- watches the `intel_vpu` busy counter and repeats an effect while the NPU is
  working (`twinkle` by default);
- plays an effect when a key combination is pressed (`circle` on the Copilot
  key, which Linux sees as `Meta + Shift + F23`). Keyboards are read via evdev
  and never grabbed;
- gives the key effect priority over the NPU effect;
- never cuts an animation short: one-shot effects play to the end, and a
  looping effect is stopped in its dark pause (or faded out if it has none);
- turns the light off on shutdown, survives suspend/resume and driver rebinds,
  and turns lighting back on if it was disabled.

A systemd unit, an optional udev rule and an install script are included.

## Supported hardware

| Laptop                     | Touchpad                     | Status |
|----------------------------|------------------------------|--------|
| Acer Swift 14 AI (SF14-51) | PixArt PIXA3838 (`093A:3838`) | developed on Ubuntu 26.04, kernel 7.0 |

Other Acer models with the same touchpad lighting may work: `tplight` also
accepts any hidraw device with the same vendor collection (`0xFF58/0x05`).
Reports are welcome.

The NPU part needs the `intel_vpu` driver
(`/sys/class/accel/accel*/device/npu_busy_time_us`).

## Install

You need a Rust toolchain (<https://rustup.rs>).

```sh
git clone https://github.com/Xevos117/acer-touchpad-light
cd acer-touchpad-light
./install.sh            # run as your normal user; it uses sudo for the install step
```

This builds the release binary as your user, then installs:

| File                                       | Purpose            |
|--------------------------------------------|--------------------|
| `/usr/local/bin/tplight`                   | the binary         |
| `/etc/acer-touchpad-light/config.toml`     | config (an existing one is kept) |
| `/etc/systemd/system/tplight.service`      | systemd unit, enabled and started |

`./install.sh --udev` also creates the group `tplight`, adds you to it and
installs a udev rule, so the CLI works without `sudo`. The daemon runs as root
and does not need it.

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
tplight import-acer DIR [--out FILE [--force] [--activate]]
                               convert the Windows animations into custom effects
tplight daemon [--config P] [--check]
```

All commands except `list`, `keys`, `import-acer` and `daemon --check` need
access to the hidraw node (root, or the udev rule). `--device /dev/hidrawN`
overrides auto-detection. Looping effects keep running in the touchpad
firmware until `tplight stop`.

## Configuration

The config file is `/etc/acer-touchpad-light/config.toml`. A full commented
example is in [`config/config.toml`](config/config.toml). Without a config
file the daemon uses the defaults shown here.

```toml
brightness = 100          # global scale, percent
reenable = true           # turn lighting back on if it was disabled

[npu]
enabled = true
effect = "twinkle"        # repeated while the NPU is busy
poll_ms = 250             # how often the busy counter is read
idle_ms = 2000            # the NPU counts as idle after this long without activity

[[key]]                   # Copilot key
keys = ["KEY_LEFTMETA", "KEY_LEFTSHIFT", "KEY_F23"]
effect = "circle"
# duration_ms = 3000      # only for looping effects; one-shot effects play to the end
```

Key names are evdev names. The `KEY_` prefix is optional and case does not
matter. Run `sudo tplight keys` to see what your keys send.

Check a config with `tplight daemon --check --config FILE`, then run
`sudo systemctl restart tplight`.

## Writing your own effects

An effect is a list of frames. Each frame is `[l1, l2, l3, l4, ms]`: the
brightness of the four lamps (0 to 100) and how long the frame is shown
(10 to 2550 ms, in steps of 10 ms). An effect can have at most 48 frames.
The whole effect is uploaded to the touchpad, which then plays it by itself.

With `loop = true` the effect repeats until something else is played or it is
stopped. With `loop = false` (the default) it plays once and the lamps keep
the levels of the last frame, so a one-shot effect should normally end with
all lamps at 0.

The lamps sit around the touchpad in this order (taken from the Windows
"circle" animation):

```
        L2 ----> L4
        ^         |
        |         v
        L1 <---- L3
```

To make light go around the touchpad, light L2, L4, L3, L1 in turn. L2 + L3
and L4 + L1 are opposite pairs. The sketch only shows the order: which
physical corner each lamp is in, and whether the order is clockwise, has not
been checked.

Add effects under `[effects.<name>]`:

```toml
# one lamp at a time going around, repeated
[effects.chase]
loop = true
frames = [
  [  0, 100,   0,   0, 150],   # L2
  [  0,   0,   0, 100, 150],   # L4
  [  0,   0, 100,   0, 150],   # L3
  [100,   0,   0,   0, 150],   # L1
]

# short flash of all lamps, then off (one-shot)
[effects.flash]
frames = [
  [100, 100, 100, 100, 120],
  [ 40,  40,  40,  40,  80],
  [  0,   0,   0,   0,  10],
]

# the two opposite pairs take turns
[effects.pairs]
description = "L2+L3, then L4+L1"
loop = true
frames = [
  [  0, 100, 100,   0, 400],
  [100,   0,   0, 100, 400],
]
```

Use an effect by its name, for the NPU or for any key combination:

```toml
[npu]
effect = "chase"

[[key]]
keys = ["KEY_LEFTMETA", "KEY_LEFTSHIFT", "KEY_F23"]
effect = "flash"

[[key]]
keys = ["KEY_LEFTCTRL", "KEY_LEFTALT", "KEY_SPACE"]
effect = "pairs"
duration_ms = 5000
```

The NPU effect is always repeated while the NPU is busy, even if it is a
one-shot effect. A custom effect with the same name as a built-in one replaces
it. Try an effect with `sudo tplight play <name>` and stop it with
`sudo tplight stop`.

## The original Windows animations

The built-in `circle` and `twinkle` effects are written from scratch to look
like the Windows ones. To get the exact Windows animations, copy them from
your own Windows installation. They are in

```
C:\Windows\System32\DriverStore\FileRepository\acerlightservicesoftwarecomponent.inf_amd64_*\Frames
```

Copy that `Frames` folder to Linux, then:

```sh
tplight import-acer ~/Frames                 # print the effects as TOML
sudo tplight import-acer ~/Frames --out /etc/acer-touchpad-light/config.toml --activate
sudo systemctl restart tplight
```

Each animation becomes a custom effect: `Circle_R` is `acer-circle` and
`Twinkle_R` is `acer-twinkle`. `--out` adds them to the config file, keeps
everything else in it, and saves the old file as `config.toml.bak`. It
refuses to overwrite effects that are already there unless you add `--force`.
`--activate` also sets the NPU effect to `acer-twinkle` and the Copilot key
effect to `acer-circle`, like on Windows.

These files belong to Acer and are not included in this project. Do not
redistribute them.

## Protocol

Short version (details in [docs/PROTOCOL.md](docs/PROTOCOL.md)):

- feature report `0x02` is `02 L1 L2 L3 L4 D F`: four lamp levels (0..100),
  duration in 10 ms units, flags `0x02` first / `0x04` last / `0x08` loop /
  `0x10` status command (bit 0 = on/off);
- an effect is all its frames sent back to back, and the firmware plays them;
- a new effect has to be sent twice while another one is playing;
- feature report `0x01` (read-only): bit 0 = lighting enabled.

## Safety notes

`tplight` only ever writes report `0x02`. This is enforced in the lowest-level
HID function. The same device has firmware update and diagnostic reports that
are never touched.

Looping effects run in the touchpad firmware even after `tplight` exits. The
daemon always stops its effect when it is no longer wanted and on shutdown.
After manual tests, run `sudo tplight stop`.

`tplight off` disables lighting completely. The daemon turns it back on unless
`reenable = false`.

The udev rule gives the `tplight` group write access to the whole touchpad
hidraw node. Only install it on single-user machines you trust.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

## In italiano

`tplight` controlla i LED "AI lighting" del touchpad dell'Acer Swift 14 AI
(SF14-51) su Linux. Come il servizio Acer su Windows, accende il touchpad
mentre la NPU lavora e quando premi il tasto Copilot (o altre combinazioni
di tasti che scegli tu).

Si installa con `./install.sh` da utente normale (usa `sudo` solo per copiare
i file) e si rimuove con `./install.sh --uninstall`. La configurazione sta in
`/etc/acer-touchpad-light/config.toml`; dopo averla modificata esegui
`sudo systemctl restart tplight`.

Gli effetti sono solo dati: puoi crearne di nuovi nel file di configurazione
(vedi "Writing your own effects"). Per avere le animazioni identiche a Windows
copia la cartella `Frames` dalla tua installazione di Windows e usa
`sudo tplight import-acer <cartella> --out /etc/acer-touchpad-light/config.toml --activate`.
Quei file sono di Acer e non vanno ridistribuiti.

Per provare a mano: `sudo tplight play circle`, `sudo tplight list`,
`sudo tplight stop`. Gli effetti in loop continuano nel firmware del touchpad
finché non esegui `sudo tplight stop` (il demone lo fa da solo).

Progetto non affiliato ad Acer.

## License

MIT, see [LICENSE](LICENSE).
