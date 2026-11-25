#!/bin/bash
# Tests for data/zohara-android-launch and data/zohara-android-menu with a fake `waydroid` (no Android needed).
set -u
HERE=$(cd "$(dirname "$0")/.." && pwd)
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
L="$HERE/data/zohara-android-launch"; M="$HERE/data/zohara-android-menu"
cat > "$T/fakewd" <<'EOS'
#!/bin/bash
# state: $T/up (exists = Android runs). Records every call in $T/calls.
echo "$*" >> "$FAKE_DIR/calls"
case "$1 $2" in
  "status "*|"status ") if [ -e "$FAKE_DIR/up" ]; then echo "Session:	RUNNING"; else echo "Session:	STOPPED"; fi ;;
  "session start") [ -e "$FAKE_DIR/never" ] || { sleep 1; : > "$FAKE_DIR/up"; } ;;
  "app list") [ -e "$FAKE_DIR/up" ] || exit 1 ;;
  *) : ;;
esac
EOS
chmod +x "$T/fakewd"
export FAKE_DIR="$T" ZOHARA_WAYDROID="$T/fakewd" ZOHARA_ANDROID_CONF="$T/android.conf" ZOHARA_ANDROID_WAIT=20 PATH="$T/nonotify:$PATH"
fails=0
ok() { if [ "$1" = 0 ]; then echo "ok   $2"; else echo "FAIL $2"; fails=$((fails+1)); fi; }
reset() { rm -f "$T/calls" "$T/up" "$T/never"; }

reset; : > "$T/up"; bash "$L" com.android.calculator2; sleep 3
grep -q '^show-full-ui' "$T/calls" && grep -q '^app launch com.android.calculator2' "$T/calls"; ok $? "running, default mode: the full window, then the app"
[ "$(grep -n '^show-full-ui' "$T/calls" | head -1 | cut -d: -f1)" -lt "$(grep -n '^app launch' "$T/calls" | head -1 | cut -d: -f1)" ]; ok $? "the window is asked for before the app"

reset; bash "$L" cc.honista.app; sleep 3
grep -q '^session start' "$T/calls" && grep -q '^app launch cc.honista.app' "$T/calls"; ok $? "stopped: Android is started, waited for, then the app is launched"

reset; : > "$T/up"; printf 'mode=windows\n' > "$T/android.conf"; bash "$L" com.android.chrome
! grep -q '^show-full-ui' "$T/calls" && grep -q '^app launch com.android.chrome' "$T/calls"; ok $? "mode windows: no full window, the app on its own"
rm -f "$T/android.conf"

reset; : > "$T/up"; bash "$L" 'x; rm -rf /' 2>/dev/null; [ "$?" = 2 ]; ok $? "a package name with shell characters is refused"
bash "$L" 2>/dev/null; [ "$?" = 2 ]; ok $? "no package name is refused"
bash "$L" 'noDots' 2>/dev/null; [ "$?" = 2 ]; ok $? "a name without dots is refused"
[ ! -s "$T/calls" ] || ! grep -q 'app launch' "$T/calls"; ok $? "nothing was launched for the refused names"

reset; : > "$T/never"; ZOHARA_ANDROID_WAIT=3 bash "$L" com.android.calculator2 2>/dev/null; [ "$?" = 1 ]; ok $? "Android that never starts: gives up with an error"
! grep -q 'app launch' "$T/calls"; ok $? "and launches nothing"

reset; : > "$T/up"; bash "$L" --status | grep -q 'mode=window' && bash "$L" --status | grep -q 'android=running'; ok $? "--status says the mode and whether Android runs"

# the menu entries
mkdir -p "$T/apps"
printf '[Desktop Entry]\nType=Application\nName=Honista\nExec=waydroid app launch cc.honista.app\nIcon=/x.png\nActions=app-settings\n\n[Desktop Action app-settings]\nName=App Settings\nExec=waydroid app intent android.settings.APPLICATION_DETAILS_SETTINGS package:cc.honista.app\n' > "$T/apps/waydroid.cc.honista.app.desktop"
printf '[Desktop Entry]\nName=Other\nExec=waydroid app launch com.x.y; evil\n' > "$T/apps/waydroid.odd.desktop"
printf '[Desktop Entry]\nName=Mine\nExec=mine\n' > "$T/apps/mine.desktop"
export ZOHARA_APPS_DIR="$T/apps" ZOHARA_ANDROID_LAUNCH=/usr/lib/zohara/zohara-android-launch
bash "$M"
grep -q '^Exec=/usr/lib/zohara/zohara-android-launch cc.honista.app$' "$T/apps/waydroid.cc.honista.app.desktop"; ok $? "the app entry points at the launcher"
grep -q '^Exec=waydroid app intent' "$T/apps/waydroid.cc.honista.app.desktop" && grep -q '^Icon=/x.png' "$T/apps/waydroid.cc.honista.app.desktop"; ok $? "other lines and actions are untouched"
grep -q 'evil' "$T/apps/waydroid.odd.desktop" && grep -q '^Exec=waydroid app launch com.x.y; evil' "$T/apps/waydroid.odd.desktop"; ok $? "an odd Exec line is left alone"
grep -q '^Exec=mine$' "$T/apps/mine.desktop"; ok $? "other programs' entries are left alone"
b=$(md5sum "$T/apps/"*.desktop | md5sum); bash "$M"; [ "$b" = "$(md5sum "$T/apps/"*.desktop | md5sum)" ]; ok $? "running it twice changes nothing"
[ -z "$(ls "$T/apps" | grep -v '\.desktop$')" ]; ok $? "no temporary files are left behind"
[ "$fails" = 0 ] && echo "all passed" || { echo "$fails failed"; exit 1; }
