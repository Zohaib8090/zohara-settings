# Maintainer: Zohara OS Team <https://github.com/Zohaib8090/zohara>
# PKGBUILD for zohara-settings
#
# This file is used by the GitHub Actions workflow (.github/workflows/build.yml)
# to package the freshly-compiled binary into a proper Arch package suitable
# for `pacman -Syu` from the [zohara-stable] / [zohara-beta] / [zohara-alpha]
# repos hosted at https://github.com/Zohaib8090/zohara-packages/releases.
#
# Local install (for testing): makepkg -si
# CI: the workflow runs `makepkg --nocheck` and uploads the .pkg.tar.zst.

pkgname=zohara-settings
pkgver=0.1.0
pkgrel=1
pkgdesc="Zohara OS system settings application (GTK4 + libadwaita)"
arch=('x86_64')
url="https://github.com/Zohaib8090/zohara-settings"
license=('GPL-3.0-or-later')
depends=(
  'gtk4'
  'libadwaita'
  'glib2'
  'dbus'
)
makedepends=(
  'rust'
  'cargo'
  'pkgconf'
)
optdepends=(
  'networkmanager: Network settings page'
  'bluez: Bluetooth settings page'
  'upower: Power settings page'
)
provides=('zohara-settings')
conflicts=('zohara-settings-git')

# Source: the workflow has already cloned this repo at $srcdir. The binary
# is prebuilt at ../target/release/zohara-settings so we don't re-cargo here
# (the workflow does that with proper caching).
source=()
sha256sums=()

pkgver() {
  # The workflow may override pkgver before calling makepkg by writing
  # _PKGVER into the environment. Use that if present, otherwise read from
  # Cargo.toml.
  if [ -n "${_PKGVER:-}" ]; then
    echo "$_PKGVER"
  else
    grep '^version' Cargo.toml | head -1 | cut -d'"' -f2
  fi
}

build() {
  # Build happens in the workflow before makepkg is invoked; this stub is
  # here so makepkg's tarball-extraction step doesn't fail when there are
  # no sources.
  if [ ! -f "$srcdir/../target/release/zohara-settings" ]; then
    echo "ERROR: prebuilt binary not found at $srcdir/../target/release/zohara-settings"
    echo "The workflow is supposed to cargo build --release before running makepkg."
    return 1
  fi
}

package() {
  # Binary
  install -Dm755 "$srcdir/../target/release/zohara-settings" \
    "$pkgdir/usr/bin/zohara-settings"

  # Desktop entry
  install -Dm644 "$srcdir/data/zohara-settings.desktop" \
    "$pkgdir/usr/share/applications/zohara-settings.desktop"

  # Optional systemd user service (used by some distros for dbus activation)
  if [ -f "$srcdir/data/zohara-settings.service" ]; then
    install -Dm644 "$srcdir/data/zohara-settings.service" \
      "$pkgdir/usr/lib/systemd/user/zohara-settings.service"
  fi

  # License
  if [ -f "$srcdir/LICENSE" ]; then
    install -Dm644 "$srcdir/LICENSE" "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
  fi
}
