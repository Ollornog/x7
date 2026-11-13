# Changelog

## 0.2.0 (2026-11-13)

### Fixed
- The window opened dark and turned light on the first click or drag on a
  light X11 desktop: the desktop's color scheme was asked only on focus
  gain, so an unfocused new window showed egui's dark default. It is now
  asked before the first frame.
- Selecting a profile in the window loaded it at once; it now only
  selects, and Load applies it.
- The delete question ran past the window with a long profile name; it
  now wraps on its own row with Delete and Cancel, and long names in the
  list are shortened with an ellipsis.

### Changed
- The installer and the README fetch releases of this fork
  (Ollornog/x7-soundblaster) instead of Stamrin/x7.

### Added
- Profiles: `x7 profile list`, `x7 profile rename <old> <new>` (never
  overwrites) and `x7 profile delete <name>`; Rename and Delete in the
  window. `x7 profile load` refuses a file with High headphone gain or
  High Power Amplification without `--yes`, as the window asks first.
- Appearance choice in the window: Desktop, Light or Dark.
- Display name "X7 Sound Blaster" for the window, the tray and the
  launcher. Binaries, SNI id and app id are unchanged.
- Speaker layouts 2.1, 3.0, 3.1 and 4.1 (raw 2, 3, 4, 6: one plus subwoofer
  plus two times center plus four times rear, as the macOS panel encodes it).
  Only 2.0, 4.0, 5.0 and 5.1 were offered before, so a subwoofer could not be
  switched on with a stereo pair.
- Safe mode for every HID command (`docs/safety.md`): no command while a
  recording from the X7 is open or its ALSA card is missing; writes only
  after an answered read (the CLI reads `23 4d` first); a missing reply,
  a failed or a slow write stops all traffic to that enumeration of the
  device, without retry, resync or reset, and leaves a hang marker that
  clears itself after a power cycle.
- Traffic log `~/.local/state/x7/x7.log` (`$X7_LOG`, `$XDG_STATE_HOME`),
  rotated at 1 MiB.
- `x7 status`: device node, access, USB bus and device, recordings, hang
  marker and log path, without sending anything.
- GUI: "Not answering" state with the power-cycle advice; a 3 s quiet
  start after opening the device.
- Tests that replay the hang with a stand-in hidraw node.

### Changed
- The GUI session no longer sends a resync (`23 24`) and retries after a
  timeout, and no longer reopens a silent device: both are a hang now.
- A command the device answers with another report is reported as not
  applied instead of timing out.

## 0.1.4 (upstream, Stamrin/x7)
- Baseline of this fork.
