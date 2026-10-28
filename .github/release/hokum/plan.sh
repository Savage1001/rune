#!/usr/bin/env bash
# plan.sh: decides what this Hokum release run builds and publishes, from the
# event and the ref alone, and writes the plan to $GITHUB_OUTPUT.
#
#   check  builds, packages, and tests the AUR package, publishes nothing;
#          its tag job pushes and deletes a throwaway hokum-v0.0.0-test-<run>
#          tag to prove it can tag.
#   stable a dispatch from main: the crate version from
#          examples/hokum/Cargo.toml. Tags never start a release; the tag job
#          tags hokum-v<version> on the released commit and creates the GitHub
#          release, so the version must not be tagged yet.
#
# Environment: EVENT, REF_NAME, INPUT_REF, CHANNEL, SHA, GH_TOKEN, and
# examples/hokum/ of the commit in the working directory.
. "$(dirname "$0")/../lib.sh"

branch=main
version=$(awk '/^\[package\]/ { package = 1; next } /^\[/ { package = 0 } package && /^version = "/ { gsub(/^version = "|"$/, ""); print; exit }' examples/hokum/Cargo.toml)
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "Releases need an X.Y.Z version in examples/hokum/Cargo.toml, not $version."
[ "$EVENT" = workflow_dispatch ] || die "Hokum releases are dispatched."
case "$CHANNEL" in
  stable)
    if [ "$REF_NAME" != "$branch" ] || [ "$INPUT_REF" != "$branch" ]; then
      die "Stable releases are dispatched from $branch for ref $branch."
    fi
    tag="hokum-v$version"
    if gh api "repos/$GITHUB_REPOSITORY/git/matching-refs/tags/$tag" --jq '.[].ref' | grep -qx "refs/tags/$tag"; then
      die "Hokum $version is already tagged $tag; raise the version in examples/hokum/Cargo.toml first."
    fi
    [ -f "examples/hokum/docs/releases/$version.md" ] || die "Add release notes at examples/hokum/docs/releases/$version.md first."
    ;;
  check)
    tag="hokum-v0.0.0-test-$GITHUB_RUN_NUMBER"
    ;;
  *) die "Unknown channel $CHANNEL." ;;
esac
{
  echo "sha=$SHA"
  echo "version=$version"
  echo "channel=$CHANNEL"
  echo "tag=$tag"
} >> "$GITHUB_OUTPUT"
echo "Releasing Hokum $version ($CHANNEL) from $SHA as $tag." >> "$GITHUB_STEP_SUMMARY"
