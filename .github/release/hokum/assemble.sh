#!/usr/bin/env bash
# assemble.sh <in> <dist>: gathers the Hokum release files from the verified
# artifacts in <in>/<name>/ into <dist>, refusing any file that is not a
# release file of HOKUM_VERSION or that two artifacts both hold, and writes
# <dist>/SHA256SUMS for the GitHub release.
. "$(dirname "$0")/../lib.sh"

in=$(posix "$1")
dist=$(posix "$2")
v=$HOKUM_VERSION
mkdir -p "$dist"
for file in "$in"/*/*; do
  name=$(basename "$file")
  [ "$name" = SHA256SUMS ] && continue
  case "$name" in
    "hokum-$v-linux-x64.tar.gz" | "hokum-$v-linux-arm64.tar.gz") ;;
    *) die "$name is not a release file of Hokum $v." ;;
  esac
  [ ! -e "$dist/$name" ] || die "Two artifacts hold $name."
  cp "$file" "$dist/$name"
done
for name in "hokum-$v-linux-x64.tar.gz" "hokum-$v-linux-arm64.tar.gz"; do
  [ -f "$dist/$name" ] || die "The release lacks $name."
done
(cd "$dist" && sha256 -- *.tar.gz > SHA256SUMS && ls -l && cat SHA256SUMS) | tee -a "${GITHUB_STEP_SUMMARY:-/dev/null}"
