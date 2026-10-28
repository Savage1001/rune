# Releasing Hokum

Hokum releases for Linux x86_64 and aarch64 from one commit of the rune
repository, through `.github/workflows/hokum-release.yml` (**Hokum release**).
It follows Hex's release (anomalyco/hex `docs/releasing.md`,
`.github/workflows/release-desktop.yml`): build jobs run repository code
without secrets, the publish jobs check out only `.github/release/` and run
only those scripts on digest-checked artifacts, and the workflow tags last.
Files go to the GitHub release, as OpenCode's do, since Hokum has no release
service registration.

```
plan                 .github/release/hokum/plan.sh: version, channel, tag
├─ linux-x64         examples/hokum/scripts/build-linux.sh on ubuntu-24.04        [build]
├─ linux-arm64       the same on ubuntu-24.04-arm                                 [build]
├─ aur-stage         examples/hokum/scripts/release-aur.sh stage + test, namcap,
│                    pacman -U and a render in archlinux:base-devel               [build]
├─ github            assemble.sh, tag-release.sh hokum-v<version>,
│                    publish-github-release.sh (GITHUB_TOKEN only)
└─ aur               push-aur.sh hokum-bin (AUR_SSH_PRIVATE_KEY, env release)     [stable]
```

## Files

- `hokum-<version>-linux-<x64|arm64>.tar.gz`: one directory,
  `hokum-<version>-linux-<arch>/`, with `hokum`, `hokum.service`,
  `app.hokum.Notifications.service`, and `LICENSE`. Reproducible: sorted,
  owner 0, the commit's time.
- `SHA256SUMS` for both tarballs.

## Channels

- `check` (the default) builds and tests everything, publishes nothing, and
  pushes and deletes a throwaway `hokum-v0.0.0-test-<run>` tag to prove the
  token can tag.
- `stable`, dispatched from `main` for ref `main`, publishes the version in
  `examples/hokum/Cargo.toml`. It needs release notes at
  `docs/releases/<version>.md` and refuses a version already tagged
  `hokum-v<version>`. Tags trigger nothing.

## Cutting a release

1. Raise `version` in `examples/hokum/Cargo.toml` (X.Y.Z), refresh
   `Cargo.lock` (`cargo check -p hokum`), and add `docs/releases/<version>.md`.
   Merge to `main`.
2. `gh workflow run hokum-release.yml --ref main -f ref=main -f channel=check`
   and wait for it to pass.
3. `gh workflow run hokum-release.yml --ref main -f ref=main -f channel=stable`.
4. Check `https://github.com/anomalyco/rune/releases/tag/hokum-v<version>` and
   `https://aur.archlinux.org/packages/hokum-bin`.

If the AUR job fails after the GitHub release exists, fix the cause and rerun
the failed jobs of the same run: the tag and release steps accept what they
already published.

## AUR

`packaging/aur/PKGBUILD` is the template for `hokum-bin`. `release-aur.sh
stage` fills in the version and both tarballs' SHA-256 and writes
`.SRCINFO`; `release-aur.sh test` builds it with `makepkg`. The package
installs `/usr/bin/hokum`, `/usr/lib/systemd/user/hokum.service`,
`/usr/share/dbus-1/services/app.hokum.Notifications.service` (both run
`hokum service`), `/usr/share/applications/hokum.desktop` (plain `hokum`, the
settings window), and the license.

It provides `hokum` and `notification-daemon` (as dunst and xfce4-notifyd do)
and conflicts only with `hokum`. Like mako, dunst, and swaync, the D-Bus
activation file has the daemon's own name rather than
`org.freedesktop.Notifications.service`, so it shares no file with any other
daemon. The running daemon owns the bus name; `hokum.service` has
`Conflicts=mako.service dunst.service swaync.service`.

`push-aur.sh` refuses to push unless both sources in `.SRCINFO` are this
version's GitHub release files and serve the pinned SHA-256.

Local test on Arch, from a clean checkout:

```sh
dist=$(mktemp -d)
HOKUM_RELEASE_DIST=$dist examples/hokum/scripts/build-linux.sh
# Without an aarch64 build, stage the x64 tarball under the arm64 name too.
examples/hokum/scripts/release-aur.sh stage <version> $dist/hokum-<version>-linux-x64.tar.gz <arm64 tarball> $dist/aur
examples/hokum/scripts/release-aur.sh test $dist/aur
```

## Secrets and setup

| Name | Where | Use |
| --- | --- | --- |
| `AUR_SSH_PRIVATE_KEY` | secret, environment `release` | pushes `hokum-bin`; check it with **Hokum AUR key check** |
| `GITHUB_TOKEN` | built in | tags and creates the GitHub release (`contents: write` in the `github` job only) |

- Create the `release` environment in anomalyco/rune, limited to `main`.
- Add the AUR account's SSH public key (Hex uses `github-actions-hex-aur` on
  Dax's account) and store the private key as `AUR_SSH_PRIVATE_KEY`. The
  first push creates `hokum-bin` on the AUR under that account.
- Let the workflow push `hokum-v*` tags past any tag ruleset.
