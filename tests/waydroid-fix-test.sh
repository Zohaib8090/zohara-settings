#!/bin/bash
# Tests for data/zohara-waydroid-fix against a throwaway folder (no root, no Waydroid needed).
set -u
HERE=$(cd "$(dirname "$0")/.." && pwd)
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
export ZOHARA_WAYDROID_DIR="$T/w" ZOHARA_WAYDROID_CONF="$T/waydroid.conf" ZOHARA_WAYDROID_STATE="$T/state"
mkdir -p "$T/w/images" "$T/w/overlay/system"; : > "$T/w/images/system.img"
printf 'ro.build.type=userdebug\nro.build.tags=test-keys\nro.build.fingerprint=waydroid/lineage_waydroid_x86_64/waydroid_x86_64:13/TQ3A.230901.001/eng.x:userdebug/test-keys\nro.build.display.id=lineage_waydroid_x86_64-userdebug 13 TQ3A.230901.001 eng.x test-keys\nro.product.model=WayDroid x86_64 Device\nro.other=1\n' > "$T/src.prop"
export ZOHARA_WAYDROID_BUILD_PROP_SRC="$T/src.prop"
F="$HERE/data/zohara-waydroid-fix"; OVER="$T/w/overlay/system/build.prop"
fails=0
ok() { if [ "$1" = 0 ]; then echo "ok   $2"; else echo "FAIL $2"; fails=$((fails+1)); fi; }

bash "$F" disguise
grep -q '^ro.build.tags=release-keys' "$OVER"; ok $? "tags become release-keys"
grep -q '^ro.build.type=user$' "$OVER"; ok $? "type becomes user"
grep -q ':user/release-keys' "$OVER"; ok $? "fingerprint ends in user/release-keys"
! grep -q 'test-keys\|userdebug' "$OVER"; ok $? "no developer marker is left"
grep -q '^ro.other=1' "$OVER" && grep -q '^ro.product.model=WayDroid' "$OVER"; ok $? "other lines are kept"
before=$(cat "$OVER"); bash "$F" disguise; [ "$before" = "$(cat "$OVER")" ]; ok $? "running it twice changes nothing"

echo 'ARM_LINE_FROM_TRANSLATOR=1' >> "$OVER"
bash "$F" disguise; grep -q '^ARM_LINE_FROM_TRANSLATOR=1' "$OVER"; ok $? "lines added by the ARM translator stay (the overlay file is the source once it exists)"

printf 'disguise=0\n' > "$T/waydroid.conf"; bash "$F" disguise
grep -q '^ro.build.tags=test-keys' "$OVER" && grep -q '^ro.build.type=userdebug$' "$OVER" && grep -q ':userdebug/test-keys' "$OVER" && grep -q -- '-userdebug 13' "$OVER"; ok $? "switching it off puts every marker back"
grep -q '^ARM_LINE_FROM_TRANSLATOR=1' "$OVER"; ok $? "switching it off keeps the ARM lines"

rm -f "$T/waydroid.conf"; bash "$F" disguise
grep -q '^ro.build.tags=release-keys' "$OVER"; ok $? "on again"

# a new system image (a Waydroid update): the old overlay file is dropped and rebuilt from the new image
sleep 1; touch "$T/w/images/system.img"; printf 'ro.build.type=userdebug\nro.build.tags=test-keys\nro.new.version=2\n' > "$T/src.prop"
bash "$F" disguise
grep -q '^ro.new.version=2' "$OVER" && ! grep -q 'ARM_LINE' "$OVER" && grep -q release-keys "$OVER"; ok $? "after an image update the new build.prop is used"

# ARM: off by default, nothing downloaded
bash "$F" arm; [ ! -d "$T/state/waydroid_script" ]; ok $? "ARM does nothing unless it was turned on"
st=$(bash "$F" status)
echo "$st" | grep -q 'arm_wanted=0'; ok $? "status says ARM is not wanted"
echo "$st" | grep -q 'disguise=applied'; ok $? "status says the disguise is applied"
printf 'GenuineIntel\n' > "$T/cpu"; export ZOHARA_WAYDROID_CPUINFO="$T/cpu"; bash "$F" status | grep -q 'translator=libhoudini'; ok $? "Intel gets libhoudini"
printf 'AuthenticAMD\n' > "$T/cpu"; bash "$F" status | grep -q 'translator=libndk'; ok $? "AMD gets libndk"
printf 'ARM\n' > "$T/cpu"; bash "$F" status | grep -q 'translator=none'; ok $? "another processor gets none"
mkdir -p "$T/w/overlay/system/lib"; : > "$T/w/overlay/system/lib/libhoudini.so"; bash "$F" status | grep -q 'arm=present'; ok $? "an installed translator is recognised"

rm -rf "$T/w/images"; bash "$F" disguise; echo "$?" | grep -q '^0$'; ok $? "no Waydroid image: quietly nothing"
[ "$fails" = 0 ] && echo "all passed" || { echo "$fails failed"; exit 1; }
