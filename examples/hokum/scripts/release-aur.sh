#!/bin/sh
# Stages and tests the AUR package hokum-bin from a release's Linux tarballs,
# without secrets. Runs on Arch (CI uses the archlinux container). The Hokum
# release workflow's AUR job pushes the staged files with
# .github/release/hokum/push-aur.sh, which holds the AUR key.
#
#   release-aur.sh stage <version> <x64 tarball> <arm64 tarball> <dir>
#   release-aur.sh test  <dir>    # makepkg on this machine's architecture
#
# The PKGBUILD downloads the tarballs from the GitHub release
# hokum-v<version>; stage copies them in under their source names, so `test`
# downloads nothing and works before the release exists.
set -eu

hokum=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
package='hokum-bin'
usage() {
  echo "Usage: $0 stage <version> <x64 tarball> <arm64 tarball> <dir> | test <dir>" >&2
  exit 1
}
[ $# -ge 1 ] || usage
mode=$1
shift

sha256_of() {
  sha256sum < "$1" | awk '{ print $1 }'
}

case "$mode" in
  stage)
    [ $# -eq 4 ] || usage
    version=$1 x64=$2 arm64=$3 dir=$4
    printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'
    [ "$(basename "$x64")" = "hokum-$version-linux-x64.tar.gz" ] || usage
    [ "$(basename "$arm64")" = "hokum-$version-linux-arm64.tar.gz" ] || usage
    for tarball in "$x64" "$arm64"; do
      gzip -t "$tarball"
    done
    mkdir -p "$dir"
    cp "$hokum/packaging/aur/$package.install" "$dir/"
    cp "$x64" "$arm64" "$dir/"
    sed -e "s/@VERSION@/$version/" \
      -e "s/@SHA256_X64@/$(sha256_of "$x64")/" \
      -e "s/@SHA256_ARM64@/$(sha256_of "$arm64")/" \
      "$hokum/packaging/aur/PKGBUILD" > "$dir/PKGBUILD"
    if grep -q '@[A-Z0-9_]*@' "$dir/PKGBUILD"; then
      echo "PKGBUILD has an unfilled placeholder." >&2
      exit 1
    fi
    (cd "$dir" && makepkg --printsrcinfo > .SRCINFO)
    echo "$dir"
    ;;
  test)
    [ $# -eq 1 ] || usage
    dir=$1
    (cd "$dir" && makepkg --force --nodeps --noconfirm >&2)
    built=$(cd "$dir" && makepkg --packagelist)
    test -f "$built"
    for file in usr/bin/hokum usr/lib/systemd/user/hokum.service \
      usr/share/dbus-1/services/app.hokum.Notifications.service \
      usr/share/applications/hokum.desktop \
      "usr/share/licenses/$package/LICENSE"; do
      bsdtar -tf "$built" | grep -qx "$file" || {
        echo "$built lacks $file." >&2
        exit 1
      }
    done
    echo "$built"
    ;;
  *) usage ;;
esac
