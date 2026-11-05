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

## Report 0x02 – lighting frame / command (write-only)

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

### Lamp layout *(probable)*

The vendor software models the four lamps as a 2×2 matrix visited in circular
order:

| Lamp | Matrix position | Physical position *(probable)* |
|------|-----------------|--------------------------------|
| L1   | row 0, col 1    | top-right                      |
| L2   | row 1, col 1    | bottom-right                   |
| L3   | row 1, col 0    | bottom-left                    |
| L4   | row 0, col 0    | top-left                       |

So L1 → L2 → L3 → L4 is a clockwise walk around the touchpad.

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

The vendor software uses effects of up to about 44 frames; `tplight` limits
effects to 48 frames.

**Quirk:** if an effect is already playing, the first upload of a new effect
only interrupts the running one (lights go off). The new effect must be sent
**twice in a row** to start. Status and terminate commands are sent once.

## Report 0x01 – lighting state (read-only)

`GET_FEATURE` with a 3-byte buffer (ID + 2 bytes). Payload byte 0, bit 0 =
lighting enabled. `tplight status` shows it; the daemon uses it to re-enable
lighting when needed.

## Reports that must not be touched

The same collection exposes other reports (IDs `0x41`, `0x42`, `0x43`) used for
firmware update and diagnostics. Writing to them could damage the touchpad.
`tplight` refuses to write any report other than `0x02` at the lowest level of
its HID code.
