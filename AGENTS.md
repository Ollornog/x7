# AGENTS.md

For an agent installing x7 on someone's machine. Working on the code
itself: read `CLAUDE.md`.

## Installing

`install.sh` asks its questions on `/dev/tty`. An agent's shell has no
terminal, so every question counts as "no" and the placement choices are
skipped. Ask the person first, then pass the answers as flags.

1. On Omarchy (`omarchy` on PATH and `~/.config/omarchy/` present), ask:
   - Where the X7 icon goes: in the middle of the bar, right of the clock
     (`--bar=center-end`), or in the tray drawer on the right, behind its
     hover (`--bar=tray`). People usually want the middle.
   - Whether the controls window floats, centred (`--float`) or tiles
     (`--no-float`). Floating is what the window is designed for.
   - Whether it should start at login (the default) or not
     (`--no-autostart`).
2. Run it with those flags:

   ```sh
   curl -fsSL https://raw.githubusercontent.com/Ollornog/x7-soundblaster/main/install.sh -o install.sh
   sh install.sh --bar=center-end --float </dev/null
   ```

3. The udev rule is the one root step. With no terminal the installer
   prints two commands instead of running them. Run them with `pkexec`
   (one graphical password prompt) or hand them to the person:

   ```sh
   pkexec sh -c 'install -m 644 ~USER/.local/share/x7/70-soundblaster-x7.rules /etc/udev/rules.d/ && udevadm control --reload && udevadm trigger --subsystem-match=hidraw'
   ```

   Replace `~USER` with the person's home directory. Running
   `udevadm trigger` means the X7 does not need to be unplugged.
4. Check: `x7 output` prints `headphones` or `speakers` when the device
   is reachable.

The answers are saved in `~/.local/share/x7/omarchy.conf`. Running the
installer again without flags keeps them, so updates do not ask again.
To change one, run the installer again with the new flag.

## What the Omarchy options touch

- `--bar=center-end` copies the bar widget plugin to
  `~/.config/omarchy/plugins/io.github.stamrin.x7/`, enables it in the
  `centerEnd` section, and adds `"x7"` to `omarchy.tray`'s `hidden` list
  so the icon does not also show in the drawer. Both steps go through
  the running Omarchy shell. If the shell is not running, the installer
  prints the command to run later. A click on the widget opens a popup
  with all of the controls. When an update changes the widget's files,
  the installer restarts the Omarchy shell, because the shell keeps
  running a loaded widget's old code until it restarts.
- `--bar=tray` disables the widget and removes `"x7"` from the tray's
  hidden list again.
- `--float` writes `~/.config/hypr/x7.lua` (one `o.window` rule) and adds
  one `require("hypr.x7")` line to `~/.config/hypr/hyprland.lua`.
  `--no-float` removes both. After either, check `hyprctl configerrors`.
- `--uninstall` reverses all of the above along with the rest of the
  install. Profiles in `~/.config/x7/` are kept.

Other desktops: there are no placement flags. GNOME needs the
AppIndicator extension for the tray icon. Window rules for plain
Hyprland are in the README.
