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
mkdir -p "$payload/omarchy"
cp -R "$root/dist/omarchy/io.github.stamrin.x7" "$payload/omarchy/"

home=$work/home
udev=$work/udev
mkdir -p "$home" "$udev"
# A pgrep that finds nothing: the checks below are about a machine with no
# x7-gui running, whatever is running here.
mkdir -p "$work/stub"
printf '#!/bin/sh\nexit 1\n' > "$work/stub/pgrep"
chmod 755 "$work/stub/pgrep"
PATH=$work/stub:$PATH
export PATH
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
# must name the binary by absolute path. The autostart copy adds --autostart.
has "$home/.config/autostart/x7-gui.desktop" "^Exec=\"$home/.local/bin/x7-gui\" --autostart$"
has "$home/.local/share/applications/x7-gui.desktop" "^Exec=\"$home/.local/bin/x7-gui\"$"

echo "1b a home with a space, an ampersand and a dollar sign: Exec is quoted and escaped"
odd="$work/ho me&\$x"
mkdir -p "$odd"
( cd "$work" && HOME=$odd X7_UDEV_DIR=$udev setsid -w sh "$payload/install.sh" ) </dev/null >"$work/out-odd" 2>&1 || { cat "$work/out-odd"; fail "install into the odd home exited non-zero"; }
# Two escaping layers, Desktop Entry value then Exec argument: $ becomes \\$.
# shellcheck disable=SC2016
grep -qF 'Exec="'"$work"'/ho me&\\$x/.local/bin/x7-gui" --autostart' "$odd/.config/autostart/x7-gui.desktop" || fail "Exec not quoted and escaped: $(grep '^Exec=' "$odd/.config/autostart/x7-gui.desktop")"
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
# Not Omarchy (no ~/.config/omarchy): no bar widget, no window rule.
lacks "$work/out" "Omarchy"
[ ! -e "$home/.config/omarchy" ] || fail "an Omarchy directory was made outside Omarchy"

echo "1c with a display, the tray is started at the end"
( cd "$work" && HOME=$home X7_UDEV_DIR=$udev DISPLAY=:99 setsid -w sh "$payload/install.sh" ) </dev/null >"$work/out" 2>&1 || { cat "$work/out"; fail "install with a display exited non-zero"; }
has "$work/out" "x7-gui started"
lacks "$work/out" "the tray icon appears at next login"

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

echo "10 Omarchy: bar placement and the float rule, through a stub omarchy CLI"
om=$work/omarchy-home
mkdir -p "$om/.config/omarchy" "$om/.config/hypr" "$work/omstub"
printf -- '-- user config\nrequire("hypr.bindings")\n' > "$om/.config/hypr/hyprland.lua"
# The stub logs every call and keeps the tray's hidden list in a file, the
# way the shell keeps it in shell.json.
printf '["steam"]\n' > "$work/hidden"
: > "$work/omarchy.log"
cat > "$work/omstub/omarchy" <<EOF
#!/bin/sh
echo "\$*" >> "$work/omarchy.log"
case "\$*" in
  "bar get omarchy.tray hidden") cat "$work/hidden" ;;
  "bar set omarchy.tray hidden "*) printf '%s\n' "\$5" > "$work/hidden" ;;
esac
exit 0
EOF
printf '#!/bin/sh\nexit 0\n' > "$work/omstub/omarchy-shell"
chmod 755 "$work/omstub/omarchy" "$work/omstub/omarchy-shell"
omrun() { ( cd "$work" && HOME=$om X7_UDEV_DIR=$udev PATH="$work/omstub:$PATH" env -u DISPLAY -u WAYLAND_DISPLAY -u HYPRLAND_INSTANCE_SIGNATURE setsid -w sh "$@" ) </dev/null >"$work/out" 2>&1 || { cat "$work/out"; fail "install.sh exited non-zero: $*"; }; }
req='require("hypr.x7") -- Sound Blaster X7 (x7 install.sh)'

echo "10a no terminal and no flags: nothing placed, nothing written"
omrun "$payload/install.sh"
[ -f "$om/.config/omarchy/plugins/io.github.stamrin.x7/manifest.json" ] || fail "plugin manifest not installed"
[ -f "$om/.config/omarchy/plugins/io.github.stamrin.x7/Panel.qml" ] || fail "plugin widget not installed"
has "$work/out" "install.sh --bar=center-end"
has "$work/out" "install.sh --float"
[ ! -e "$om/.config/hypr/x7.lua" ] || fail "rule written without being asked for"
lacks "$work/omarchy.log" "plugin enable"

echo "10b --bar=center-end --float"
omrun "$payload/install.sh" --bar=center-end --float
has "$work/omarchy.log" "plugin enable io.github.stamrin.x7 --section centerEnd"
[ "$(cat "$work/hidden")" = '["steam","x7"]' ] || fail "tray hidden list: $(cat "$work/hidden")"
has "$om/.config/hypr/x7.lua" 'o.window("^(x7-gui)\$", { float = true, center = true })'
[ "$(grep -cxF "$req" "$om/.config/hypr/hyprland.lua")" = 1 ] || fail "require line not added once"
has "$om/.local/share/x7/omarchy.conf" "^bar=center-end$"
has "$om/.local/share/x7/omarchy.conf" "^float=yes$"

echo "10c an update with no flags keeps the answers and adds nothing twice"
omrun "$payload/install.sh"
[ "$(grep -cxF "$req" "$om/.config/hypr/hyprland.lua")" = 1 ] || fail "require line duplicated"
[ "$(cat "$work/hidden")" = '["steam","x7"]' ] || fail "tray hidden list changed: $(cat "$work/hidden")"
has "$work/out" "right of the clock"

echo "10d --bar=tray --no-float undoes both and keeps the rest"
omrun "$payload/install.sh" --bar=tray --no-float
has "$work/omarchy.log" "plugin disable io.github.stamrin.x7"
[ "$(cat "$work/hidden")" = '["steam"]' ] || fail "tray hidden list: $(cat "$work/hidden")"
[ ! -e "$om/.config/hypr/x7.lua" ] || fail "rule file still there"
lacks "$om/.config/hypr/hyprland.lua" "hypr.x7"
has "$om/.config/hypr/hyprland.lua" 'require("hypr.bindings")'
has "$om/.local/share/x7/omarchy.conf" "^bar=tray$"

echo "10e --uninstall removes the widget, the rule and the answers"
omrun "$payload/install.sh" --bar=center-end --float
omrun "$payload/install.sh" --uninstall
[ ! -e "$om/.config/omarchy/plugins/io.github.stamrin.x7" ] || fail "plugin still there"
[ ! -e "$om/.config/hypr/x7.lua" ] || fail "rule file still there"
lacks "$om/.config/hypr/hyprland.lua" "hypr.x7"
[ "$(cat "$work/hidden")" = '["steam"]' ] || fail "tray hidden list after uninstall: $(cat "$work/hidden")"
[ ! -e "$om/.local/share/x7" ] || fail "answers still there"

echo "10f an unknown --bar value is refused"
if ( HOME=$om sh "$payload/install.sh" --bar=left ) </dev/null >"$work/out" 2>&1; then fail "expected exit 2 for --bar=left"; fi
has "$work/out" "unknown argument --bar=left"

echo "all install.sh checks passed"
