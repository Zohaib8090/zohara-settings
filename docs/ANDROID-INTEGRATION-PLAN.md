# Android in Zohara OS: one unified experience (owner's request, 2026-11-22)

Goal: Android apps (Waydroid) feel like a part of Zohara OS for EVERY user, not a fix for one laptop: keyboard and mouse work
properly, navigation is obvious, microphone, camera, files and permissions are in their place, and one Settings page controls it
all. Ships to everyone as ordinary updates (alpha first, then beta/stable only when the owner says).

## What exists today
- Waydroid 1.6.3 from pacman, no Zohara layer except `zohara-waydroid-fix` (build-marker fix, ARM translation; docs/WAYDROID-ARM-AND-ROOT.md) and Settings > Apps > Android apps (two switches).
- Verified on the owner's laptop (2026-11-22): the microphone (PipeWire source) and the webcam (/dev/video0) are visible to the container, Android reports `android.hardware.camera` and `microphone`, its audio policy defines a Built-In Mic, but apps hold CAMERA and RECORD_AUDIO as "not granted" and the owner reports neither works. Cause not found yet: diagnostic `waydroid-mic-camera-check.sh` waits for the owner's output.

## Facts the design relies on
- Waydroid CLI: `status, session start|stop, container start|stop|freeze|unfreeze, app install|remove|launch|list|intent, prop get|set, show-full-ui, first-launch, shell, logcat, adb, upgrade, init, bugreport`. `shell` needs root; `adb` needs `android-tools`.
- Documented props (docs.waydro.id/usage/waydroid-prop-options; most need a session restart): `persist.waydroid.multi_windows` (apps as normal desktop windows), `width`/`height` (resolution override), `width_padding`/`height_padding`, `suspend` (sleep when idle), `uevent` (hotplugged devices), `reverse_scrolling`, `fake_touch` (comma list of packages whose mouse input is sent as touch, `*` wildcards, 91 characters), `fake_wifi`, and debug ones (`no_presentation`, `invert_colors`, `cursor_on_subsurface`, `cursor_force_shm`, `no_background_subsurface`, `use_subsurface`). Display density is `lcd_density` in `waydroid.cfg`.
- Root-only work goes through one root helper with polkit (the `zohara-waydroid-fix` pattern); per-app permissions use `pm grant/revoke` (adb or `waydroid shell`).

## The unified page (Settings > Apps > Android apps), in order of value
1. **Control and status**: first-run setup in one click (init, kernel module check), Start / Stop / Freeze, is it running, image version, "Update Android" (`waydroid upgrade`), "Reset" and "Repair" with a confirm, "Save a problem report" (`waydroid bugreport` + our logs).
2. **Windows and display**: apps as normal windows (multi_windows, default ON), default window size, density (small/normal/large), paddings, "sleep when nothing is open" (suspend), apps remember their window size.
3. **Keyboard, mouse and navigation**: mouse as touch for chosen apps (fake_touch editor with a list of installed apps, default for known games), reverse scrolling, **Back / Home / Recents / Menu** reachable by keyboard and mouse: global shortcuts (defaults Meta+Backspace, Meta+Home-style, shown in the page, re-assignable) plus a small "Android keys" window/panel button that sends `input keyevent`; Esc stays Back inside Android windows where the app supports it; text input and the on-screen keyboard choice (a hardware keyboard layout follows the desktop layout); copy and paste both ways; drag and drop of files into an Android app.
4. **Devices and permissions**: microphone and camera: status (found / default input / which webcam), a Test button for each (records 3 seconds from the host source / shows a still), "Use this microphone / this camera" pickers, and a per-app permission list (camera, microphone, location, storage, notifications) with switches that run `pm grant/revoke`. Hotplug (USB webcam/headset) via `uevent`. Speaker and headphone choice follow Settings > Sound.
5. **Apps**: list of installed Android apps with launch, uninstall, "Open in Android" for `.apk` files (Zohara Store / file manager integration), Start-menu entries kept in step, app shortcuts on the taskbar.
6. **Files and network**: "Android files" folder in the file manager (the container's `media/0`), downloads land in a visible place; network follows the host (proxy/VPN notice).
7. **Compatibility**: ARM apps and the build-marker fix (done), optional Google services (microG) and Widevine, clear text about what cannot work (Play Integrity, apps that need real root).

## Delivery
- Every step: pure logic in Rust unit tests; root helper steps with throwaway-folder shell tests; anything that touches a real container is tested in the lab VM with a real Waydroid and real apps (owner's rule), then alpha, then beta/stable only on the owner's word.
- Order of work: (a) find the mic/camera cause (needs the diagnostic output) and fix it for everyone; (b) page skeleton with control/status and windows; (c) keyboard/mouse/navigation; (d) devices and permissions; (e) apps and files; (f) polish and docs for people.
- Not built yet: everything above except the ARM/build-marker switches.

## 2026-11-25: opening apps from the menu (built)
Finding: on the owner's Intel + NVIDIA laptop Waydroid's per-app window mode never delivers windows to the desktop (Android reports the app active, no Wayland window arrives); `waydroid app launch` does nothing when Android is not running yet and prints `Sending reply failed` after a failed window. Upstream issue waydroid/waydroid#2436 describes the same for the 2026-09-27 vendor image (2026-04-28 works there; here it did not help). Waydroid's FULL Android window works everywhere, so:
- `/usr/lib/zohara/zohara-android-launch <package>`: starts Android if needed, waits until it answers (`waydroid app list`), shows the full Android window and starts the app in it. `mode=windows` in `~/.config/zohara/android.conf` uses per-app windows instead (for computers where they work).
- `/usr/lib/zohara/zohara-android-menu` + the user path unit `zohara-android-menu.path`: rewrites Waydroid's menu entries (`~/.local/share/applications/waydroid.*.desktop`, Exec exactly `waydroid app launch <package>`) to call the launcher; runs again whenever the folder changes (Waydroid rewrites those files).
- Settings > Apps > Android apps: "Each Android app in its own window" (writes the mode and `persist.waydroid.multi_windows`) and "Android screen shape" (Automatic / Phone 450x960 / Tablet 1100x700 / Large 1600x960 through `persist.waydroid.width/height`).
- Tests: `tests/android-launch-test.sh` (17 checks with a fake waydroid) and unit tests in `src/pages/android_apps.rs`. Not tested: a real menu click, the units on a clean machine, the page on screen, how the app looks in each shape. To be done in the lab VM and on the owner's laptop.
- Open: whether "Phone" shape in the single window gives phone layouts in every app; per-app shapes need per-app windows (not available on this laptop), so they stay a later step.
