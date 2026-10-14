# Roadmap: Windows games and programs built into the OS (written 2026-10-14)

**Goal (owner's words, paraphrased):** no Steam in the middle. The compatibility layers (Proton-GE, Wine, DXVK) live in the
operating system; the person turns them on in Settings and plays by double-clicking. Everything about a game (graphics chip,
layer, options) is managed from Settings, for NVIDIA, AMD and Intel.

What exists today is in `WINDOWS-GAMES.md`. Everything below is **not built yet** unless it says so.

## Done (alpha only; nothing promoted to stable as of 2026-10-14)
* Gaming > Windows games: one-click support (multilib, Steam, Lutris, winetricks, 32-bit drivers), Proton-GE download/update/
  remove, ProtonUp-Qt. Gaming > Graphics in use: live load/memory/temperature per chip and which app is on which chip.
* My games: `.bat/.cmd/.msi/.lnk`, "Run with" (Automatic / Wine / Proton-GE / file itself), gear window per game (DXVK, graphics
  chip, Game Mode, MangoHud, options, variables).
* Gaming > Windows programs: double-click any Windows file with Proton-GE or Wine; a game from My games keeps its own settings
  and Windows folder (match by file, then folder). Verified on the owner's laptop with Cities: Skylines (log says
  `game "City Skylines"`, own prefix, NVIDIA chip, Game Mode and MangoHud on).

## Next, in this order

### 1. On by default after install  (do first; needs a VM test)
Today the double-click switch is off, and when turned on it writes `Exec=` with whatever binary was running (the test build in
`/tmp` on the owner's laptop). A fresh Zohara must work without anyone finding the switch.
* Ship `zohara-windows.desktop` in the `zohara-settings` package (`/usr/share/applications`, `Exec=/usr/bin/zohara-settings
  --run-windows %f`) and make it the default for the six Windows MIME types system-wide (`/usr/share/applications/mimeapps.list`
  or the KDE-supported defaults file; check which one Plasma 6 honours and that it beats `wine.desktop`).
* The switch becomes "on unless the person turned it off"; turning it off still only writes the person's own `mimeapps.list`.
* `set_enabled(true)` must prefer `/usr/bin/zohara-settings` (done only when it exists and the running binary is not in `/tmp`).
* Proton-GE is not preinstalled (565 MB). On first double-click without it: today a notification and plain Wine. Better: a
  dialog "Install Proton-GE now?" with progress, then run.
* Test in the VM (`scripts/vm/vm.py`; `~/zohara-vm` does not exist now, create one): fresh install, double-click a harmless
  `.bat`, `.exe`, `.msi`.
* Not building an ISO unless the owner asks (rule); multilib is already on in the ISO profile.

### 2. Say when a game fails  (small; do with 1)
* Per-game log: `~/.cache/zohara/windows/<game or shared>.log` (today one shared `windows-run.log`, overwritten each run).
* Watch the started process: if it exits within ~10 s with a non-zero code, notify "<game> closed right away. Open the log?"
  with an action that opens the log. Keep the first log line `Zohara: game "<name>" with …`.
* Settings: "Show log" button in the gear window.

### 3. Finding installed programs and icons
* After a setup `.exe`, Proton makes no menu entries. Watch `drive_c/Program Files*` and `drive_c/users/*/AppData` of the shared
  and per-game Windows folders; offer "Add these to My games?" (notification action or a "New programs" row).
* Right-click service menu on `.exe`/`.bat`: "Run with Zohara…" (pick layer/chip for this launch) and "Add as a game".
* Real icons: extract from the `.exe` (`wrestool`/`icotool` from icoutils) into `~/.local/share/icons/hicolor/…`.

### 4. Safety for unknown programs  (important; double-click lowers the barrier)
Wine and Proton can read and delete everything in the person's home. The owner already ran a cracked-game archive.
* Sandbox option (bubblewrap): program sees only its own Windows folder, the game folder and a "Windows Documents" folder;
  no network optional. Default ON for files that match no game in My games; per-game switch for games.
* Starting point: the parked local branch `restricted-mode-wip` in `zohara-settings` (not pushed; see
  `zohara-os-state-2026-10-13.md` memory). Check what it already does before writing new code.
* Honest limit to tell the person: it limits damage, it is not an antivirus; anti-cheat and some DRM will not run inside it.

### 5. Per-game tools in the gear window
Winetricks libraries (vcredist, dotnet, corefonts), open the game's C: drive, reset its Windows folder, choose another Proton-GE
version for this game (today: newest installed), duplicate settings from another game.

### 6. Later
* **Driver management** (earlier item 3): install/switch NVIDIA open vs proprietary and the AMD Vulkan drivers, with warnings;
  VM-test first, never on the owner's laptop first.
* **GPU tuning** (earlier item 4): power limit, fan curve, gamescope/FSR upscaling and frame limit. Riskier; VM-test.
* Proton-GE update notice (notify only, never automatic; honour the update pause).
* Matching a Steam/Heroic/Lutris game's own settings (today only My games has the gear).
* Controller setup and per-game MangoHud presets.

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
