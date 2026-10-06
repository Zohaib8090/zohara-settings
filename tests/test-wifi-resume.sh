#!/bin/sh
# Tests data/zohara-wifi-resume against a fake /sys/class/net, with the tools replaced by recorders.
set -eu
HOOK="$(cd "$(dirname "$0")/.." && pwd)/data/zohara-wifi-resume"
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
export ZOHARA_NET="$T/net" ZOHARA_STATE="$T/state" ZOHARA_WAIT=0 ZOHARA_MODPROBE="$T/modprobe" ZOHARA_SYSTEMCTL="$T/systemctl"
printf '#!/bin/sh\necho "modprobe $*" >> "%s/calls"\n' "$T" > "$T/modprobe"
printf '#!/bin/sh\necho "systemctl $*" >> "%s/calls"\n' "$T" > "$T/systemctl"
chmod +x "$T/modprobe" "$T/systemctl"
fail() { echo "FAIL: $*"; exit 1; }

# A Wi-Fi interface wlan0 whose driver module is rtw89_pci.
mkdir -p "$T/net/wlan0/wireless" "$T/mods/rtw89_pci" "$T/net/wlan0/device/driver"
ln -s "$T/mods/rtw89_pci" "$T/net/wlan0/device/driver/module"

sh "$HOOK" pre suspend
[ "$(cat "$T/state")" = "rtw89_pci" ] || fail "pre did not remember the driver (got: $(cat "$T/state"))"

# Wi-Fi came back by itself: the hook must do nothing.
sh "$HOOK" post suspend
[ ! -e "$T/calls" ] || fail "post touched things although Wi-Fi was fine: $(cat "$T/calls")"

# Wi-Fi did not come back: reload the driver, then restart NetworkManager.
rm -rf "$T/net/wlan0/wireless"
sh "$HOOK" post suspend
grep -qx "modprobe -r rtw89_pci" "$T/calls" || fail "driver was not removed"
grep -qx "modprobe rtw89_pci" "$T/calls" || fail "driver was not loaded again"
grep -qx "systemctl restart NetworkManager" "$T/calls" || fail "NetworkManager was not restarted"

# Nothing was recorded (no Wi-Fi before sleep): post does nothing.
rm -f "$T/calls"; : > "$T/state"
sh "$HOOK" post suspend
[ ! -e "$T/calls" ] || fail "post acted without a remembered driver"

# A hostile module name is never remembered.
mkdir -p "$T/net/wlan1/wireless" "$T/net/wlan1/device/driver" "$T/mods/ev;il"
ln -s "$T/mods/ev;il" "$T/net/wlan1/device/driver/module"
sh "$HOOK" pre suspend
grep -q ';' "$T/state" && fail "an unsafe module name was remembered"
echo "wifi-resume hook: all checks passed"
