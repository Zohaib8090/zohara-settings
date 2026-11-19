# Waydroid: ARM apps and "this phone looks rooted" (2026-11-19)

Status: findings and plan, nothing built. Owner wants both fixed by an ordinary Zohara update, with no manual steps.

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
