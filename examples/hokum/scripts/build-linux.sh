#!/bin/sh
# Builds the Hokum Linux release tarball for this machine's architecture,
# dist/hokum-<version>-linux-<x64|arm64>.tar.gz, from a clean checkout of the
# rune workspace, and prints its path. The Hokum release workflow
# (.github/workflows/hokum-release.yml) publishes it on the GitHub release
# hokum-v<version>, and the AUR package hokum-bin installs from it.
#
# The tarball holds one directory, hokum-<version>-linux-<arch>/, with:
#   hokum                              the executable (/usr/bin/hokum)
#   hokum.service                      the systemd user unit
#   app.hokum.Notifications.service    the D-Bus activation file
#   LICENSE
#
# Environment: HOKUM_RELEASE_DIST (default <workspace>/dist), CARGO_TARGET_DIR.
set -eu

hokum=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
root=$(CDPATH='' cd -- "$hokum/../.." && pwd)
dist=${HOKUM_RELEASE_DIST:-"$root/dist"}
target_dir=${CARGO_TARGET_DIR:-"$root/target"}

for command in cargo file git jq tar gzip; do
  if ! command -v "$command" >/dev/null 2>&1; then
    echo "Linux builds require $command." >&2
    exit 1
  fi
done
case "$(uname -s) $(uname -m)" in
  "Linux x86_64") label=x64 elf='x86-64' ;;
  "Linux aarch64") label=arm64 elf='ARM aarch64' ;;
  *)
    echo "Linux releases must be built on x86_64 or aarch64 Linux." >&2
    exit 1
    ;;
esac
version=$(cargo metadata --no-deps --format-version 1 --manifest-path "$root/Cargo.toml" |
  jq -r '.packages[] | select(.name == "hokum") | .version')
if ! printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'; then
  echo "Release builds need an X.Y.Z version in examples/hokum/Cargo.toml, not $version." >&2
  exit 1
fi
if [ -n "$(git -C "$root" status --porcelain)" ]; then
  echo "Commit or remove all working-tree changes before building a release." >&2
  exit 1
fi

(cd "$root" && cargo build --locked --release -p hokum >&2)
executable="$target_dir/release/hokum"
file "$executable" | grep -E "ELF 64-bit.*$elf" >/dev/null

# Headless renders of the cards and the settings window: proves the executable starts,
# finds a Vulkan device (lavapipe in CI), and draws, without a compositor or D-Bus.
test "$("$executable" --version)" = "hokum $version"
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT HUP INT TERM
XDG_CONFIG_HOME="$temporary" "$executable" service --snap "$temporary/cards.png" --scene demo >&2
XDG_CONFIG_HOME="$temporary" "$executable" --snap "$temporary/settings.png" --scene hokum >&2
file "$temporary/cards.png" "$temporary/settings.png" | grep -c 'PNG image data' | grep -qx 2

name="hokum-$version-linux-$label"
stage="$temporary/$name"
mkdir -p "$stage" "$dist"
install -m755 "$executable" "$stage/hokum"
install -m644 "$hokum/packaging/hokum.service" "$hokum/packaging/app.hokum.Notifications.service" \
  "$hokum/packaging/hokum.desktop" \
  "$root/LICENSE" "$stage/"
# Reproducible: sorted names, no owners, and the commit's time.
epoch=$(git -C "$root" log -1 --format=%ct)
tarball="$dist/$name.tar.gz"
tar --sort=name --owner=0 --group=0 --numeric-owner --mtime="@$epoch" \
  -C "$temporary" -cf - "$name" | gzip -9n > "$tarball"
echo "$tarball"
