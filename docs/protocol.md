# Sound Blaster X7 HID protocol

## Known before capture — from `Sayrus/x7-bluetooth-control`

Source: <https://github.com/Sayrus/x7-bluetooth-control> (Python, ~150 lines).
Different transport — **Bluetooth RFCOMM / SPP**, not USB HID — and it only
implements mute and speaker/headphone switching. Appears to be reverse-engineered
from Creative's **Android app** (a comment notes a value "is actually
`Integer.MIN_VALUE` in Java"), not from a USB capture.

Treat everything in this section as *unverified over USB*. The working
hypothesis was that Creative wraps this same application-layer packet inside
the 64-byte HID feature reports; the USB capture below killed it.

### Packet framing

```
byte 0   0x5A (90)   STARTBYTEID
byte 1   command id
byte 2   payload length
byte 3+  payload
```

Payload byte 0 is a get/set discriminator: `0` = set, `1` = get.
Responses arrive with the same framing; a reply with `is_get == 1` carries the
current value.

### Command IDs seen

| ID | Hex | Name |
|----|-----|------|
| 2  | 0x02 | `ACK` |
| 38 | 0x26 | `HARDWARE_BUTTON_STATE` |
| 41 | 0x29 | `SPEAKER_CONFIGURATION` |

Sparse and low-numbered — likely one contiguous enum, so the unknown IDs between
and above these are the rest of the control surface (SBX, EQ, amp, ...).

### `HARDWARE_BUTTON_STATE` (0x26)

Set: payload `[0, button_id, state]`, state `1` = enabled, `0` = disabled.
Get reply: payload `[1, b0, b1, b2, b3]` — a 32-bit **little-endian** bitmask
where button `n` occupies bit `n - 1`.

Button IDs (only the first three are said to exist on the X7):

| ID | Button |
|----|--------|
| 1  | `SBX` |
| 8  | `MUTE` |
| 17 | `CRYSTAL_VOICE` |
| 4  | `VOICE` (not on X7) |
| 5  | `MICROPHONE` (not on X7) |
| 7  | `PHONE` (not on X7) |
| 9  | `NOISE_REDUCTION` (not on X7) |
| 10–16 | `BP_*` playback/record buttons (not on X7) |

The repo's own comment says capabilities should really be queried from the device
first rather than hardcoded — so there is probably a capabilities command we
haven't identified.

### `SPEAKER_CONFIGURATION` (0x29)

Set: payload `[0]` + a **32-bit little-endian signed** value.

| Value | Meaning |
|-------|---------|
| 1 | `HEADPHONES` |
| 2 | `STEREO_2_0` |
| 3 | `MULTI_CHANNEL_5_1` |
| `-2147483648` (`INT_MIN`) | toggle to speakers without knowing current config |

A commented-out `SpeakerAdvancedConfiguration` enum in the source hints at a
separate property with more layouts (2.0, 2.1, 3.0, 3.1, 4.0, 4.1, 5.0, 5.1
numbered 1–8) — presumably reached through a different command.

## USB findings (2026-10-10 Windows capture, sessions 1-3)

The 0x5A hypothesis is dead for USB. Only one 0x5A command appeared at app
launch (`5a 2c 01 02`). Everything else:

- Host -> device: 64-byte SET_REPORT on HID interface 4, wValue 0x0200, no
  GET_REPORT at all. Replies come back on **interrupt IN endpoint 0x86**,
  prefixed with `00` then an echo of the command. Device-initiated events use
  **endpoint 0x84** (e.g. `00 01 00 02 00 14` after an output switch).
- Output switch: `23 4e 01` = headphones, `23 4e 00 00 00 80` = speakers
  (INT_MIN, as in the Bluetooth notes). `23 4d` reads it back (01 / 02).
- SBX master: `23 23 01 00` off, `23 23 01 01` on; `23 24` reads it.
- Parameter get: `26 01 <group> <id>`, reply `00 26 01 00 <group> <id> <float32 BE>`.
  Group 0x96 = SBX, 0x95 = CrystalVoice/mic, 0x97 = ?
- Parameter set: `20 00 16 0a d5 02 08 <2*id> 20 96 <float32 BE>` (group 0x96).
  Note the set id is twice the get id.

| SBX param (group 0x96) | get id | set id | value |
|---|---|---|---|
| Surround enable | 00 | 00 | 0.0 / 1.0 |
| Surround level | 01 | 02 | 0.0-1.0 |
| Dialog Plus enable / level | 02 / 03 | 04 / 06 | 0/1, 0.0-1.0 |
| Smart Volume enable / level / mode | 04 / 05 / 06 | 08 / 0a / 0c | mode 0 Normal, 1 Loud, 2 Night |
| Crystalizer enable / level | 07 / 08 | 0e / 10 | |
| Bass crossover Hz | 17 | 2e | 10.0-500.0, default 80 |
| Bass enable / level | 18 / 19 | 30 / 32 | |

Sliders send one set per step during a drag (no debounce), ~23 ms apart.

## USB findings, session 4 (headphones / speakers / cinematic / scout / standby)

Simple toggles follow `23 <set-id> <value>` with a paired `23 <set-id - 1>` read:

| Control | set | read |
|---|---|---|
| Headphone gain | `23 45 01` high, `23 45 00` normal | `23 44` |
| Direct Mode | `23 43 01` / `23 43 00` | `23 42` |
| Direct Mode (SPDIF-In) | `23 4b 01` / `23 4b 00` | `23 4a` |
| High Power Amplification | `23 59 01` / `23 59 00` | `23 58` |
| SBX master / Scout Mode | `23 23 01 xx` / `23 23 02 xx` | `23 24` |
| Speaker layout (SPEAKER_CONFIGURATION) | `23 4e 01` HP, `23 4e 02` 2.0, `23 4e 04` with rears | `23 4d` |

Scout on/off did not send an explicit SBX command; the readback after `23 24`
reflects the exclusivity. HPA on triggered ~1.3 s of isochronous audio frames
(tone or amp re-sync); harmless.

0x5A-framed commands exist for a few device-level settings, with a
`5a xx 01 01` read right after:

| Control | set |
|---|---|
| Speaker Output Target | `5a 2c 05 00 03` Line+Amp, `01` Line Out, `02` Amplifier Out |
| Auto Standby | `5a 39 03 00 0b 01` on, `... 00` off |

Speaker configuration changes (`23 4e`) are followed by a complete re-run of
the app-launch init sequence, then a float set in **group 0x80**, set id 08:
1.0 = 2.0, 5.0 = 4.0, 7.0 = 5.0, 8.0 = 5.1 (the `SpeakerAdvancedConfiguration`
enum from the Bluetooth notes, 2.0=1 ... 5.1=8).
The value is `1 + subwoofer + 2 x center + 4 x rear`, so 2.0 = 2.1,
3.0 = 3.0, 4.0 = 3.1 and 6.0 = 4.1 as well (alcatron/mb-x7-control
`PROTOCOL.md`, section 3).

Bass management (group 0x96): set id 2a = Bass Redirection enable, 2c =
bass-management crossover Hz (default 80), 3c = Subwoofer Gain enable,
34/36/38/3a = per-channel full-range flags (toggled by Full-Range boxes;
exact mapping not pinned down).

DRC (Cinematic) is group 0x97 set id 04: 1.0 Full, 2.0 Normal, 3.0 Night.

## USB findings, session 5 (equalizer)

All in group 0x96 as float32 dB, set via the usual `20 00 16 0a d5 02 08 <id> 20 96 <f32>`:

| set id | control | range |
|---|---|---|
| 12 | EQ enable | 0 / 1 |
| 14 | LEVEL (preamp) | -6.0 .. +6.0 |
| 16, 18, 1a, 1c, 1e, 20, 22, 24, 26, 28 | bands 31, 62, 125, 250, 500, 1k, 2k, 4k, 8k, 16k Hz | -12.0 .. +12.0 |

Presets are not a device concept: choosing one makes the app send all ten
band values (Rock = -1,-1,1,2,-1,-1,0,0,4,4 with preamp 0). Preset list in UI
order: Acoustic, Classical, Country, Dance, Flat, Jazz, New Age, Pop, Rock, Vocal.

## USB findings, session 6 (device-initiated events, control panel closed)

- **Hardware SBX button**: the device pushes the affected parameters unsolicited on
  endpoint 0x86 (`00 26 01 00 96 <id> <f32>` for surround/crystalizer/dialog
  enables etc.) followed by an SBX state event `00 23 23 00 01 <on> <on> 00 01`.
  So a daemon only has to listen on 0x86 to track button presses.
- **Volume knob**: standard HID consumer-control reports on 0x86, `01` = volume
  up, `02` = volume down, each followed by `00` (release). Windows then writes
  the new master volume back as a 2-byte little-endian int16 in 1/256 dB
  (e.g. `27 ec` = -5081 = -19.85 dB, `dd f0` = -3875 = -15.1 dB). The same
  write follows every output switch (`01 eb` = -21 dB in session 2), so it is
  Windows volume sync, not a control-panel command.
- **Headphone jack**: event `00 01 00 00 00 02` on endpoint 0x84, then the device
  pushes `00 23 12 00 04` (jack present; `00` when removed), `00 23 4c 00 0x ...`,
  and `00 23 4d 00 01` / `02`. The device switches output by itself on
  insert/remove and reports the new state via the `23 4d` reply.
- The event after an output switch in session 2, `00 01 00 02 00 14` on 0x84, has
  the same shape with a different code.

## USB findings, session 7 (mixer and CrystalVoice)

### Mixer = standard USB Audio Class 1 control requests, not HID

Every mixer slider and mute is a class request on interface 0 (the audio
control interface): `bmRequestType 0x21, bRequest 0x01 (SET_CUR)`,
`wIndex = (feature_unit << 8) | 0`, and

- mute: `wValue = 0x0100`, 1 data byte, `01` muted / `00` unmuted
- volume: `wValue = 0x0201` (left) and `0x0202` (right), int16 LE in 1/256 dB,
  `0xC000` = -64 dB (slider 0), `0x0600` = +6 dB (slider max)

| Feature unit | Mixer control |
|---|---|
| 0x0F | Mic-In/Mic Array **monitoring** (the room-sound loopback). Same unit as the CrystalVoice page's Mic Monitoring Volume. |
| 0x13 | USB Host monitoring |
| 0x12 | SPDIF In monitoring |
| 0x11 | Bluetooth monitoring |
| 0x10 | Line In monitoring |
| 0x14 | Högtalare / master playback volume |
| 0x09 | Mic-In/Mic Array **recording** |
| 0x0A | Line In recording |
| 0x0C | What U Hear recording |
| not exercised | Bluetooth / SPDIF In / USB Host recording (likely 0x0B, 0x0D, 0x0E) |

SPDIF Out mute sent nothing over USB (checked twice). Because this is plain UAC,
on Linux these should already show up as ALSA mixer controls; killing the room
loopback is "mute feature unit 0x0F" with no custom protocol needed.

### CrystalVoice = HID group 0x95, same set/get scheme as SBX

- Master: `23 23 11 01` on / `23 23 11 00` off (feature 0x11 in the `23 23`
  family, alongside SBX 01 and Scout 02). Read with `23 24`.

| set id | get id | control |
|---|---|---|
| 00 | 00 | Acoustic Echo Cancellation enable |
| 08 | 04 | Noise Reduction enable (app also reads `23 24` after) |
| 0c | 06 | Focus enable |
| 10 | 08 | Focus wedge angle, degrees 20.0 .. 180.0 (default 30) |
| 14 | 0a | FX enable |
| 16 .. 24 | 0b .. 12 | FX voice parameters, 8 floats (Neutral = 400, 1400, 2000, 1.0, 1.0, 1.0, 0, 0) |
| 26 | 13 | Mic EQ enable |
| 28 .. 36 | 14 .. 1b | Mic EQ bands, 8 floats in dB (DM-1 = 0, 8, 0, 12, 12, 4, 8, 10) |
| 58 | 2c | Smart Volume (mic) enable |
| 5a | 2d | Smart Volume (mic) level 0.0 .. 1.0 (default 0.74) |

FX presets (18, UI order: Alien Brute, Alien Infiltrator, Deep Voice, Demon,
Dwarf, Elderly, Elf, Emo, Female to Male, From Up North, Male to Female,
Marine, Munchkin, Neutral, Orc, Robot, Scrappy Kid, Unstable) and Mic EQ
presets (DM-1, Flat, Preset 1..10) are client-side: the app sends the full
parameter set. Mic Boost was greyed out with no mic connected.

## USB findings, session 8 (master mute)

- Knob press: HID consumer-control report `04` (mute) then `00` (release) on
  endpoint 0x86, same family as `01` volume up / `02` volume down.
- All three paths (knob press, keyboard mute key, app speaker icon) end in the
  same write from Windows: UAC `SET_CUR`, `wValue 0x0100`, feature unit 0x14
  (`wIndex 0x1400`), 1 byte `01` muted / `00` unmuted. Keyboard and app send
  only that write; nothing Creative-specific is involved.
- So master mute and volume are feature unit 0x14 mute/volume, and the knob and
  its button are ordinary HID consumer controls. On Linux both should work out
  of the box through ALSA and the kernel HID consumer-control mapping.

## Linux verification (2026-10-10, a throwaway libusb test tool against hidraw)

Everything below was confirmed live on Ubuntu 24.04 with the X7 on
`/dev/hidraw5`, using the udev rule in `70-soundblaster-x7.rules`.

### Transport corrections

- The HID report descriptor has **no report IDs**. Commands are 64-byte
  **Output** reports, not Feature reports: the Windows `wValue 0x0200` means
  type 2 = Output. A plain `write()` of `00` + 64 bytes to hidraw becomes the
  same `SET_REPORT` control transfer (the X7 has no interrupt OUT endpoint),
  and the kernel strips the leading report-id byte. Feature-report ioctls are
  silently ignored by the device.
- The input report is 16 bytes: byte 0 is the consumer-control bitfield
  (`01` volume up, `02` down, `04` mute), bytes 1..15 carry protocol replies.
  That is the leading `00` on every reply in the captures.
- Endpoint 0x86 is the HID interface's interrupt IN, so **replies and
  device-initiated events all arrive on hidraw reads**. Endpoint 0x84 is the
  UAC status interrupt on interface 0, owned by snd-usb-audio; a HID daemon
  never sees it and does not need it.

### Confirmed commands

| Test | Sent | Got |
|---|---|---|
| Output read | `23 4d` | `00 23 4d 00 01` (headphones) |
| Switch to speakers | `23 4e 00 00 00 80` | `00 23 4d 00 02`, `00 23 4e 00 00 00 00 80`, then pushes of group 96 ids 1a, 15, 1e |
| Switch to headphones | `23 4e 01` | `00 23 4e 00 01` + same three pushes |
| Crystalizer level get | `26 01 96 08` | `00 26 01 00 96 08 3f 26 66 66` (0.65) |
| Crystalizer level set | `20 00 16 0a d5 02 08 10 20 96 3e 80 00 00` | `00 20` ack; next get returns 0.25 |

Replies arrive within a few ms. Trailing zeros are trimmed in the listings.

### Device-initiated events, as seen on hidraw

- SBX button off: pushes `26 01 00 96 <id> <f32>` for ids 00, 07, 02 (value
  0.0), then 1a, 15, 1e, then state `00 23 23 00 01` (trailing zeros = off).
  SBX on: same ids with `3f 80` (1.0), state `00 23 23 00 01 01 01`.
- Knob: `02` per detent down, `01` per detent up, each followed by an
  all-zero release report ~10 ms later. Knob press: `04` then release, once
  per press (mute and unmute look identical; state lives in UAC unit 0x14).
- Jack removed: `00 23 12`, `00 23 4c 00 06 00 00 80`, `00 23 4d 00 02`.
  Jack inserted: `00 23 12 00 04`, `00 23 4c 00 07 00 00 80`, `00 23 4d 00 01`.
  Both followed by the 1a/15/1e pushes. The device switches output itself.

### Feature mask (`23 24` / `23 25`), confirmed live

- `23 24` replies `00 23 24 00` + a 32-bit little-endian bitmask of enabled
  features, bit `fid - 1`: SBX (fid 1) bit 0, Scout (fid 2) bit 1,
  CrystalVoice (fid 0x11) bit 16. `01 00 01 00` = SBX and CrystalVoice on.
- `23 25` is the same mask for available features: `03 00 01 00`.
- The `23 23 fid val` set reply and the SBX-button push are
  `00 23 23 00 fid val` followed by the updated mask. This is the
  `HARDWARE_BUTTON_STATE` encoding from the Bluetooth notes.

### UAC (mixer) from userspace

`SET_CUR` mute on feature unit 0x0F via usbfs fails with **EBUSY** while
snd-usb-audio owns interface 0. With a detach / claim / send / release /
reattach cycle the request succeeds (`Ok(1)`) and the device honours it,
but **do not do this**: detaching interface 0 destroys the ALSA card, and
reattaching only interface 0 rebuilds it without any PCM devices because
the streaming interfaces are still bound to the dead driver instance.
PipeWire then drops the X7 and falls back to another sink. Recovery needed
a physical replug plus a WirePlumber restart. So the monitoring units
0x0F-0x13 cannot be driven from userspace; they need an ALSA mixer control
or a kernel-side route. Still untested: whether the ALSA
`Mic-In/Mic Array Capture Switch` affects recording or monitoring.

The mute state on the device persists: a mute=0 sent during testing left the
mic room loopback audible until a mute=1 was sent again.

What the kernel driver does expose as ALSA mixer controls: master mute and
volume (`Speaker Playback Switch/Volume`, unit 0x14) and the recording section
(the `* Capture Switch/Volume` controls and `PCM Capture Source`, units
0x09-0x0E). Nothing maps to the monitoring units 0x0F-0x13, so mixer settings
such as mic monitoring are not reachable from userspace, and this software
does not touch them.
