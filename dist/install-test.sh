#!/bin/sh
# Exercises install.sh without root, a device, or GitHub.
# usage: install-test.sh <repo-root>
set -eu
root=$(cd "$1" && pwd -P)
work=$(mktemp -d)
server=
trap 'kill $server 2>/dev/null || true; rm -rf "$work"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }
has() { grep -q -- "$2" "$1" || fail "expected '$2' in $1"; }
lacks() { grep -q -- "$2" "$1" && fail "did not expect '$2' in $1"; return 0; }

# A payload laid out like the tarball's top directory.
payload=$work/x7-0.0.0
mkdir -p "$payload"
printf '#!/bin/sh\necho x7 0.0.0\n' > "$payload/x7"
printf '#!/bin/sh\necho x7-gui 0.0.0\n' > "$payload/x7-gui"
chmod 755 "$payload/x7" "$payload/x7-gui"
cp "$root/70-soundblaster-x7.rules" "$root/dist/x7-gui.desktop" "$root/LICENSE" "$root/README.md" "$payload/"
cp "$root/install.sh" "$payload/install.sh"

home=$work/home
udev=$work/udev
mkdir -p "$home" "$udev"
# setsid: no controlling terminal, so the udev prompt must not block.
# No DISPLAY: the installer must not start the tray from here.
run() { ( cd "$work" && HOME=$home X7_UDEV_DIR=$udev env -u DISPLAY -u WAYLAND_DISPLAY setsid -w sh "$@" ) </dev/null >"$work/out" 2>&1 || { cat "$work/out"; fail "install.sh exited non-zero: $*"; }; }

echo "1 install from the payload, by absolute path, from another directory"
run "$payload/install.sh"
[ -x "$home/.local/bin/x7" ] || fail "x7 not installed"
[ -x "$home/.local/bin/x7-gui" ] || fail "x7-gui not installed"
[ -f "$home/.local/share/applications/x7-gui.desktop" ] || fail "launcher entry missing"
[ -f "$home/.config/autostart/x7-gui.desktop" ] || fail "autostart entry missing (default on)"
# systemd's xdg-autostart generator resolves a bare Exec= against the
# service manager's PATH, which lacks ~/.local/bin at login: both copies
# must name the binary by absolute path.
has "$home/.config/autostart/x7-gui.desktop" "^Exec=\"$home/.local/bin/x7-gui\"$"
has "$home/.local/share/applications/x7-gui.desktop" "^Exec=\"$home/.local/bin/x7-gui\"$"

echo "1b a home with a space, an ampersand and a dollar sign: Exec is quoted and escaped"
odd="$work/ho me&\$x"
mkdir -p "$odd"
( cd "$work" && HOME=$odd X7_UDEV_DIR=$udev setsid -w sh "$payload/install.sh" ) </dev/null >"$work/out-odd" 2>&1 || { cat "$work/out-odd"; fail "install into the odd home exited non-zero"; }
# Two escaping layers, Desktop Entry value then Exec argument: $ becomes \\$.
# shellcheck disable=SC2016
grep -qF 'Exec="'"$work"'/ho me&\\$x/.local/bin/x7-gui"' "$odd/.config/autostart/x7-gui.desktop" || fail "Exec not quoted and escaped: $(grep '^Exec=' "$odd/.config/autostart/x7-gui.desktop")"
gen=/usr/lib/systemd/user-generators/systemd-xdg-autostart-generator
if [ -x "$gen" ]; then
  mkdir -p "$work/gen"
  env -i PATH=/usr/bin HOME="$odd" XDG_CONFIG_HOME="$odd/.config" "$gen" "$work/gen" "$work/gen" "$work/gen" >/dev/null 2>&1 || true
  [ -f "$work/gen/app-x7\\x2dgui@autostart.service" ] || fail "systemd's autostart generator made no unit for the odd home"
else
  echo "  (no systemd-xdg-autostart-generator here; generator check skipped)"
fi
has "$work/out" "sudo install -m 644 \"$home/.local/share/x7/70-soundblaster-x7.rules\" $udev/"
has "$work/out" "sudo udevadm control --reload"
has "$work/out" "installed x7-gui 0.0.0"
has "$work/out" "the tray icon appears at next login"
lacks "$work/out" "x7-gui started"

echo "1c with a display, the tray is started at the end"
if command -v pgrep >/dev/null 2>&1 && pgrep -x x7-gui >/dev/null 2>&1; then
  echo "  (an x7-gui is running on this machine; the start check is skipped)"
else
  ( cd "$work" && HOME=$home X7_UDEV_DIR=$udev DISPLAY=:99 setsid -w sh "$payload/install.sh" ) </dev/null >"$work/out" 2>&1 || { cat "$work/out"; fail "install with a display exited non-zero"; }
  has "$work/out" "x7-gui started"
  lacks "$work/out" "the tray icon appears at next login"
fi

echo "2 PATH without ~/.local/bin warns"
( cd "$work" && HOME=$home X7_UDEV_DIR=$udev PATH=/usr/bin:/bin setsid -w sh "$payload/install.sh" ) </dev/null >"$work/out" 2>&1
has "$work/out" "not on your PATH"

echo "3 rule already identical: skipped"
cp "$root/70-soundblaster-x7.rules" "$udev/"
run "$payload/install.sh"
has "$work/out" "already installed, unchanged"
lacks "$work/out" "sudo install"

echo "4 rule present but different: the two lines again"
echo "# stale" > "$udev/70-soundblaster-x7.rules"
run "$payload/install.sh"
has "$work/out" "sudo install -m 644"

echo "5 --no-autostart removes the autostart entry"
run "$payload/install.sh" --no-autostart
[ ! -e "$home/.config/autostart/x7-gui.desktop" ] || fail "autostart entry still there"

echo "6 --uninstall removes what it put and leaves profiles"
mkdir -p "$home/.config/x7/profiles" && touch "$home/.config/x7/profiles/keep.toml"
run "$payload/install.sh" --uninstall
if [ -e "$home/.local/bin/x7" ] || [ -e "$home/.local/bin/x7-gui" ]; then fail "binaries still there"; fi
[ ! -e "$home/.local/share/applications/x7-gui.desktop" ] || fail "launcher entry still there"
[ -f "$home/.config/x7/profiles/keep.toml" ] || fail "profiles touched"
[ ! -e "$home/.local/share/x7" ] || fail "durable rule copy still there"
has "$work/out" "sudo rm $udev/70-soundblaster-x7.rules"
has "$work/out" "sudo udevadm control --reload"

echo "7 download path against a fake release server"
site=$work/site
mkdir -p "$site"
tar -czf "$site/x7-0.0.0-x86_64-linux.tar.gz" -C "$work" x7-0.0.0
(cd "$site" && sha256sum x7-0.0.0-x86_64-linux.tar.gz > x7-0.0.0-x86_64-linux.tar.gz.sha256)
port=18765
cat > "$site/latest.json" <<EOF
{"assets": [
  {"name": "x7-0.0.0-x86_64-linux.tar.gz.sha256", "browser_download_url": "http://127.0.0.1:$port/x7-0.0.0-x86_64-linux.tar.gz.sha256"},
  {"name": "x7-0.0.0-x86_64-linux.tar.gz", "browser_download_url": "http://127.0.0.1:$port/x7-0.0.0-x86_64-linux.tar.gz"}
]}
EOF
( cd "$site" && python3 -I -m http.server "$port" --bind 127.0.0.1 ) >/dev/null 2>&1 &
server=$!
i=0; until curl -fs "http://127.0.0.1:$port/latest.json" >/dev/null 2>&1; do i=$((i+1)); [ $i -lt 50 ] || fail "fake server did not start"; timeout 0.2 tail -f /dev/null || true; done
rm -rf "$home"; mkdir -p "$home"
( cd "$work" && HOME=$home X7_UDEV_DIR=$udev X7_RELEASES_URL="http://127.0.0.1:$port/latest.json" setsid -w sh "$root/install.sh" ) </dev/null >"$work/out" 2>&1 || { cat "$work/out"; fail "download install exited non-zero"; }
has "$work/out" "downloading the latest release"
[ -x "$home/.local/bin/x7-gui" ] || fail "download path installed nothing"
has "$work/out" "installed x7-gui 0.0.0"
# The payload is a temp dir gone at exit: the printed sudo line must name a durable copy.
has "$work/out" "sudo install -m 644 \"$home/.local/share/x7/70-soundblaster-x7.rules\""
[ -f "$home/.local/share/x7/70-soundblaster-x7.rules" ] || fail "no durable copy of the rule"
[ -z "$(find "${TMPDIR:-/tmp}" -maxdepth 1 -name 'tmp.*' -newer "$site/latest.json" -type d 2>/dev/null | head -1)" ] || echo "note: a temp dir newer than the test exists; check install.sh cleans up"

echo "7b a tarball that fails its checksum is refused before unpacking"
cp "$site/x7-0.0.0-x86_64-linux.tar.gz.sha256" "$work/good.sha256"
printf '%s  x7-0.0.0-x86_64-linux.tar.gz\n' 0000000000000000000000000000000000000000000000000000000000000000 > "$site/x7-0.0.0-x86_64-linux.tar.gz.sha256"
mkdir -p "$work/home-bad"
if ( cd "$work" && HOME=$work/home-bad X7_UDEV_DIR=$udev X7_RELEASES_URL="http://127.0.0.1:$port/latest.json" setsid -w sh "$root/install.sh" ) </dev/null >"$work/out" 2>&1; then fail "expected exit 1 on a bad checksum"; fi
has "$work/out" "checksum"
[ ! -e "$work/home-bad/.local/bin/x7-gui" ] || fail "installed despite a bad checksum"
cp "$work/good.sha256" "$site/x7-0.0.0-x86_64-linux.tar.gz.sha256"

echo "8 a release without the tarball: one-line error, exit 1"
printf '{"assets": []}\n' > "$site/latest.json"
if ( cd "$work" && HOME=$home X7_UDEV_DIR=$udev X7_RELEASES_URL="http://127.0.0.1:$port/latest.json" setsid -w sh "$root/install.sh" ) </dev/null >"$work/out" 2>&1; then fail "expected exit 1"; fi
has "$work/out" "no x86_64 Linux tarball"

echo "9 piped from a directory holding installed binaries: still downloads"
printf '{"assets": [{"name": "x7-0.0.0-x86_64-linux.tar.gz", "browser_download_url": "http://127.0.0.1:%s/x7-0.0.0-x86_64-linux.tar.gz"}]}\n' "$port" > "$site/latest.json"
( cd "$home/.local/bin" && HOME=$home X7_UDEV_DIR=$udev X7_RELEASES_URL="http://127.0.0.1:$port/latest.json" setsid -w sh ) < "$root/install.sh" >"$work/out" 2>&1 || { cat "$work/out"; fail "piped install exited non-zero"; }
has "$work/out" "downloading the latest release"

echo "all install.sh checks passed"
