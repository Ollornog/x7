#!/bin/sh
# Builds the release tarball: x7 (static musl), x7-gui (glibc), the udev
# rule, the desktop file, install.sh, LICENSE and README. release.yml
# runs it inside ubuntu:22.04, which sets the glibc floor; the same
# image runs it by hand. The version must equal both crate versions.
# usage: dist/package.sh <version>
set -eu
version=${1:?usage: dist/package.sh <version>}
root=$(cd "$(dirname "$0")/.." && pwd -P)
cd "$root"

for crate in x7 x7-gui; do
  have=$(sed -n 's/^version = "\(.*\)"/\1/p' "$crate/Cargo.toml" | head -n 1)
  [ "$have" = "$version" ] || { echo "package.sh: $crate/Cargo.toml says $have, the tag says $version" >&2; exit 1; }
done

cargo build --release -p x7 --target x86_64-unknown-linux-musl
cargo build --release -p x7-gui
target=${CARGO_TARGET_DIR:-target}
x7_bin=$target/x86_64-unknown-linux-musl/release/x7
gui_bin=$target/release/x7-gui

# `file` says "statically linked" or, for a static PIE, "static-pie linked".
file "$x7_bin" | grep -Eq 'static(ally|-pie) linked' || { echo "package.sh: x7 is not a static binary:" >&2; file "$x7_bin" >&2; exit 1; }
# The dependency-free promise: x7-gui may need only libc, libm, libgcc_s,
# the vDSO and the loader. Anything else fails the release, not the
# user's install.
extra=$(ldd "$gui_bin" | awk '{print $1}' \
  | grep -Ev '^(linux-vdso\.so|libc\.so|libm\.so|libgcc_s\.so|/lib64/ld-linux-x86-64\.so)' || true)
[ -z "$extra" ] || { echo "package.sh: x7-gui links libraries beyond the allow list:" >&2; echo "$extra" >&2; exit 1; }

out=$target/package
stage=$out/x7-$version
rm -rf "$out"
mkdir -p "$stage"
install -m755 "$x7_bin" "$stage/x7"
install -m755 "$gui_bin" "$stage/x7-gui"
strip "$stage/x7" "$stage/x7-gui"
install -m644 70-soundblaster-x7.rules dist/x7-gui.desktop LICENSE README.md "$stage/"
install -m755 install.sh "$stage/install.sh"
tarball=x7-$version-x86_64-linux.tar.gz
tar -czf "$out/$tarball" -C "$out" "x7-$version"
(cd "$out" && sha256sum "$tarball" >"$tarball.sha256")
echo "$out/$tarball"
