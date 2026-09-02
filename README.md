# zohara-settings

Zohara OS system settings application — GTK4 + libadwaita, talks to
NetworkManager / BlueZ / UPower over D-Bus.

## Build

Local:
```bash
cargo build --release
sudo install -Dm755 target/release/zohara-settings /usr/bin/zohara-settings
```

As an Arch package (testing only):
```bash
makepkg -si
```

## Distribution

This repo's CI builds the binary, packages it via `makepkg`, and
publishes the result to a release in
[`Zohaib8090/zohara-packages`](https://github.com/Zohaib8090/zohara-packages),
which is the Arch package repo that Zohara OS systems read with
`pacman -Syu`.

| Branch    | Channel |
|-----------|---------|
| `main`    | stable  |
| `beta`    | beta    |
| (manual)  | alpha   |

## Layout

```
src/
  main.rs              - app entry point
  backend/
    mod.rs
    dbus.rs            - D-Bus clients for NM, BlueZ, UPower
    network.rs
    process.rs         - pacman, bluetoothctl wrappers
    system.rs
  pages/
    mod.rs
    home.rs
    system.rs
    bluetooth.rs
    network.rs
    personalization.rs
    apps.rs
    accounts.rs
    time_language.rs
    gaming.rs
    accessibility.rs
    privacy.rs
    updates.rs
    advanced.rs        (deprecated, see home.rs)
data/
  zohara-settings.desktop
  zohara-settings.service
  win11.css            - Windows 11-style theme
PKGBUILD               - Arch package build script
.github/workflows/
  build.yml            - this repo's CI (build + publish)
```
