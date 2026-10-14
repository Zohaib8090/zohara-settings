# Roadmap: Windows games and programs built into the OS (written 2026-10-14)

**Goal (owner's words, paraphrased):** no Steam in the middle. The compatibility layers (Proton-GE, Wine, DXVK) live in the
operating system; the person turns them on in Settings and plays by double-clicking. Everything about a game (graphics chip,
layer, options) is managed from Settings, for NVIDIA, AMD and Intel.

What exists today is in `WINDOWS-GAMES.md`. Everything below is **not built yet** unless it says so.

## Done (alpha only; nothing promoted to stable as of 2026-10-14)
* Gaming > Windows games: one-click support (multilib, Lutris, winetricks, 32-bit drivers), Proton-GE download/update/
  remove, ProtonUp-Qt. Gaming > Graphics in use: live load/memory/temperature per chip and which app is on which chip.
* My games: `.bat/.cmd/.msi/.lnk`, "Run with" (Automatic / Wine / Proton-GE / file itself), gear window per game (DXVK, graphics
  chip, Game Mode, MangoHud, options, variables).
* Gaming > Windows programs: double-click any Windows file with Proton-GE or Wine; a game from My games keeps its own settings
  and Windows folder (match by file, then folder). Verified on the owner's laptop with Cities: Skylines (log says
  `game "City Skylines"`, own prefix, NVIDIA chip, Game Mode and MangoHud on).

## Build order (re-arranged 2026-10-14, after the owner asked for Steam, Flatpak and Java games to be covered)
Order of work: each step is small, goes to alpha only, and ends with a stated test result.

### A. Steam cleanup  (DONE 2026-10-14, alpha)
* The setup button must not create a second Steam: install Arch `steam` only when no Flatpak Steam exists. (Owner removed
  the Flathub Steam on 2026-10-14 and keeps the Arch one; Proton-GE goes into whichever Steam folder exists.)
* Check that Steam lists Proton-GE in its compatibility menu (never verified).

### B. Read-only check with a Flatpak/Steam game running  (no code changes)
* "Graphics in use": are Flatpak/Steam games named correctly (sandbox names like `bwrap`)? Fix the name mapping.

### C. Per-game settings for Steam games  (gear button)  (BUILT 2026-10-14, alpha; tested on a fake library only)
Code: `src/backend/steam_games.rs` (VDF parser/writer, game list, launch options, Proton-GE per game, backup
`*.zohara-backup`, refuses while Steam runs), `src/pages/steam_settings.rs` (window), gear on Gaming > Your games > Steam.
Not yet tried on a real Steam game; Flatpak Steam hides Game Mode/MangoHud (step D). Original plan:
* Read installed Steam games (`libraryfolders.vdf`, `appmanifest_*.acf`) for the Arch Steam and, if present, the Flatpak Steam.
* Show them in My games with the same gear. Saving writes the game's launch options (`localconfig.vdf`, e.g.
  `gamemoderun mangohud %command%` plus chip variables). Steam must be closed while writing; Settings asks for that.

### D. Flatpak apps
* Chip choice per Flatpak app with `flatpak override --user --env=...`.
* One-click install of the Flatpak MangoHud and Game Mode extensions.
* Live view maps sandbox process names to the app name.

### E. Heroic and Lutris games  (BUILT 2026-10-14, alpha; not tried on a real Lutris/Heroic game)
* One small adapter each (Heroic per-game JSON, Lutris `.yml`) to list games and write the same settings.

### F. Java games (Minecraft, Modrinth/Prism/ATLauncher)  (new, from the owner's report)
* Gaming > Java: list installed Javas (and the ones launchers bring themselves, e.g. Modrinth's Zulu 25), show which version
  each Minecraft version needs (1.21+ Java 21, 1.18-1.20.4 Java 17, older Java 8), install/switch with `archlinux-java`,
  warn when a launcher's Java is too old for the profile.
* Chip choice and Game Mode for Minecraft launchers (Flatpak override or launcher env).
* First ask the owner for the actual error from the launcher log; do not guess.

### G. On by default after install  (BUILT 2026-10-14, alpha; VM test still to do)
Shipped: `data/zohara-windows.desktop` + `data/zohara-mimeapps.list` (package puts them in `/usr/share/applications`). The switch is "on unless the person turned it off" (off writes wine.desktop into their own mimeapps.list; on removes their lines and any private copy of the handler). Checked with the real `xdg-mime`: the system default wins over wine.desktop's registration and the person's own choice wins over the system default. NOT yet seen in Dolphin on a fresh install. Missing Proton-GE: runs with Wine and the notification has an "Install Proton-GE" button (opens Gaming settings) instead of an in-place dialog.
Today the double-click switch is off, and when turned on it writes `Exec=` with whatever binary was running (the test build in
`/tmp` on the owner's laptop). A fresh Zohara must work without anyone finding the switch.
* Ship `zohara-windows.desktop` in the `zohara-settings` package (`/usr/share/applications`, `Exec=/usr/bin/zohara-settings
  --run-windows %f`) and make it the default for the six Windows MIME types system-wide (check which defaults file Plasma 6
  honours and that it beats `wine.desktop`).
* The switch becomes "on unless the person turned it off"; `set_enabled(true)` prefers `/usr/bin/zohara-settings`.
* Proton-GE is not preinstalled (565 MB). On first double-click without it: dialog "Install Proton-GE now?" with progress.
* VM test (`scripts/vm/vm.py`; `~/zohara-vm` does not exist now): fresh install, double-click a harmless `.bat`, `.exe`, `.msi`.

### H. Say when a game fails  (BUILT 2026-10-14, alpha)
Per-game log `~/.cache/zohara/windows/<id|shared>.log`; a program that exits with an error within 10 s gives "did not start" with an "Open the log" button; "Show log" row in the gear window; My games > Play of a Windows program now goes through the same path. Limit: `wine start` returns 0 at once, so with plain Wine a failing game is not noticed; Proton is. Notification button tried only as plain notify-send (no press).
* Per-game log `~/.cache/zohara/windows/<game or shared>.log`; notify "<game> closed right away. Open the log?" if the process
  exits within ~10 s with a non-zero code; "Show log" button in the gear window.

### I. Finding installed programs and icons  (DONE 2026-10-14: real icons from the .exe, "Find Windows programs" button in My games (shared Windows folders, keeps using that folder); PART DONE earlier 2026-10-14: right-click menu "Run with Proton-GE / Wine / dedicated chip / Add as a game"; still to do: watching Program Files after an installer, real icons)
* Watch `drive_c/Program Files*` after a setup `.exe`, offer "Add these to My games?"; right-click "Run with Zohara…" and
  "Add as a game"; extract real icons from the `.exe` (icoutils).

### J. Safety for unknown programs  (BUILT 2026-10-14, alpha; see "Safety mode" in WINDOWS-GAMES.md; not the old restricted-mode-wip code, which stays parked for ordinary apps)
* bubblewrap sandbox: only its own Windows folder, the game folder and a "Windows Documents" folder; no-network option.
  Default ON for files that match no game. Starting point: the parked local branch `restricted-mode-wip`.
* Honest limit: it limits damage, it is not an antivirus; anti-cheat and some DRM will not run inside it.

### K. Per-game tools  (BUILT 2026-10-14, alpha; "duplicate settings from another game" not done)
Winetricks libraries, open the game's C: drive, reset its Windows folder, pick another Proton-GE version for this game.

### L. Later
Driver management (NVIDIA open vs proprietary, AMD Vulkan; VM first), GPU tuning (power limit, fans, gamescope/FSR, frame
limit; VM first), Proton-GE update notice (notify only), controller setup, per-game MangoHud presets.

## Known gaps in what is built
* The double-click switch is per user and points at one binary path (see 1).
* No Windows C: drive shortcut inside the per-game gear window yet (only on the Windows programs rows).
* Games that need Steam itself (Steam DRM) still need Steam; anti-cheat games do not run on Linux.
* Not tested: AMD graphics (only an Intel + NVIDIA laptop was available), MangoHud on a second machine, a fresh install.
* `a_running_program_is_noticed_by_its_process_name` fails inside the Docker test container only (it passes on the laptop);
  it is not related to this work.

## How to continue
Read this file and `WINDOWS-GAMES.md`, then start at item 1. Code: `src/backend/{proton,game_launch,windows_apps,gpu_live}.rs`,
`src/pages/{gaming,my_games}.rs`. Build and test in the `zs-img` container (see the `zohara-os-dev` skill); real-hardware checks
on the owner's laptop need the owner for any password prompt and for double-clicks.
