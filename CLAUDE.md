# CLAUDE.md

Control software for the Creative Sound Blaster X7 over USB HID (hidraw).
Rust 2024 workspace: `x7` (library and CLI), `x7-gui` (ksni tray and
egui window).

## Hard rule

Never send USB Audio Class requests to the device and never detach the
kernel audio driver, in code or in tests. The reason is in
`docs/protocol.md` under "UAC (mixer) from userspace".

Every HID command goes through the safe mode in `x7/src/transport.rs`
and `x7/src/safety.rs` (`docs/safety.md`): no command during a
recording, writes only after an answered read, and no retry, resync,
reset or reopen after a missing reply. Do not add a path around it.

## Working here

- Gates before any commit: `cargo fmt --all --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`, and `sh dist/install-test.sh .` when
  `install.sh` or `dist/` changed. CI runs the same plus shellcheck.
- No device at hand: `cargo run -p x7-gui -- --fake` (an in-memory X7)
  or `--fake=absent`. The fake answers with the device's own reports.
- Releases: both crate versions move together. Bump them, tag
  `v<version>`, push the tag; `release.yml` builds the tarball in an
  `ubuntu:22.04` container and creates the GitHub Release. `install.sh`
  fetches the latest Release when run on its own.
- Desktop entries the installer writes name the binary by absolute,
  quoted path; systemd's autostart generator needs that. Keep
  `dist/x7-gui.desktop` with the bare name for system-wide packages.
- `dist/omarchy/io.github.stamrin.x7/` is the Omarchy bar widget the
  installer places with `--bar=center-end`: a bar icon plus a popup with
  every control, which reads and writes through the `x7` CLI, so CLI
  output and argument forms are its interface. It is shown while
  x7-gui's tray item (SNI id `x7`) exists; keep that id stable.
  `node dist/omarchy/model-test.js` tests its parsing. The shell does not
  hot-reload a live widget: `omarchy restart shell` after editing it. `AGENTS.md` is the install
  guide for agents and lists every installer flag; update it with them.
- No emoji in code, commits, docs or PRs.
