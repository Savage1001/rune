#!/usr/bin/env bash
# publish-github-release.sh <dist>: creates the GitHub release HOKUM_RELEASE_TAG
# (which tag-release.sh pushed at the release commit; this never creates the
# tag) with the release notes, uploads the tarballs and SHA256SUMS, and checks
# that GitHub serves them byte for byte. Kept off "Latest": the rune
# repository's releases are not all Hokum's.
#
# Environment: HOKUM_VERSION, HOKUM_RELEASE_COMMIT, HOKUM_RELEASE_TAG,
# HOKUM_RELEASE_NOTES (the notes file), GITHUB_REPOSITORY, GH_TOKEN.
. "$(dirname "$0")/../lib.sh"

dist=$(posix "${1:?Usage: publish-github-release.sh <dist>}")
repo=$GITHUB_REPOSITORY
version=${HOKUM_VERSION:?Set HOKUM_VERSION}
commit=${HOKUM_RELEASE_COMMIT:?Set HOKUM_RELEASE_COMMIT}
tag=${HOKUM_RELEASE_TAG:?Set HOKUM_RELEASE_TAG}
notes=${HOKUM_RELEASE_NOTES:?Set HOKUM_RELEASE_NOTES}
[ -f "$notes" ] || die "Add release notes at $notes."

assets=(SHA256SUMS "hokum-$version-linux-x64.tar.gz" "hokum-$version-linux-arm64.tar.gz")
for name in "${assets[@]}"; do [ -f "$dist/$name" ] || die "No $name in $dist."; done
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT

tagged=$(gh api "repos/$repo/git/ref/tags/$tag" --jq .object.sha)
[ "$tagged" = "$commit" ] || die "Tag $tag names $tagged, not $commit."
if ! gh release view "$tag" -R "$repo" --json name > /dev/null 2>&1; then
  gh release create "$tag" -R "$repo" --verify-tag --title "Hokum $version" \
    --notes-file "$notes" --latest=false
fi
gh release view "$tag" -R "$repo" --json assets > "$temporary/release.json"
mkdir -p "$temporary/mirror"
for name in "${assets[@]}"; do
  if ! jq -e --arg name "$name" '.assets | any(.name == $name)' "$temporary/release.json" > /dev/null; then
    gh release upload "$tag" "$dist/$name" -R "$repo"
  fi
  gh release download "$tag" -R "$repo" --pattern "$name" --dir "$temporary/mirror" --clobber
  cmp -s "$dist/$name" "$temporary/mirror/$name" || die "GitHub's $name differs from the release bytes."
  rm -f "$temporary/mirror/$name"
done
echo "https://github.com/$repo/releases/tag/$tag"
