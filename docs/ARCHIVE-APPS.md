# Apps from archives (`.tar.gz` and friends), 2026-10-20

Code: `src/backend/archive_install.rs` (all the logic, 12 tests), `src/pages/archive_apps.rs` (the group on Settings > Apps),
`data/zohara-archive-servicemenu.desktop` (right-click), `--install-archive FILE` in `src/main.rs`.

## What it does

An archive is not an installer, so Zohara looks inside first (`tar -tvf`, which reads gzip, xz, bzip2 and zstd) and decides what it is:

| Looks like | Kind | What Zohara does |
|---|---|---|
| a program with the "executable" flag at the top or in `bin/` (Firefox, Blender ...) | ready-to-run app | **installs it** (below) |
| `configure`, `CMakeLists.txt`, `Cargo.toml` ... with source files, or no program | source code | explains; points to the Store (Flathub, AUR) |
| a `PKGBUILD` | Arch build recipe | explains; the Store's AUR section builds it properly |
| `.pkg.tar.*` or `.PKGINFO` | Arch package | explains |
| nothing startable | ordinary files | explains; suggests the archive manager |

Source code is deliberately **not** built: a hand-built program is invisible to pacman, so it can be neither updated nor removed cleanly.

## Installing a ready-to-run app

* Unpacks to `~/.local/share/zohara/archive-apps/<slug>/` (the single top folder is stripped), `tar --no-same-owner --no-same-permissions`, into a staging folder first and then renamed into place.
* Main program: the best-ranked executable (named like the app, at the top or in `bin/`; libraries, installers, `chrome-sandbox` and the like are skipped); must start with an ELF header or `#!`.
* Writes `~/.local/share/applications/zohara-archive-<slug>.desktop` (name, quoted `Exec`, `Path`, icon found in the app or a generic one) and `.zohara-archive.json` (what to remove later).
* Name suggestion from the file name: `blender-4.2.0-linux-x64.tar.xz` becomes "Blender".
* **Remove** (Settings > Apps > Apps from archives) deletes the folder and the menu entry; only folders Zohara made (with the `.zohara-archive.json` file) can be removed or replaced.
* Nothing in the archive is run during install, and no system folder is touched (no sudo).

## Safety (checked on the listing, before anything is written)

Refused: absolute paths, `..`, symbolic or hard links that point outside the folder, more than 400,000 files, more than 16 GB unpacked, empty or unreadable archives.

## Ways in

1. Settings > Apps > **Apps from archives** > "Install from an archive…": file chooser, then a box with the name and (if several) the program to start with.
2. Right-click an archive > Zohara > **Install as an app** (`--install-archive FILE`, no window: installs with the suggested name and best-guess program, then a notification; other kinds get the explanation as a notification).

## Tested / not tested

Tested: 12 unit tests, including a **real** `tar` archive installed, started (it printed its message), listed and removed; foreign folders are never removed; a broken file says so.
The real command (`--install-archive`) was run on a throwaway data folder: a ready-to-run app installed (menu entry and files checked, the program ran); a source-code archive, a photo archive, a broken file and a `../../` archive all exited with 1 and installed nothing.
The Apps page group was rendered in a hidden desktop with the installed app showing.

**Not tested:** clicking the file chooser, the install box and the Remove confirmation (no safe way to send clicks to the hidden desktop; do it in the VM); the right-click entry inside Dolphin; the Start menu actually showing the new entry; large real-world archives (Firefox, Blender); `.AppImage` inside a tarball; apps that need a terminal (they would start with no window).

## Known limits / ideas

* Electron apps with a `chrome-sandbox` that must be setuid will not sandbox (no sudo); they usually still run with `--no-sandbox`, which is not added automatically.
* Command-line tools extract and get a menu entry but open no window (`Terminal=false`); a "needs a terminal" choice could be added.
* Updating means installing the new archive over the old one under the same name (it replaces the folder).
* Part of the "run anything" plan: `zohara/docs/RUN-ANYTHING-MACOS-IOS-2026-10-19.md`.
