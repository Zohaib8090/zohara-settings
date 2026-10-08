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
  'zohara-keyring' # Zohara's package-signing key; a new package only reaches existing machines if an installed one needs it
  'polkit'         # the administrator prompt for system updates
  'curl'           # downloads the signed update approval list
  'fakeroot'       # lets the update check use a private copy of the package lists
  'libinput-tools' # the Mouse page's "Which one am I using?" listens to input devices with `libinput debug-events`
)
makedepends=(
  'rust'
  'cargo'
  'pkgconf'
)
optdepends=(
  'libnotify: problem notifications from the background health check'
  'wl-clipboard: voice typing'
  'ydotool: voice typing types into apps instead of copying'
  'networkmanager: Network settings page'
  'bluez: Bluetooth settings page'
  'upower: Power settings page'
)
# !strip: keep function names so a coredump (coredumpctl) has a readable backtrace.
options=(!debug !strip)
install=zohara-settings.install
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
    grep '^version' "$startdir/Cargo.toml" | head -1 | cut -d'"' -f2
  fi
}

build() {
  # Build happens in the workflow before makepkg is invoked; this stub is
  # here so makepkg's tarball-extraction step doesn't fail when there are
  # no sources.
  if [ ! -f "$startdir/target/release/zohara-settings" ]; then
    echo "ERROR: prebuilt binary not found at $startdir/target/release/zohara-settings"
    echo "The workflow is supposed to cargo build --release before running makepkg."
    return 1
  fi
}

package() {
  # Binary. startdir is the dir makepkg was invoked from (the workflow's
  # workspace root), where the prebuilt binary and the data/ directory live.
  install -Dm755 "$startdir/target/release/zohara-settings" \
    "$pkgdir/usr/bin/zohara-settings"

  # App icon: zohara-settings.desktop's Icon=zohara-settings resolves to
  # this, in the hicolor theme's standard scalable/apps location.
  install -Dm644 "$startdir/data/icons/scalable/apps/zohara-settings.svg" \
    "$pkgdir/usr/share/icons/hicolor/scalable/apps/zohara-settings.svg"

  # The Zohara logo for the start button (the hicolor theme is the fallback every icon set can see), and the
  # autostart that points the start menu at it once.
  install -Dm644 "$startdir/data/icons/scalable/apps/zohara-start.svg" \
    "$pkgdir/usr/share/icons/hicolor/scalable/apps/zohara-start.svg"
  install -Dm644 "$startdir/data/zohara-branding.desktop" "$pkgdir/etc/xdg/autostart/zohara-branding.desktop"
  install -Dm644 "$startdir/data/zohara-wallet-default.desktop" "$pkgdir/etc/xdg/autostart/zohara-wallet-default.desktop"

  # The terminal banner logo (copied over the ISO's old one by the install hook on machines installed earlier).
  install -Dm644 "$startdir/data/zohara-logo.txt" "$pkgdir/usr/share/zohara/zohara-logo.txt"

  # High contrast colour scheme for Accessibility > High contrast (this Plasma ships none).
  install -Dm644 "$startdir/data/color-schemes/ZoharaHighContrast.colors" "$pkgdir/usr/share/color-schemes/ZoharaHighContrast.colors"

  # The programs Zohara recommends after a computer was set up (Zohara Update offers the missing ones).
  install -Dm644 "$startdir/data/recommended.json" "$pkgdir/usr/share/zohara/recommended.json"

  # Desktop entry
  install -Dm644 "$startdir/data/zohara-settings.desktop" \
    "$pkgdir/usr/share/applications/zohara-settings.desktop"

  # Optional systemd user service (used by some distros for dbus activation)
  if [ -f "$startdir/data/zohara-settings.service" ]; then
    install -Dm644 "$startdir/data/zohara-settings.service" \
      "$pkgdir/usr/lib/systemd/user/zohara-settings.service"
  fi

  # Background health check (enabled per user by the ISO, or with
  # `systemctl --user enable --now zohara-settings-health.timer`).
  install -Dm644 "$startdir/data/zohara-settings-health.service"     "$pkgdir/usr/lib/systemd/user/zohara-settings-health.service"
  install -Dm644 "$startdir/data/zohara-settings-health.timer"     "$pkgdir/usr/lib/systemd/user/zohara-settings-health.timer"

  # Background check for system updates (notification only; turned on for every user by zohara-settings.install).
  install -Dm644 "$startdir/data/zohara-settings-update-check.service" "$pkgdir/usr/lib/systemd/user/zohara-settings-update-check.service"
  install -Dm644 "$startdir/data/zohara-settings-update-check.timer" "$pkgdir/usr/lib/systemd/user/zohara-settings-update-check.timer"

  # Automatic time zone (turned on for every user by zohara-settings.install) and the polkit rule that lets it run without a password.
  install -Dm644 "$startdir/data/zohara-settings-timezone.service" "$pkgdir/usr/lib/systemd/user/zohara-settings-timezone.service"
  install -Dm644 "$startdir/data/zohara-settings-timezone.timer" "$pkgdir/usr/lib/systemd/user/zohara-settings-timezone.timer"
  install -Dm644 "$startdir/data/50-zohara-timezone.rules" "$pkgdir/usr/share/polkit-1/rules.d/50-zohara-timezone.rules"

  # On-screen keyboard button for the taskbar, and the first-login setup that adds it on a touch screen computer.
  install -Dm644 "$startdir/data/zohara-keyboard.desktop" "$pkgdir/usr/share/applications/zohara-keyboard.desktop"
  install -Dm644 "$startdir/data/zohara-touch-setup.desktop" "$pkgdir/etc/xdg/autostart/zohara-touch-setup.desktop"

  # After sleep, if the Wi-Fi chip did not wake up, reload its driver (does nothing when Wi-Fi is fine).
  install -Dm755 "$startdir/data/zohara-wifi-resume" "$pkgdir/usr/lib/systemd/system-sleep/zohara-wifi-resume"

  # Voice typing (Meta+H). kglobalaccel picks up the default shortcut from
  # desktop files linked into its own directory.
  install -Dm644 "$startdir/data/zohara-dictation.desktop"     "$pkgdir/usr/share/applications/zohara-dictation.desktop"
  install -d "$pkgdir/usr/share/kglobalaccel"
  ln -s /usr/share/applications/zohara-dictation.desktop     "$pkgdir/usr/share/kglobalaccel/zohara-dictation.desktop"

  # Tray indicators for microphone / camera / location use (autostarted).
  install -Dm644 "$startdir/data/zohara-privacy-indicator.desktop" \
    "$pkgdir/etc/xdg/autostart/zohara-privacy-indicator.desktop"

  # License
  if [ -f "$startdir/LICENSE" ]; then
    install -Dm644 "$startdir/LICENSE" "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
  fi
}
