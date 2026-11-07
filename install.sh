#!/bin/sh
# Installs the Sound Blaster X7 tools for the current user: the two
# binaries, a launcher entry, an autostart entry, and the udev rule (the
# one root step, run only after you say yes). Run it from an unpacked
# release tarball, or on its own to fetch the latest release. Running it
# again updates. `--uninstall` reverses it. On Omarchy it also asks where
# the bar icon goes and whether the controls window floats; --bar and
# --float/--no-float answer in advance, and the answers are remembered.
set -eu

# Everything runs from main, called on the last line: a download cut
# short under `curl ... | sh` then runs nothing at all.
main() {

  usage='usage: install.sh [--no-autostart] [--bar=center-end|tray] [--float|--no-float] [--uninstall]'
  repo=${X7_REPO:-Stamrin/x7}
  releases_url=${X7_RELEASES_URL:-https://api.github.com/repos/$repo/releases/latest}
  udev_dir=${X7_UDEV_DIR:-/etc/udev/rules.d}
  rule=70-soundblaster-x7.rules
  bin_dir=$HOME/.local/bin
  app_dir=$HOME/.local/share/applications
  autostart_dir=$HOME/.config/autostart
  # A durable copy of the rule, so the printed sudo line still works after
  # a downloaded payload is cleaned up.
  share_dir=$HOME/.local/share/x7
  # Omarchy only: the bar widget plugin, the Hyprland rule file, and the
  # answers given, so an update does not ask again.
  plugin_id=io.github.stamrin.x7
  plugin_dir=$HOME/.config/omarchy/plugins/$plugin_id
  hypr_dir=$HOME/.config/hypr
  choices=$share_dir/omarchy.conf

  autostart=1
  uninstall=0
  bar=
  float=
  for arg in "$@"; do
    case $arg in
      --no-autostart) autostart=0 ;;
      --uninstall) uninstall=1 ;;
      --bar=center-end | --bar=tray) bar=${arg#--bar=} ;;
      --float) float=yes ;;
      --no-float) float=no ;;
      -h | --help) echo "$usage"; exit 0 ;;
      *) echo "install.sh: unknown argument $arg" >&2; echo "$usage" >&2; exit 2 ;;
    esac
  done

  say() { printf '%s\n' "$*"; }
  die() { printf 'install.sh: %s\n' "$*" >&2; exit 1; }

  # A yes/no question on the terminal itself, so it works under
  # `curl ... | sh` where stdin is the pipe. No terminal: the answer is no.
  ask() {
    if ( exec </dev/tty ) 2>/dev/null; then
      printf '%s [y/N] ' "$1" >/dev/tty
      read -r answer </dev/tty || answer=
      case $answer in y | Y | yes | YES) return 0 ;; esac
    fi
    return 1
  }

  # Print two privileged commands, offer to run them.
  offer_root() {
    say "  needs root; the two commands are:"
    say "    $1"
    say "    $2"
    if command -v sudo >/dev/null 2>&1 && ask "  Run them now?"; then
      return 0
    fi
    say "  not run; run them yourself"
    return 1
  }

  # Omarchy: its CLI on PATH and its config in this home.
  is_omarchy() { command -v omarchy >/dev/null 2>&1 && [ -d "$HOME/.config/omarchy" ]; }

  saved() { if [ -f "$choices" ]; then sed -n "s/^$1=//p" "$choices" | tail -n 1; fi; }
  save() {
    mkdir -p "$share_dir"
    { if [ -f "$choices" ]; then grep -v "^$1=" "$choices" || true; fi; printf '%s=%s\n' "$1" "$2"; } >"$choices.new"
    mv "$choices.new" "$choices"
  }

  # Add "x7" to, or take it out of, the tray's hidden list, keeping the
  # other entries. The shell owns shell.json, so this goes through it.
  tray_hidden() {
    current=$(omarchy bar get omarchy.tray hidden 2>/dev/null) || current='[]'
    case $current in '['*) ;; *) current='[]' ;; esac
    if [ "$1" = add ]; then
      new=$(printf '%s' "$current" | jq -c 'if index("x7") then . else . + ["x7"] end')
    else
      new=$(printf '%s' "$current" | jq -c 'map(select(. != "x7"))')
    fi
    [ "$new" = "$(printf '%s' "$current" | jq -c .)" ] || omarchy bar set omarchy.tray hidden "$new" --json >/dev/null
  }

  # center-end: the widget right of the clock, and the tray copy hidden.
  # tray: the widget off, the icon back in the tray drawer. Both go through
  # the running shell; without one, say what to run later.
  place_icon() {
    if [ "$1" = center-end ]; then
      omarchy-shell shell rescanPlugins >/dev/null 2>&1 || true
      # The rescan returns before the shell has read the new plugin, so the
      # first enable can answer "unknown": try for a few seconds.
      tries=0 enabled=0
      while [ "$tries" -lt 10 ]; do
        if omarchy plugin enable "$plugin_id" --section centerEnd >/dev/null 2>&1; then enabled=1; break; fi
        tries=$((tries + 1))
        sleep 0.5
      done
      if [ "$enabled" = 1 ] && tray_hidden add; then
        say "  in the bar's centerEnd section, right of the clock"
      else
        say "  the Omarchy shell did not answer; once it runs:"
        say "    omarchy plugin enable $plugin_id --section centerEnd"
        say "    and hide \"x7\" in the tray (right click its chevron)"
      fi
    else
      omarchy plugin disable "$plugin_id" >/dev/null 2>&1 || true
      tray_hidden remove 2>/dev/null || true
      say "  in the tray drawer"
    fi
  }

  # The rule lives in its own file, required from hyprland.lua, so taking
  # it out is deleting one file and one line.
  hypr_rule=$hypr_dir/x7.lua
  hypr_require='require("hypr.x7") -- Sound Blaster X7 (x7 install.sh)'
  set_float() {
    if [ "$1" = yes ]; then
      printf '%s\n' '-- Written by the x7 installer; install.sh --no-float removes it.' \
        'o.window("^(x7-gui)$", { float = true, center = true })' >"$hypr_rule"
      grep -qxF "$hypr_require" "$hypr_dir/hyprland.lua" || printf '\n%s\n' "$hypr_require" >>"$hypr_dir/hyprland.lua"
      say "  floats, centred: $hypr_rule"
    else
      if [ -f "$hypr_dir/hyprland.lua" ] && grep -qxF "$hypr_require" "$hypr_dir/hyprland.lua"; then
        grep -vxF "$hypr_require" "$hypr_dir/hyprland.lua" >"$hypr_dir/hyprland.lua.x7-new" || true
        cat "$hypr_dir/hyprland.lua.x7-new" >"$hypr_dir/hyprland.lua"
        rm -f "$hypr_dir/hyprland.lua.x7-new"
      fi
      rm -f "$hypr_rule"
      say "  tiles like any window"
    fi
    if command -v hyprctl >/dev/null 2>&1 && [ -n "${HYPRLAND_INSTANCE_SIGNATURE:-}" ]; then
      errors=$(hyprctl configerrors 2>/dev/null | grep -v '^$' || true)
      [ -z "$errors" ] || say "  hyprctl configerrors: $errors"
    fi
  }

  if [ "$uninstall" = 1 ]; then
    say "removing $bin_dir/x7, $bin_dir/x7-gui, the launcher and autostart entries"
    rm -f "$bin_dir/x7" "$bin_dir/x7-gui" "$app_dir/x7-gui.desktop" "$autostart_dir/x7-gui.desktop"
    if is_omarchy; then
      if [ -d "$plugin_dir" ]; then
        say "removing the Omarchy bar widget: $plugin_dir"
        place_icon tray >/dev/null
        rm -rf "$plugin_dir"
      fi
      if [ -f "$hypr_rule" ]; then
        say "removing the Hyprland rule: $hypr_rule"
        set_float no >/dev/null
      fi
    fi
    rm -f "$share_dir/$rule" "$choices"
    rmdir "$share_dir" 2>/dev/null || true
    if [ -f "$udev_dir/$rule" ]; then
      say "udev rule: $udev_dir/$rule"
      if offer_root "sudo rm $udev_dir/$rule" "sudo udevadm control --reload"; then
        sudo rm "$udev_dir/$rule" && sudo udevadm control --reload
      fi
    fi
    if command -v pgrep >/dev/null 2>&1 && pgrep -x x7-gui >/dev/null 2>&1; then
      say "x7-gui is still running; Quit it from the tray"
    fi
    say "done; profiles in ~/.config/x7 were left alone"
    exit 0
  fi

  # Where the files are: beside this script when run from an unpacked
  # tarball, else the latest release, downloaded. Absolute either way, so
  # every path printed below works from any directory.
  tmp=
  cleanup() { if [ -n "$tmp" ]; then rm -rf "$tmp"; fi; }
  trap cleanup EXIT
  # Only an on-disk install.sh has a payload beside it. Under `curl | sh`
  # $0 is `sh` and its directory is the caller's, which may be ~/.local/bin
  # with the installed binaries in it: those must not pass as the payload.
  script_dir=
  case $0 in
    */install.sh | install.sh) script_dir=$(cd "$(dirname "$0")" && pwd -P) ;;
  esac
  # A complete payload: both binaries (-f as well as -x, since in the source
  # tree `x7/` is a directory), the rule and the desktop entry.
  if [ -n "$script_dir" ] && [ -f "$script_dir/x7" ] && [ -x "$script_dir/x7" ] && [ -f "$script_dir/x7-gui" ] \
    && [ -f "$script_dir/$rule" ] && [ -f "$script_dir/x7-gui.desktop" ]; then
    payload_dir=$script_dir
  else
    command -v curl >/dev/null 2>&1 || die "curl is needed to download the release"
    tmp=$(mktemp -d)
    say "downloading the latest release of $repo"
    url=$(curl -fsSL "$releases_url" \
      | grep -o '"browser_download_url": *"[^"]*-x86_64-linux\.tar\.gz"' \
      | head -n 1 | sed 's/.*"\(http[^"]*\)"$/\1/')
    [ -n "$url" ] || die "no x86_64 Linux tarball in the latest release ($releases_url)"
    # The release carries <tarball>.sha256 beside the tarball; check it
    # before unpacking anything.
    name=${url##*/}
    curl -fsSL "$url" -o "$tmp/$name"
    curl -fsSL "$url.sha256" -o "$tmp/$name.sha256"
    (cd "$tmp" && sha256sum -c "$name.sha256" >/dev/null 2>&1) || die "checksum of $name does not match $name.sha256"
    tar -xzf "$tmp/$name" -C "$tmp"
    payload_dir=$(cd "$tmp"/x7-*/ && pwd -P)
  fi

  say "binaries: $bin_dir"
  install -Dm755 "$payload_dir/x7" "$bin_dir/x7"
  install -Dm755 "$payload_dir/x7-gui" "$bin_dir/x7-gui"
  case ":$PATH:" in
    *":$bin_dir:"*) ;;
    *) say "  note: $bin_dir is not on your PATH yet; most desktops add it at next login" ;;
  esac

  # The entry names the binary by absolute path: systemd's xdg-autostart
  # generator resolves a bare Exec= against the service manager's PATH,
  # which does not have ~/.local/bin at login (UWSM, GNOME), and then
  # generates no unit at all. The path is quoted for a home with a space
  # in it, and backslash, double quote, dollar and backtick are escaped
  # twice: once for the Desktop Entry value, once for the Exec argument.
  # A percent sign is left alone: the spec says %%, but systemd 261's
  # generator then reports the binary as missing and makes no unit.
  say "launcher entry: $app_dir/x7-gui.desktop"
  mkdir -p "$app_dir"
  exec_path=$(printf '%s' "$bin_dir/x7-gui" | sed 's/\\/\\\\\\\\/g; s/["$`]/\\\\&/g')
  { grep -v '^Exec=' "$payload_dir/x7-gui.desktop"; printf 'Exec="%s"\n' "$exec_path"; } >"$app_dir/x7-gui.desktop"
  chmod 644 "$app_dir/x7-gui.desktop"
  # The autostart copy passes --autostart: started before the tray host is
  # up (GNOME loads its extension after autostarted apps), the app waits
  # for the host instead of opening its window.
  if [ "$autostart" = 1 ]; then
    say "autostart: $autostart_dir/x7-gui.desktop"
    mkdir -p "$autostart_dir"
    { grep -v '^Exec=' "$app_dir/x7-gui.desktop"; printf 'Exec="%s" --autostart\n' "$exec_path"; } >"$autostart_dir/x7-gui.desktop"
    chmod 644 "$autostart_dir/x7-gui.desktop"
  else
    say "autostart: skipped (--no-autostart)"
    rm -f "$autostart_dir/x7-gui.desktop"
  fi

  say "udev rule: $udev_dir/$rule"
  install -Dm644 "$payload_dir/$rule" "$share_dir/$rule"
  if [ -f "$udev_dir/$rule" ] && cmp -s "$share_dir/$rule" "$udev_dir/$rule"; then
    say "  already installed, unchanged"
  elif offer_root "sudo install -m 644 \"$share_dir/$rule\" $udev_dir/" "sudo udevadm control --reload"; then
    sudo install -m 644 "$share_dir/$rule" "$udev_dir/" && sudo udevadm control --reload
    say "  installed; unplug and replug the X7"
  else
    say "  then unplug and replug the X7"
  fi

  # Omarchy: where the icon goes and whether the window floats. A flag
  # wins, then the answer from a previous run, then the question; with no
  # terminal to ask on, nothing changes.
  if is_omarchy; then
    if [ -d "$payload_dir/omarchy/$plugin_id" ]; then
      say "Omarchy bar widget: $plugin_dir"
      # The shell keeps a loaded widget's old code until it restarts, so
      # note whether this run changes the files of one already there.
      changed=0
      if [ -d "$plugin_dir" ]; then
        for f in "$payload_dir/omarchy/$plugin_id/"*; do
          cmp -s "$f" "$plugin_dir/${f##*/}" || changed=1
        done
      fi
      mkdir -p "$plugin_dir"
      cp "$payload_dir/omarchy/$plugin_id/"* "$plugin_dir/"
      [ -n "$bar" ] || bar=$(saved bar)
      if [ -z "$bar" ] && ( exec </dev/tty ) 2>/dev/null; then
        if ask "  Put the X7 icon in the middle of the bar, right of the clock (no: tray drawer)?"; then bar=center-end; else bar=tray; fi
      fi
      if [ -n "$bar" ]; then
        place_icon "$bar"
        save bar "$bar"
        if [ "$changed" = 1 ] && [ "$bar" = center-end ] && omarchy-shell shell ping >/dev/null 2>&1; then
          say "  updated; restarting the Omarchy shell to load it"
          omarchy restart shell >/dev/null 2>&1 || say "  restart failed; run: omarchy restart shell"
        fi
      else
        say "  not placed; install.sh --bar=center-end puts it right of the clock"
      fi
    fi
    if [ -f "$hypr_dir/hyprland.lua" ]; then
      say "controls window:"
      [ -n "$float" ] || float=$(saved float)
      if [ -z "$float" ] && ( exec </dev/tty ) 2>/dev/null; then
        if ask "  Float the controls window, centred?"; then float=yes; else float=no; fi
      fi
      if [ -n "$float" ]; then
        set_float "$float"
        save float "$float"
      else
        say "  unchanged; install.sh --float floats it"
      fi
    fi
  fi

  say "installed $("$bin_dir/x7-gui" --version)"
  # Start the tray now when a desktop session is here. One runs per
  # session: a second launch only asks the running one to show its window,
  # so an update says to restart it instead. Started before the replug it
  # shows "not connected" and picks the device up by itself.
  if command -v pgrep >/dev/null 2>&1 && pgrep -x x7-gui >/dev/null 2>&1; then
    say "x7-gui is running; Quit it from the tray and start it again to pick up the update"
  elif [ -n "${DISPLAY:-}" ] || [ -n "${WAYLAND_DISPLAY:-}" ]; then
    (setsid "$bin_dir/x7-gui" >/dev/null 2>&1 </dev/null &)
    say "x7-gui started; the tray icon is up"
  else
    say "the tray icon appears at next login, or now with: x7-gui &"
  fi
}

main "$@"
