# Windows games (Settings > Gaming > Windows games), 2026-10-14

Owner's ask: Windows games that work out of the box, with Proton-GE and ProtonUp-Qt built into the OS, inside the
existing Gaming page (not a page of its own).

## What the person sees
Three rows in a "Windows games" group on the Gaming page:
1. **Windows game support** (button "Set up", then "Open Steam"). One administrator prompt. Turns on `[multilib]`, installs
   `steam lutris winetricks lib32-gamemode lib32-mangohud lib32-mesa` plus the 32-bit driver for the graphics chip found
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
