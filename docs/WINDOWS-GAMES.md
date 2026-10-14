# Windows games (Settings > Gaming > Windows games), 2026-10-14

> What comes next: `ROADMAP-WINDOWS-GAMES.md`. Everything here is on the **alpha** channel only.

Owner's ask: Windows games that work out of the box, with Proton-GE and ProtonUp-Qt built into the OS, inside the
existing Gaming page (not a page of its own).

## What the person sees
Three rows in a "Windows games" group on the Gaming page:
1. **Windows game support** (button "Set up", then "Open Steam" only if a Steam exists). One administrator prompt. Turns on `[multilib]`, installs
   `lutris winetricks lib32-gamemode lib32-mangohud lib32-mesa` plus the 32-bit driver for the graphics chip found
   Steam is installed only when no Flatpak Steam exists (one Steam only, 2026-10-14).
   (`lib32-nvidia-utils`, `lib32-vulkan-radeon`, `lib32-vulkan-intel`; if none is recognised, both Vulkan drivers), then
   ProtonUp-Qt (from Chaotic-AUR, allowed to fail).
   It **refuses while updates are waiting** (exit 6, "Update the computer first"), because `pacman -Sy` on a computer that is
   behind and then installing 32-bit libraries would mix old and new libraries.
2. **Proton-GE** ("Install" / "Update" / "Check for newer"). Looks up the newest release, downloads it with a progress
   line ("Downloading GE-Proton11-9: 230 of 564 MB"), checks the published SHA-512, unpacks it. One row per installed
   version with Remove.
3. **ProtonUp-Qt** ("Install" / "Open"): pacman package, Flatpak `net.davidotek.pupgui2` as the fallback.

## How Proton-GE is installed (decision: download on demand, not a package)
No repository carries it and the archive is 565 MB, so it is **not** shipped in `zohara-packages` (that would re-upload
565 MB per release to every channel and grow the ISO by the same). `backend/proton.rs`:
* release JSON from `api.github.com/repos/GloriousEggroll/proton-ge-custom/releases/latest`; only
  `GE-Proton*` tags made of letters, digits, `-` and `.` and only downloads from that project's GitHub are accepted;
* `GE-ProtonN-N-x86_64.tar.gz` and its `.sha512sum`; install is refused unless the hashes match;
* unpacked to `.<tag>.part` next to the target and renamed, so Steam never sees half a version. The archive's top folder
  is named `GE-ProtonN-N-x86_64`; it is renamed to the plain tag;
* target: `~/.local/share/Steam/compatibilitytools.d` and, if the Flatpak Steam is there,
  `~/.var/app/com.valvesoftware.Steam/.local/share/Steam/compatibilitytools.d`. No administrator needed. Steam shows it in
  a game's Properties > Compatibility the next time Steam starts.

## ISO and installed systems
* `zohara-profile/pacman.conf`: `[multilib]` on. `customize_airootfs.sh` turns it on in the installed system's
  `/etc/pacman.conf` (uncomment, or append). Steam and the 32-bit libraries are **not** preinstalled (they depend on the
  graphics chip; the button installs them). **The ISO was not rebuilt**: new installs get this only from the next ISO.
* Machines that already exist have no multilib until the person presses Set up (the script enables it).

## Verified
* 9 unit tests in `backend/proton.rs` (release parsing and refusal of odd tags/URLs, checksum file, version order,
  Steam folder discovery, multilib sed edit, a fake archive unpacked from a `file://` URL and a wrong checksum refused).
* **Real download**: `cargo test live_proton_ge -- --ignored` fetched GE-Proton11-9 (564,697,280 bytes), checked the
  SHA-512, unpacked, listed and removed it.
* The setup script ran in an Arch container: appended `[multilib]`, synced, and exited 6 because that image was behind.
  Every package name in it exists in `core/extra/multilib`.
* The Gaming page was rendered under Xvfb (rows, buttons).

## Not verified
* The setup button on a real, up-to-date machine (pkexec, steam + lib32 install). 
* A real game starting with Proton-GE selected in Steam; anti-cheat games do not work on Linux.
* Steam's Flatpak seeing the folder (only the path is the documented one).
* ProtonUp-Qt install through pkexec; its Flatpak fallback.

## Found on the owner's laptop (2026-10-14)
First real run of "Set up" failed (error 1): Zohara ships its own `/etc/lsb-release` (branding, owned by no package) and Lutris
pulls in the `lsb-release` package, so pacman stopped on "exists in filesystem". The script now copies Zohara's file aside,
installs with `--overwrite /etc/lsb-release`, and puts Zohara's file back. The script also writes what pacman said to
`/var/log/zohara-games-setup.log` and the error dialog shows the `error:` lines. Second run: steam, lutris, winetricks,
lib32-mesa, lib32-nvidia-utils (Intel + NVIDIA laptop, so also lib32-vulkan-radeon/intel by the fallback rule) and protonup-qt
installed; `/etc/lsb-release` still says Zohara OS. Any other ISO file that a dependency also ships would fail the same way.

**Second finding (same laptop):** Proton-GE was already in the Flatpak Steam folder (put there by ProtonUp-Qt's Flatpak), so the
button thought it was installed and did nothing for the native Steam. Install now checks every Steam folder (`missing_in`,
`fully_installed`) and downloads only into the ones that lack the version. Result: GE-Proton11-9 unpacked into
`~/.local/share/Steam/compatibilitytools.d` on the laptop, download file removed. Steam's own internal name for it is
`GE-Proton11-9-x86_64` (from the archive's `compatibilitytool.vdf`).

## My games: .bat files and "Run with" (2026-10-14)
Owner double-clicked a game's `Run Me!.bat` and it ran through Wine, so "My games" now handles that too. The add-a-game
window has **Run with**: *Automatic* (a .exe goes to `wine`; .bat, .cmd, .msi and .lnk go to `wine start /unix`; anything
else runs as it is), *Windows app support (Wine)* (any file at all, even without a known ending) and *Run the file
itself*. The launcher's working folder is the file's folder, which .bat files need. Verified with real Wine: a `.bat` with
spaces and `!` in its name, started with `wine start /unix`, wrote its output file next to itself. Plain Wine, not Proton-GE.

## Live graphics and per-game settings (2026-10-14)
**Gaming > Graphics in use** (`backend/gpu_live.rs`): each chip's name (from `/usr/share/hwdata/pci.ids`, no `lspci`), load,
video memory and temperature (NVIDIA: one `nvidia-smi` call; AMD: `/sys`; Intel: current MHz), and a list of apps with the chip
each uses. **A sleeping chip is never woken**: `power/runtime_status` is read first and a sleeping chip shows "Asleep"
(waking the NVIDIA chip costs battery and froze the page before; see `gpu.rs`). Refreshes every 2 s only while the page is on
screen. Which chip an app uses comes from the device files it has open: `/dev/nvidiaN` wins, else the render node, preferring
the chip that is not the screen chip. Child processes whose parent is listed are dropped (Electron helpers, `node`), desktop
parts and Wine helper processes are skipped. Checked on the owner's laptop: RTX 3050 (53 MiB, 64 °C) and Intel UHD (600 MHz).

**My games > gear button** (`backend/game_launch.rs`, `pages/my_games.rs`): per game: Run with (Automatic, Windows app support
(Wine), Proton-GE, Run the file itself), DXVK on/off (Proton: `PROTON_USE_WINED3D=1` when off), Graphics chip (Automatic /
Dedicated: the `switcherooctl` variables are put in the launch line), Game Mode, MangoHud, game options and environment
variables. The settings are stored in the game's own desktop entry as `X-Zohara-*` keys and the `Exec=` line is rebuilt on
Save. Typed text is split into words and every word is quoted; variable names are checked. Games that were only *marked* from
an installed app have no gear (they keep their own launcher). Proton-GE per game uses a private Windows folder,
`~/.local/share/zohara/prefixes/<game>`.
Verified: unit tests (command building, quoting, round trip), real GE-Proton11-9 on the owner's laptop ran
`proton run start /unix file.bat` (wrote its output file) and `proton run file.exe` with a private prefix. Not verified:
a real game under Proton-GE from the gear window, the Dedicated switch on an AMD laptop, MangoHud (not installed on this laptop).

## Windows programs with a double-click, no Steam (2026-10-14)
Owner's goal: the compatibility layer is part of the OS. Turn it on once in Settings, then double-click any Windows game or
program and it runs; Steam is not needed in the background.

Gaming > **Windows programs**: "Open Windows programs with a double-click" switch, Compatibility layer (Proton-GE,
recommended / Wine), DXVK, stronger graphics chip, Game Mode, MangoHud, and "Windows C: drive > Open".
* The switch writes `~/.local/share/applications/zohara-windows.desktop` (`Exec=zohara-settings --run-windows %f`,
  NoDisplay) and makes it the default for `application/x-ms-dos-executable`, `x-msdownload`, `x-msi`, `x-ms-shortcut`,
  `x-bat`, `x-mswinurl` with `xdg-mime default` (the person's own `~/.config/mimeapps.list`; no password). Off takes only our
  entry out of `mimeapps.list` and deletes the desktop file, so Wine's own handler is back.
* `--run-windows FILE` (`backend/windows_apps.rs`) reads `~/.config/zohara/windows-apps.conf`, starts the file from its own
  folder and returns. Proton-GE runs **directly** (`proton run`, plus `start /unix` for non-.exe files) with
  `STEAM_COMPAT_DATA_PATH=~/.local/share/zohara/windows` (one shared Windows folder, so installed programs find each other) and
  `STEAM_COMPAT_CLIENT_INSTALL_PATH=~/.local/share/Steam` (created if missing; Steam itself is not needed). Plain Wine uses its
  own folder `~/.local/share/zohara/windows-wine` (Proton's and Wine's must not be shared). If Proton-GE is chosen but not
  installed, it falls back to Wine and says so in a notification. Output goes to `~/.cache/zohara/windows-run.log`.
* Verified on the owner's laptop: `zohara-settings --run-windows "…/Game Folder/Run Me!.bat"` (throwaway data folder)
  created the Proton prefix with GE-Proton11-9 and the .bat wrote its result next to itself.
* Known limits: Proton turns off Wine's start-menu shortcuts, so an installed program is found through "Windows C: drive";
  anti-cheat games do not run; a game that needs Steam itself (Steam DRM) still needs Steam.

### A double-click follows the game's own settings (2026-10-14)
`--run-windows FILE` first looks for a game in My games (`known_games`, `match_game` in `backend/windows_apps.rs`): the game
whose added file is `FILE`, else the game whose folder holds `FILE` (closest folder wins). A game added from a broad folder
(home, `/`, Downloads, Desktop, Documents, Music, Pictures, Videos, Public) matches by its exact file only. A matched game
runs with its own gear settings and its own Windows folder (`~/.local/share/zohara/prefixes/<game>`), the same as its Play
button; anything else uses the shared defaults and shared C: drive. The first line of `~/.cache/zohara/windows-run.log` says
which one was used (`game "<name>"` or `shared defaults`). Verified with the real GE-Proton11-9: a different `.bat` in a
registered game's folder ran with that game's Proton + dedicated-chip variables and its own prefix.

## Steam games (2026-10-14)
Gaming > Your games > Steam: each game has a gear. It edits Steam's own files (`userdata/<id>/config/localconfig.vdf`
launch options, `config/config.vdf` Proton tool), only while Steam is closed, and keeps a `.zohara-backup` copy of each
file the first time. Launch options look like `env DRI_PRIME=1 FOO=bar gamemoderun mangohud %command% -windowed`;
options Zohara did not write are shown and only replaced when the person saves. Tested with a fake library in the
container (list, round trip, Save writes the file); not with a real Steam game yet.
