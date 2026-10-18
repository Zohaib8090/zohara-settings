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
  installed, it falls back to Wine and says so in a notification. Output goes to `~/.cache/zohara/windows/<game id or shared>.log`.
* Verified on the owner's laptop: `zohara-settings --run-windows "…/Game Folder/Run Me!.bat"` (throwaway data folder)
  created the Proton prefix with GE-Proton11-9 and the .bat wrote its result next to itself.
* Known limits: Proton turns off Wine's start-menu shortcuts, so an installed program is found through "Windows C: drive";
  anti-cheat games do not run; a game that needs Steam itself (Steam DRM) still needs Steam.

### A double-click follows the game's own settings (2026-10-14)
`--run-windows FILE` first looks for a game in My games (`known_games`, `match_game` in `backend/windows_apps.rs`): the game
whose added file is `FILE`, else the game whose folder holds `FILE` (closest folder wins). A game added from a broad folder
(home, `/`, Downloads, Desktop, Documents, Music, Pictures, Videos, Public) matches by its exact file only. A matched game
runs with its own gear settings and its own Windows folder (`~/.local/share/zohara/prefixes/<game>`), the same as its Play
button; anything else uses the shared defaults and shared C: drive. The first line of `~/.cache/zohara/windows/<game id or shared>.log` says
which one was used (`game "<name>"` or `shared defaults`). Verified with the real GE-Proton11-9: a different `.bat` in a
registered game's folder ran with that game's Proton + dedicated-chip variables and its own prefix.

## Steam games (2026-10-14)
Gaming > Your games > Steam: each game has a gear. It edits Steam's own files (`userdata/<id>/config/localconfig.vdf`
launch options, `config/config.vdf` Proton tool), only while Steam is closed, and keeps a `.zohara-backup` copy of each
file the first time. Launch options look like `env DRI_PRIME=1 FOO=bar gamemoderun mangohud %command% -windowed`;
options Zohara did not write are shown and only replaced when the person saves. Tested with a fake library in the
container (list, round trip, Save writes the file); not with a real Steam game yet.

## Right-click menu (2026-10-14)
`data/zohara-windows-servicemenu.desktop` (installed to `/usr/share/kio/servicemenus/`) adds Zohara > Run with Proton-GE /
Run with Wine / Run on the dedicated graphics chip / Add as a game for Windows files in Dolphin. They call
`zohara-settings --run-windows FILE [--layer proton|wine] [--dedicated]` and `--add-game FILE` (game name from the file
name, shared Windows-program settings; Wine when Proton-GE is not installed). The command lines were run by hand; the menu
itself has not been seen in Dolphin yet.

## Safety mode (2026-10-14)
`src/backend/windows_sandbox.rs`. Double-clicked Windows programs that are not one of My games start inside bubblewrap
(the same technology Flatpak uses): read-only system, **empty home folder** (`--tmpfs $HOME`; `/run`, `/tmp`, `/mnt`, `/root`
too), writable only the program's own Windows folder (shared Proton/Wine one, or the game's), the program's own folder
(read-only when it is a broad one like Downloads), Proton-GE read-only, and the display/sound sockets. The session bus,
password agents and the real Steam folder are not visible. Switch: Gaming > Windows programs > "Safety mode for unknown
programs" (on by default, key `sandbox=` in `windows-apps.conf`); per game: switch in the gear window (`X-Zohara-Sandbox`,
off by default); per start: right-click > Zohara > Run in / without Safety mode (`--safe`, `--unsafe`). If bubblewrap is not
usable the person is asked before it runs unprotected.
Proved on the laptop with a real Proton-GE run of a `.bat`: it could not read a canary file in the real home, saw only its
own folder in `Z:\home\<user>`, and a file it "wrote" to the home never reached the real disk, while writing in its own
folder worked. Not covered: it can use the network; no network switch yet; Game Mode does not reach in (no session bus);
anti-cheat/DRM will not run; not tested with a heavy game, with AMD, or on a fresh install. Do **not** add
`--die-with-parent`: the starter stops watching after 10 s and would kill the program.

## Tools for one game (2026-10-14)
Gear window > "Tools for this game" (Windows programs only; `src/backend/game_tools.rs`): pick a Proton-GE version for this
game (`X-Zohara-ProtonTag`, newest when unset); add a Windows component (vcrun2022/2019, d3dx9, corefonts, dotnet48, xact)
with winetricks, using the **same Wine as the game** (Proton-GE's `files/bin/wine` for Proton, the system's for Wine);
open the game's own C: drive; reset the game's own Windows folder (moved to the trash; refused for the shared one).
Checked by hand: the exact winetricks command with GE-Proton11-9's Wine installed `corefonts` into a throwaway folder
(exit 0). The window itself was seen in the test container; Install/Reset were not pressed there.

## Lutris and Heroic games (2026-10-14)
Gaming > Your games: a gear for Lutris and Heroic games too (`src/backend/launcher_config.rs`, `src/pages/launcher_settings.rs`).
* **Lutris**: edits `~/.config/lutris/games/<slug>-<id>.yml` (or the Flatpak path): `system: gamemode, mangohud, env`,
  `game: args`, `wine: dxvk`. Only those keys are touched, the rest of the file stays byte for byte; a file with tabs, a
  one-line `env: {..}` or other odd shapes is refused. The edited output was parsed with PyYAML to check it is valid YAML.
* **Heroic**: edits `GamesConfig/<app>.json` (`autoInstallDxvk, useGameMode, showMangohud, launcherArgs,
  enviromentOptions`; key names checked against Heroic's `types.ts` / `game_config.ts`); a game without a file gets
  `{"<app>": {...}, "version": "v0.1", "explicit": false}`.
* The graphics chip is written as environment variables (from `switcherooctl`); "Dedicated" is recognised when they are all there.
* The launcher must be closed (it rewrites its files); a `.zohara-backup` copy is kept the first time.
* In a Flatpak launcher the Game Mode / MangoHud switches are shown but cannot be changed (not inside the sandbox).
* **Not verified against a real Lutris or Heroic game**: none was installed on the laptop. Tested with files in the
  documented shape, in the test container (opened the gear, pressed Save, read the files back).

## Java for Minecraft (2026-10-14)
Gaming > Java for Minecraft (`src/backend/java_games.rs`): lists the system Javas (`archlinux-java status`), the Javas that
launchers brought (Modrinth, Prism, the official launcher; found by their `release` files), installs Java 8/17/21/25
(`pacman -S jre*-openjdk`, one password prompt) and switches the default (`archlinux-java set`). The version table is
Mojang's published one for 8/16/17/21; "26.x needs Java 25" is inferred from Modrinth bringing Java 25 for Minecraft 26.3
(not from a Mojang statement). A Flatpak launcher cannot see the system Java, which the page says. On the owner's laptop it
finds Modrinth's Zulu 25. Install/Make default were not pressed (they need a password).
The owner's actual Modrinth problem is still unknown: its log from 2026-10-12 shows Minecraft 26.3 running fine with its own Java 25.

## Safety mode and Game Mode (2026-10-18)
Safety mode hides the session's message bus (`/run/user/<uid>/bus`) on purpose. `gamemoderun` needs that bus, so inside Safety mode it printed "Could not connect to bus"
dozens of times and did nothing. The launcher now leaves `gamemoderun` out of the command when Safety mode is on (`windows_apps::without_gamemode`, tested); Game Mode
works for programs started with Safety mode off.

## Failures explained in plain words (2026-10-18)
When a program closes right away, the notice reads the end of its log (`explain_failure`, `log_tail`) and says why when it is a known cause:
* `UniversalApiContract` / `Windows.UI.Xaml` / `Windows.ApplicationModel`: a Microsoft Store (WinRT) program. **Microsoft's `MinecraftInstaller.exe` is one**; Wine and Proton
  have no WinRT, so it cannot run (owner's log, 2026-10-18; not caused by Safety mode). Use Java Edition (Modrinth / Prism Launcher, Gaming > Java) or Bedrock (mcpelauncher).
* "Could not load file or assembly": a .NET program missing a Windows part (Tools can add .NET). `vcruntime` missing: Tools can add the Visual C++ runtimes.

## Folders per game, Windows parts, and the log doctor (2026-10-18)
**Where the Windows folders ("C: drives") are.** Proton-GE and plain Wine cannot share one, so each has its own:
* shared, for double-clicked programs that are not registered games: Proton `~/.local/share/zohara/windows` (C: is `pfx/drive_c`), Wine `~/.local/share/zohara/windows-wine`;
* one folder **per game** (a game in My games, or a file inside a registered game's folder): Proton `~/.local/share/zohara/prefixes/<game>`, Wine
  `~/.local/share/zohara/prefixes-wine/<game>`. The Play button and a double-click of the game's file use the same folder. A program an installer put into a shared
  folder keeps using that shared folder.
* Before 2026-10-18 plain Wine had no per-game folder: it fell back to `~/.wine` for everything (and inside Safety mode that folder was lost on close). Games already
  saved keep the old line until their gear window is saved again.

**Windows parts** (`game_tools::COMPONENTS`, Gaming > Windows programs > "Windows parts", and the game's Tools): WebView2, Visual C++ runtimes, DirectX files, fonts,
.NET. Most are winetricks verbs run with the game's own Wine. **WebView2 has no winetricks verb**: Microsoft's Evergreen installer
(`https://go.microsoft.com/fwlink/p/?LinkId=2124703`, 1.8 MB, downloads the runtime itself) is run silently with `proton run … /silent /install` (or `wine`).
Verified 2026-10-18 with the real GE-Proton11-9 in a throwaway folder: exit 0, `Program Files (x86)/Microsoft/EdgeWebView/Application/155.0.4283.62/msedgewebview2.exe`
and the registry key `…\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}` with `pv` (`game_tools::component_installed` reads it; tested on the real folder).
Every Windows launch also carries `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--no-sandbox` (the person's own value wins): known to be needed under Wine, **not yet verified with a
real WebView2 program**.

**The log doctor** (`backend/win_diag.rs`, `windows_apps::run_file_once`): when a program closes right away, the end of its log is matched against rules: WinRT / Microsoft Store programs
(no fix: the Minecraft Launcher installer), WebView2 missing, Visual C++ DLLs, d3dcompiler, .NET assemblies, Game Mode noise. The notice says why in plain words and, when a part fixes it,
offers a button ("Install WebView2 and try again"): it installs the part into the folder the program used and starts the program again **once** (`Once::fixed`, no loop).
Unit-tested on the real Minecraft log; the notice buttons and a real failing program were not exercised end to end.

**Testing Windows programs without touching the owner's screen:** a nested hidden desktop works: `dbus-run-session -- kwin_wayland --virtual --xwayland --no-lockscreen --exit-with-session ./script.sh`
(sets `DISPLAY=:2`, `WAYLAND_DISPLAY=wayland-1`; the real session is untouched; wrap in `nice -n 19`). Do not `pkill -f` a pattern that appears in your own command line.

**Hermes Desktop** (Nous Research) is an Electron app with a **native Linux build**: install the CLI and run `hermes desktop`; the Windows installer is not needed (the owner's WebView error came from running `Hermes-Setup.exe`). After WebView2 was installed (2026-10-18) the same installer got one step further and failed on `install.ps1 -Manifest produced no parseable JSON payload`: it downloads and runs a PowerShell script, which Wine does not have. The log doctor now says so (rule before the web view rule, no fix button).

## Windows programs out of the box (2026-10-18)
Goal (owner): on a fresh Zohara, download any `.exe` and double-click it; nothing to set up first.
* **Double-click is the default.** The package ships `zohara-windows.desktop` and a system `mimeapps.list` (`/usr/share/applications/`) that make Zohara open Windows file types;
  a person's own `~/.config/mimeapps.list` still wins, and the Settings switch turns it off. (Nothing to add: this existed since 2026-10-14.)
* **The Windows layer is installed with the system.** `zohara-settings` now depends on `wine winetricks wine-mono wine-gecko cabextract` (all Arch `extra`), so a machine gets them
  with the next update; the ISO list (`zohara-profile/packages.x86_64`, commit `5ca1ba4`, `[skip ci]`, no ISO was built) names them too. wine-mono and wine-gecko matter: without
  them Wine stops to download them whenever it makes a Windows folder.
* **Proton-GE (about 560 MB) is downloaded on first use**, not shipped: when a double-click falls back to Wine because it is missing, the notice has **Install Proton-GE now**
  (`zohara-settings --install-proton`, own process, a notice at the start and at the end).
* **Not verified:** the notice buttons on a real laptop, and a fresh-install run from the ISO (no ISO was built).
* **Other places that use Wine:** the Store's own Windows-installer flow makes `~/.wine_zohara_<app_id>` per app (see `zohara/docs/zohara_store_packages_guide.md`); it is separate from these folders.
* Trade-off: Wine (hundreds of MB) is now a hard dependency of Settings. If someone wants it removable, split a `zohara-windows-support` package later (it would need to exist in every channel first).

## PowerShell under Wine (tested 2026-10-19, nothing built yet)
Installers that run a PowerShell script (Hermes' installer does) fail under Wine: Wine's own `powershell.exe` is an empty stub. With GE-Proton11-9 in a throwaway folder:
`winetricks -q powershell_core` installs PowerShell 7.4.11; `winetricks -q powershell` installs a real `powershell.exe` wrapper; `pwsh` needs
`DOTNET_SYSTEM_GLOBALIZATION_INVARIANT=1` (else an ICU error) and **a terminal**: with no terminal it crashes in `ConsoleHost.Start` or silently does nothing, run under a
pseudo-terminal (`script -qec "wine pwsh.exe …"`) it worked. Not tested: the real Hermes installer with these, double-click launches. Full notes and the idea for a "PowerShell" Windows part:
`zohara/docs/RUN-ANYTHING-MACOS-IOS-2026-10-19.md` section 6. Never run the system `wineboot` before Proton's `wine` (wrong Wine server, `version mismatch`).
