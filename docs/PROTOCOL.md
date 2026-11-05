# Touchpad lighting protocol (PixArt PIXA3838)

This document describes the HID protocol used to drive the four "AI lighting"
LEDs of the touchpad on the Acer Swift 14 AI (SF14-51). It was worked out for
interoperability purposes from the device's HID report descriptor, from the
behaviour of the vendor's Windows lighting service and by testing on real
hardware. No vendor code or data is included in this project.

Items marked *(probable)* have not been fully confirmed.

## Device

| Property        | Value                                                        |
|-----------------|--------------------------------------------------------------|
| Touchpad        | PixArt `PIXA3838`, connected over I²C (`i2c_hid_acpi`)       |
| HID_ID          | `0018:0000093A:00003838` (bus I²C, VID `093A`, PID `3838`)   |
| Linux interface | `/dev/hidrawN` (root-only by default)                        |
| Collection      | vendor Usage Page `0xFF58`, Usage `0x05` (`06 58 FF 09 05`)  |

`tplight` finds the node by matching the HID_ID and checking that the report
descriptor contains the vendor collection above.

All communication uses HID **feature reports** (`HIDIOCSFEATURE` /
`HIDIOCGFEATURE` ioctls on the hidraw node).

## Report 0x02: lighting frame or command (write-only)

7 bytes including the report ID:

```
02  L1  L2  L3  L4  D  F
```

| Byte  | Meaning                                                              |
|-------|----------------------------------------------------------------------|
| L1-L4 | brightness of the four lamps, `0..100`                               |
| D     | frame duration in 10 ms units, `1..255` (`max(1, ms / 10)`)          |
| F     | flags (bits 5-7 are padding and must be 0)                           |

Flags:

| Bit    | Meaning                                                    |
|--------|------------------------------------------------------------|
| `0x02` | first frame of an effect                                   |
| `0x04` | last frame of an effect                                    |
| `0x08` | loop (set on the last frame)                               |
| `0x10` | status command; bit 0 (`0x01`) = lighting on/off           |

Reading report 0x02 back returns 0 bytes (it is not readable).

### Lamp order

The vendor's "circle" animation (played when the Copilot key is pressed) lights
the lamps one after the other in this order and then turns them off in the
same order:

```
L2 -> L4 -> L3 -> L1 -> (back to L2)
```

So this is the order of the lamps around the touchpad. Lamps that are opposite
each other in this ring form two pairs, L2 + L3 and L4 + L1; the vendor's NPU
animation fades between these two pairs.

Which corner each lamp is in has not been checked, and neither has the
direction (clockwise or not). An earlier version of this document assumed
L1 -> L2 -> L3 -> L4, which does not match the vendor animation.

### Commands

| Command          | Bytes                  |
|------------------|------------------------|
| enable lighting  | `02 00 00 00 00 01 11` |
| disable lighting | `02 00 00 00 00 01 10` |
| terminate effect | `02 00 00 00 00 01 06` |

While disabled, the device ignores all frames until *enable* is sent again.

### Effects

An effect is a sequence of frames sent back-to-back without delays; the
firmware stores them and plays them on its own using each frame's `D` value.
The first frame carries `0x02`, the last one `0x04` (plus `0x08` to loop).
A single steady looping frame therefore uses flags `0x0E`.

The vendor software uses effects of up to 44 frames; `tplight` limits
effects to 48 frames.

**Quirk:** if an effect is already playing, the first upload of a new effect
only interrupts the running one (lights go off). The new effect must be sent
**twice in a row** to start. Status and terminate commands are sent once.

## How the vendor service uses it *(from static analysis)*

This was read from the disassembly of the Windows service component
(`AcerLightingComponent.exe`); it was not observed on Windows.

The animations are files in the service's `Frames` folder: `<Name>.ini` with
`[Frame] FrameAmount`, `[Speed] fps` and `[Duration] LifeTime`, plus one
`<Name>_NN.ini` per frame with `[lamp1]`..`[lamp4]` `brightness`.
Each frame lasts `1000 / fps` ms (integer division). `LifeTime` is read but
its value is not used when the touchpad effect is set.

- Copilot key: the touchpad plays `Circle_R` once (no loop flag). The
  animation starts and ends dark, so nothing is sent at the end. (The service
  also plays `Blink` on the Copilot key's own LED, which is a different
  device.)
- NPU: the service samples NPU utilization once per second. When it goes
  above its threshold, the touchpad plays `Twinkle_R` with the loop flag.
  The NPU state is checked again once per animation cycle
  (`FrameAmount * 1000 / fps` ms, about 3.4 s for `Twinkle_R`); when the NPU
  is idle at that point the service sends *terminate*. Since the animation
  ends with a few dark frames, the light simply stays off.
- After a Copilot key press while the NPU is busy, `Twinkle_R` is started
  again after about one cycle.

`tplight daemon` does the same with its built-in `circle` and `twinkle`
effects (or with the original files, see `tplight import-acer`). It stops a
looping effect in the middle of its dark pause, and fades out effects that
have no dark pause.

## Report 0x01: lighting state (read-only)

`GET_FEATURE` with a 3-byte buffer (ID + 2 bytes). Payload byte 0, bit 0 =
lighting enabled. `tplight status` shows it; the daemon uses it to re-enable
lighting when needed.

## Reports that must not be touched

The same collection exposes other reports (IDs `0x41`, `0x42`, `0x43`) used for
firmware update and diagnostics. Writing to them could damage the touchpad.
`tplight` refuses to write any report other than `0x02` at the lowest level of
its HID code.
