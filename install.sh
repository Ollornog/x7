#!/bin/sh
# Installs the Sound Blaster X7 tools for the current user: the two
# binaries, a launcher entry, an autostart entry, and the udev rule (the
# one root step, run only after you say yes). Run it from an unpacked
# release tarball, or on its own to fetch the latest release. Running it
# again updates. `--uninstall` reverses it.
set -eu

# Everything runs from main, called on the last line: a download cut
# short under `curl ... | sh` then runs nothing at all.
main() {

  usage='usage: install.sh [--no-autostart] [--uninstall]'
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

  autostart=1
  uninstall=0
  for arg in "$@"; do
    case $arg in
      --no-autostart) autostart=0 ;;
      --uninstall) uninstall=1 ;;
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

  if [ "$uninstall" = 1 ]; then
    say "removing $bin_dir/x7, $bin_dir/x7-gui, the launcher and autostart entries"
    rm -f "$bin_dir/x7" "$bin_dir/x7-gui" "$app_dir/x7-gui.desktop" "$autostart_dir/x7-gui.desktop"
    rm -f "$share_dir/$rule"
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
  if [ "$autostart" = 1 ]; then
    say "autostart: $autostart_dir/x7-gui.desktop"
    install -Dm644 "$app_dir/x7-gui.desktop" "$autostart_dir/x7-gui.desktop"
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
