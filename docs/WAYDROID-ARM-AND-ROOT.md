# Waydroid: ARM apps and "this phone looks rooted" (2026-11-19)

Status (2026-11-19): BUILT and shipped as part of the zohara-settings package. Worked by hand on the owner's laptop first (Hoinsta, which said "App Can NOT Run on Rooted Devices", opens after the build markers were changed; abilist shows arm64-v8a/armeabi-v7a/armeabi after libhoudini).

## What the update does
- `/usr/lib/zohara/zohara-waydroid-fix` (data/zohara-waydroid-fix) with two root services:
  - `zohara-waydroid-fix.service` runs before every `waydroid-container.service` start, offline: it builds `/var/lib/waydroid/overlay/system/build.prop` from the image's own build.prop (read-only loop mount) with `test-keys`->`release-keys` and `userdebug`->`user`. After a Waydroid image update it starts again from the new image (the old overlay file belongs to the old image). Settings: `disguise=1` by default; `0` puts the markers back.
  - `zohara-waydroid-arm.service` runs at boot and when Settings turns ARM on: only if `arm=1` in `/etc/zohara/waydroid.conf` (OFF by default: it downloads Intel's libhoudini / the libndk translation onto the person's computer, which Zohara does not redistribute). It fetches the PINNED waydroid_script commit (sha256 checked, pip packages pinned), runs `main.py install libhoudini|libndk` (by CPU vendor), then re-applies the build markers on top of the file the translator edited. Without internet it tries again next boot.
- Settings > Apps > Android apps: two switches, a status line, a "Stop Android" button when a restart is needed. Status is in `/var/lib/zohara/waydroid/status`.
- Never blocks Waydroid: every step is guarded and the exit code is always 0.
- Tests: `tests/waydroid-fix-test.sh` (19 checks against a throwaway folder: markers, idempotence, ARM lines kept, switch off restores, image update, CPU choice, no-Waydroid) and the unit tests in `src/pages/android_apps.rs`.

## Not verified
- The two services and the package hooks on a clean machine, the ARM download from inside the service, and the Settings page on screen. (Done by hand on the owner's laptop: the same steps.)
- Bumping the pinned waydroid_script commit needs a test run; Intel's houdini build inside it expires on 2028-07-01.
- The disguise does not hide `su`/Magisk files, a writable /system, or Google's Play Integrity.

## Original findings and plan (kept for reference)


## What the owner's laptop shows (read over adb from the running container, nothing changed)
- ABI list `x86_64,x86`, `ro.dalvik.vm.native.bridge=0`: no ARM translator, so ARM-only apps do not install or crash.
- CPU is Intel (i5-13420H): the matching translator is libhoudini; on AMD it is libndk.
- Build `userdebug`, tags `test-keys`, fingerprint `waydroid/lineage_waydroid_x86_64/...:userdebug/test-keys`, model "WayDroid x86_64 Device". No `su` file, `ro.secure=1`, `ro.debuggable=0`. Apps flag the developer-build markers; Play Integrity (hardware/device attestation) can never pass in Waydroid.
- Waydroid builds `/var/lib/waydroid/waydroid.prop` from `/var/lib/waydroid/waydroid_base.prop` at container start (tools/helpers/images.py make_prop); `[properties]` in waydroid.cfg is for runtime `persist.*` props.

## Plan: do it as part of the update
1. **ARM translator**: a root one-shot `zohara-waydroid-extras` (systemd, runs when `waydroid-container.service` is up and the system image changed). It picks libhoudini (Intel) or libndk (AMD), installs it with casualsnek/waydroid_script (`main.py install libhoudini|libndk`, GPL-3.0; its houdini build has an expiry of 2028-07-01, so it must be fetched fresh, never pinned), then restarts the session. Reasons it is NOT a pacman post-install hook: no network in package scripts, and Intel's houdini is a proprietary binary (taken from Microsoft's WSA) that Zohara must not redistribute: it is downloaded on the person's computer, after a one-time consent shown in Settings ("Add support for apps made for ARM phones. Downloads Intel's translation library"). A Waydroid image update replaces system.img and removes the translator, so the service re-checks on every start (compare the abi list) and re-applies.
2. **Looks rooted**: remove the developer-build markers (build tags/type/fingerprint/model) only through props Waydroid loads at start; whether ro.* values from waydroid_base.prop win over the image's own build.prop must be tested first (Android sets ro.* once). If it does not work that way the fallback is what waydroid_script's Magisk Delta does (resetprop), which adds real root-like tooling and is NOT acceptable as a default. Never install Magisk to hide root.
3. A Settings page "Android apps (Waydroid)": ARM support (switch + status), "Look like a normal phone" (switch + undo), what each one changes, a note that banking apps that use Play Integrity will still refuse.
4. Test in the lab VM first (owner's rule), with a real ARM-only APK and two root-checker apps, before this reaches alpha.
