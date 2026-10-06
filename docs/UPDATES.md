# Zohara Update: how system updates work (Settings)

Since 2026-10-06 **Settings updates the operating system and Zohara Store updates apps.** They are two separate programs
that share no code and can be used one without the other. This page describes Settings' half.

## What each program updates

| Program | Updates | Engine |
|---|---|---|
| **Settings > Zohara Update** | everything that comes from pacman: the system, the kernel, Zohara's own programs (Settings, the Store, ...) and apps installed from the Arch repositories (Firefox, VLC ...) | `src/sysupdate/` |
| **Zohara Store > Updates** | apps installed from Flathub (Flatpak), and going back to an earlier version of one | `zohara-store-rs/src/updates.rs` (in the `zohara` repo) |

Why pacman apps go with the system: on Arch, updating one package without the rest can break programs, so every pacman
update is a full, pinned system update.

## What the page does

* **Check** (on open, or the refresh button): reads the signed approval list, then asks a *private copy* of the package
  databases what is newer. Looking never changes the system.
* **Update all / Update selected**: Zohara's own packages can be updated one at a time; system packages go together.
  One administrator prompt. A restore point is saved before and after (snapper hooks).
* **Update channel**: stable, beta or alpha (`zohara-channel`); beta and alpha ask for confirmation.
* **Go back**: *Undo the last update* (from `/var/log/pacman.log` and the package cache), older versions of Zohara
  packages, and **restore points** (whole-system snapshots, Btrfs).
* **Health check** after an update (`zohara-settings --health-json`), with a banner if something looks wrong.
* **Background check**: `zohara-settings --check-updates`, run by the user timer `zohara-settings-update-check.timer`
  (every 6 hours); one desktop notification per new set of updates; skipped while updates are paused.
* Pause updates, update history and advanced options (cache cleanup) are separate rows lower on the page.

## The signed approval list (why updates are "pinned")

Machines never follow live Arch. `Zohaib8090/zohara-pipeline/manifest.json` names an **approved Arch Linux Archive day**
(`approved_date`), signed with minisign (key `E2D2009325647762`). A system update first verifies the signature, then moves
the mirror to that day (`/etc/pacman.d/mirrorlist`), then runs one full upgrade. It refuses to go backwards in time.

* The public key is **built into the program** (`data/manifest.pub`, `include_str!`), not read from a file, so it cannot be
  missing and Settings and the Store never fight over a shared file.
* The root part is `zohara-settings --pin-date MANIFEST SIGNATURE` (run through pkexec): it re-verifies everything itself
  instead of trusting the caller.
* The private check uses `pacman --disable-sandbox`: under `fakeroot` pacman's download sandbox cannot start on a real
  install (found 2026-10-06), and that sync only downloads into a private temp folder.

## Files

`src/sysupdate/{manifest,channel,updates,ui}.rs` and `mod.rs`; the page itself is `src/pages/updates.rs`; units
`data/zohara-settings-update-check.{service,timer}`; `zohara-settings.install` turns the timer on for existing machines
when Settings is upgraded (the ISO enables it in `customize_airootfs.sh`); tests in the modules and `tests/` (a throwaway
key for the signature tests).

## Verified (2026-10-06, VM on alpha)

Found a pending Settings update through the real signed-approval path, installed it from the page with the administrator
prompt (restore points before and after), "Everything is up to date" afterwards; the root helper accepted the real signed
manifest and refused a tampered one. **Not yet verified**: a full system update with many Arch packages through the page.
