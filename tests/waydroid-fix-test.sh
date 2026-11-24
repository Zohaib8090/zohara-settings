#!/bin/bash
# Tests for data/zohara-waydroid-fix against a throwaway folder (no root, no Waydroid needed). Folders stand in for the mounted images.
set -u
HERE=$(cd "$(dirname "$0")/.." && pwd)
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
export ZOHARA_WAYDROID_DIR="$T/w" ZOHARA_WAYDROID_CONF="$T/waydroid.conf" ZOHARA_WAYDROID_STATE="$T/state"
mkdir -p "$T/w/images"; : > "$T/w/images/system.img"; : > "$T/w/images/vendor.img"
S="$T/sysroot"; V="$T/vendroot"
mkdir -p "$S/system/system_ext/etc" "$S/system/product/etc" "$V/odm/etc" "$V/vendor_dlkm/etc"
fp() { echo "waydroid/lineage_waydroid_x86_64/waydroid_x86_64:13/TQ3A.230901.001/$1:userdebug/test-keys"; }
printf 'ro.build.type=userdebug\nro.build.tags=test-keys\nro.build.fingerprint=%s\nro.system.build.fingerprint=%s\nro.build.display.id=lineage_waydroid_x86_64-userdebug 13 TQ3A.230901.001 eng.x test-keys\nro.product.model=WayDroid x86_64 Device\nro.other=1\n' "$(fp eng.x)" "$(fp a09271437)" > "$S/system/build.prop"
printf 'ro.system_ext.build.fingerprint=%s\nro.system_ext.build.tags=test-keys\nro.system_ext.build.type=userdebug\n' "$(fp a09271437)" > "$S/system/system_ext/etc/build.prop"
printf 'ro.product.build.fingerprint=%s\nro.product.fine=1\n' "waydroid/x/y:13/T/e:userdebug/release-keys" > "$S/system/product/etc/build.prop"
printf 'ro.vendor.build.type=userdebug\nro.vendor.build.tags=test-keys\nro.vendor.build.fingerprint=%s\nro.vendor.other=1\n' "$(fp a10221306)" > "$V/build.prop"
printf 'ro.odm.build.fingerprint=%s\nro.odm.build.tags=test-keys\nro.odm.build.type=userdebug\n' "$(fp a10221306)" > "$V/odm/etc/build.prop"
printf 'ro.vendor_dlkm.build.fingerprint=%s\nro.vendor_dlkm.build.tags=test-keys\nro.vendor_dlkm.build.type=userdebug\n' "$(fp a10221306)" > "$V/vendor_dlkm/etc/build.prop"
export ZOHARA_WAYDROID_SYSTEM_ROOT="$S" ZOHARA_WAYDROID_VENDOR_ROOT="$V"
F="$HERE/data/zohara-waydroid-fix"; O="$T/w/overlay"
fails=0
ok() { if [ "$1" = 0 ]; then echo "ok   $2"; else echo "FAIL $2"; fails=$((fails+1)); fi; }
clean() { ! grep -q 'test-keys' "$1"; }

bash "$F" disguise
grep -q '^ro.build.tags=release-keys' "$O/system/build.prop" && grep -q '^ro.build.type=userdebug$' "$O/system/build.prop"; ok $? "system: the tag is release, the type stays userdebug (type user makes Android warn)"
clean "$O/system/build.prop" && grep -q ':userdebug/release-keys' "$O/system/build.prop"; ok $? "system: no test-keys left, fingerprint ends userdebug/release-keys"
grep -q '^ro.other=1' "$O/system/build.prop" && grep -q '^ro.product.model=WayDroid' "$O/system/build.prop"; ok $? "system: other lines are kept"
clean "$O/system/system_ext/etc/build.prop"; ok $? "system_ext has no developer marker (it was the missing one)"
clean "$O/vendor/build.prop"; ok $? "vendor has no developer marker"
clean "$O/vendor/odm/etc/build.prop"; ok $? "odm has no developer marker (it was the missing one)"
clean "$O/vendor/vendor_dlkm/etc/build.prop"; ok $? "vendor_dlkm has no developer marker (it was the missing one)"
[ ! -e "$O/system/product/etc/build.prop" ]; ok $? "a file that never had a marker is not copied"
n=$(grep -rl 'test-keys' "$O" | wc -l); [ "$n" = 0 ]; ok $? "nothing in the overlay still says test-keys"; ! grep -rqE "^ro\.[a-z_.]*build\.type=user$|:user/release" "$O"; ok $? "and no partition says build type user"
before=$(find "$O" -type f | sort | xargs cat | md5sum); bash "$F" disguise; [ "$before" = "$(find "$O" -type f | sort | xargs cat | md5sum)" ]; ok $? "running it twice changes nothing"

echo 'ro.dalvik.vm.native.bridge=libhoudini.so' >> "$O/system/build.prop"; mkdir -p "$O/system/lib"; : > "$O/system/lib/libhoudini.so"
bash "$F" disguise; grep -q '^ro.dalvik.vm.native.bridge=libhoudini.so' "$O/system/build.prop"; ok $? "lines added by the ARM translator stay"
bash "$F" status | grep -q 'arm=present'; ok $? "an installed translator (libraries and the bridge line) is recognised"

printf 'disguise=0\n' > "$T/waydroid.conf"; bash "$F" disguise
grep -q '^ro.build.tags=test-keys' "$O/system/build.prop" && grep -q ':userdebug/test-keys' "$O/system/build.prop" && grep -q -- '-userdebug 13' "$O/system/build.prop"; ok $? "switching it off: system markers come back"
grep -q 'test-keys' "$O/vendor/odm/etc/build.prop" && grep -q 'test-keys' "$O/system/system_ext/etc/build.prop"; ok $? "switching it off: odm and system_ext come back too"
grep -q '^ro.dalvik.vm.native.bridge=libhoudini.so' "$O/system/build.prop"; ok $? "switching it off keeps the ARM lines"
rm -f "$T/waydroid.conf"; bash "$F" disguise; clean "$O/vendor/vendor_dlkm/etc/build.prop"; ok $? "on again"

# an earlier version's overlay files (no record) are taken over, not left stale
rm -rf "$T/state" "$O"; mkdir -p "$O/system" "$O/vendor" "$T/state"
sed -e 's/test-keys/release-keys/g' -e 's/userdebug/user/g' "$S/system/build.prop" > "$O/system/build.prop"; echo 'ro.dalvik.vm.native.bridge=libhoudini.so' >> "$O/system/build.prop"
sed -e 's/test-keys/release-keys/g' -e 's/userdebug/user/g' "$V/build.prop" > "$O/vendor/build.prop"
bash "$F" disguise
clean "$O/vendor/odm/etc/build.prop" && clean "$O/system/system_ext/etc/build.prop"; ok $? "an older install gets the missing partitions fixed"
grep -q '^ro.build.type=userdebug$' "$O/system/build.prop" && ! grep -q ':user/release' "$O/vendor/build.prop"; ok $? "and the type the first version changed to user goes back to userdebug"
grep -q 'libhoudini' "$O/system/build.prop"; ok $? "and keeps the ARM lines it already had"
bash "$F" disguise 2>&1 >/dev/null; printf 'disguise=0\n' > "$T/waydroid.conf"; bash "$F" disguise; grep -q '^ro.build.tags=test-keys' "$O/system/build.prop"; ok $? "an older file can be switched off too"; rm -f "$T/waydroid.conf"; bash "$F" disguise

# a managed file that is gone (the overlay was cleared) is written again
rm -rf "$O"; mkdir -p "$O"; bash "$F" disguise; clean "$O/vendor/odm/etc/build.prop" && grep -q '^ro.build.tags=release-keys' "$O/system/build.prop"; ok $? "after the overlay was cleared the files are written again"

# a new image (a Waydroid update): the old overlay files go and everything is rebuilt from the new images
sleep 1; touch "$T/w/images/system.img"
printf 'ro.build.type=userdebug\nro.build.tags=test-keys\nro.new.version=2\n' > "$S/system/build.prop"
bash "$F" disguise
grep -q '^ro.new.version=2' "$O/system/build.prop" && ! grep -q 'libhoudini' "$O/system/build.prop" && clean "$O/system/build.prop"; ok $? "after an image update the new build.prop is used"
bash "$F" status | grep -q 'arm=absent'; ok $? "and the translator is no longer reported as present (the switch brings it back)"

# ARM: off by default, nothing downloaded
bash "$F" arm; [ ! -d "$T/state/waydroid_script" ]; ok $? "ARM does nothing unless it was turned on"
st=$(bash "$F" status)
echo "$st" | grep -q 'arm_wanted=0'; ok $? "status says ARM is not wanted"
echo "$st" | grep -q 'disguise=applied'; ok $? "status says the disguise is applied"
printf 'GenuineIntel\n' > "$T/cpu"; export ZOHARA_WAYDROID_CPUINFO="$T/cpu"; bash "$F" status | grep -q 'translator=libhoudini'; ok $? "Intel gets libhoudini"
printf 'AuthenticAMD\n' > "$T/cpu"; bash "$F" status | grep -q 'translator=libndk'; ok $? "AMD gets libndk"
printf 'ARM\n' > "$T/cpu"; bash "$F" status | grep -q 'translator=none'; ok $? "another processor gets none"
rm -rf "$T/w/images"; bash "$F" disguise; echo "$?" | grep -q '^0$'; ok $? "no Waydroid image: quietly nothing"
[ "$fails" = 0 ] && echo "all passed" || { echo "$fails failed"; exit 1; }
