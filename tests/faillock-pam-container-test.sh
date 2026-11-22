#!/bin/bash
# Real PAM test (slow, about 5 minutes, needs podman and the internet): the escalating login lock against pam_faillock and
# sudo in an Arch container with small numbers (3 tries, 1 minute). Run from the repository root:
#   bash tests/faillock-pam-container-test.sh
set -eu
HERE=$(cd "$(dirname "$0")/.." && pwd); W=$(mktemp -d); trap 'rm -rf "$W"' EXIT
cp "$HERE/data/zohara-faillock-policy" "$HERE/data/zohara-faillock-install" "$W/"
cat > "$W/run.sh" <<'IN'
set -u
pacman -Syu --noconfirm --needed sudo >/dev/null 2>&1 || { echo "pacman install of sudo failed"; exit 2; }
mkdir -p /usr/lib/zohara /etc/zohara /var/lib/zohara
install -m755 /t/zohara-faillock-policy /t/zohara-faillock-install /usr/lib/zohara/
printf 'tries=3\nminutes=1\n' > /etc/zohara/faillock.conf
useradd -m tester; echo 'tester:goodpass' | chpasswd; usermod -aG wheel tester; echo '%wheel ALL=(ALL:ALL) ALL' > /etc/sudoers.d/w
/usr/lib/zohara/zohara-faillock-install apply
grep -c zohara-faillock-policy /etc/pam.d/system-auth | grep -q '^2$' && echo "PASS two lines placed" || echo "FAIL lines"
try() { su tester -c "echo '$1' | sudo -S -k true" >/dev/null 2>&1 && echo OK || echo REFUSED; }
conf() { grep -E '^(deny|unlock_time)' /etc/security/faillock.conf | tr -d ' ' | tr '\n' ' '; }
for i in 1 2 3; do try wrong >/dev/null; done
[ "$(try goodpass)" = REFUSED ] && echo "PASS locked after 3 wrong tries (even the right password)" || echo "FAIL not locked"
sleep 65
try wrong >/dev/null
[ "$(conf)" = "deny=2 unlock_time=120 " ] && echo "PASS second round is 2 tries / 2 minutes" || echo "FAIL second round: $(conf)"
sleep 125
[ "$(try goodpass)" = OK ] && echo "PASS the right password works once the lock has ended" || echo "FAIL still locked"
/usr/lib/zohara/zohara-faillock-install remove
[ "$(grep -c zohara /etc/pam.d/system-auth)" = 0 ] && echo "PASS removed again" || echo "FAIL remove"
IN
podman run --rm -v "$W":/t:ro mirror.gcr.io/library/archlinux:latest bash /t/run.sh
