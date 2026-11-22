#!/bin/bash
# Logic tests for data/zohara-faillock-policy with a fake `faillock` and a fixed clock (no PAM, no root).
set -u
HERE=$(cd "$(dirname "$0")/.." && pwd); F="$HERE/data/zohara-faillock-policy"
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
mkdir -p "$T/bin" "$T/state"
printf '# stock\n# deny = 3\nsilent\n' > "$T/faillock.conf"
export ZOHARA_FAILLOCK_CONF="$T/faillock.conf" ZOHARA_FAILLOCK_STATE="$T/state" ZOHARA_FAILLOCK_SETTINGS="$T/none" ZOHARA_FAILLOCK_USER=tester PATH="$T/bin:$PATH"
BASE=1800000000
# the fake: prints $FAKE_COUNT valid failures that happened $FAKE_AGE seconds before NOW
cat > "$T/bin/faillock" <<'EOS'
#!/bin/bash
echo "tester:"; echo "When                Type  Source                                           Valid"
for i in $(seq 1 "${FAKE_COUNT:-0}"); do echo "$(date -d @$((ZOHARA_FAILLOCK_NOW - FAKE_AGE)) '+%Y-%m-%d %H:%M:%S') TTY   /dev/pts/0                                           V"; done
EOS
chmod +x "$T/bin/faillock"
fails=0
conf() { grep -E "^(deny|unlock_time)" "$T/faillock.conf" | tr -d ' ' | tr '\n' ' '; }
check() { if [ "$1" = "$2" ]; then echo "ok   $3"; else echo "FAIL $3 (got: $1, wanted: $2)"; fails=$((fails+1)); fi; }
run() { ZOHARA_FAILLOCK_NOW=$NOW bash "$F" "${1:-before}"; }
state_tier() { grep '^tier=' "$T/state/tester" | cut -d= -f2; }

NOW=$BASE; FAKE_COUNT=0 FAKE_AGE=0 run
check "$(conf)" "deny=5 unlock_time=300 " "first round: 5 tries, 5 minutes (the defaults)"
# five wrong tries just now: locked, the numbers stay
NOW=$((BASE+10)); FAKE_COUNT=5 FAKE_AGE=2 run; check "$(conf)" "deny=5 unlock_time=300 " "while locked the numbers do not change"
# the lock ends (400 s later, failures still inside the 15 minute window): the next round is stricter
for expect in "deny=4 unlock_time=600 " "deny=3 unlock_time=1200 " "deny=2 unlock_time=2400 " "deny=2 unlock_time=3600 " "deny=2 unlock_time=3600 "; do
    NOW=$((NOW+4000)); FAKE_COUNT=0 FAKE_AGE=0 run                  # lock ended and the old failures are out of the window: round up
    check "$(conf)" "$expect" "after a lock ends: $expect"
    NOW=$((NOW+5)); FAKE_COUNT=$(echo "$expect" | sed 's/deny=\([0-9]*\).*/\1/') FAKE_AGE=1 run   # locked again in this round
done
check "$(state_tier)" "4" "the rounds stop at the strictest one (never longer than an hour)"
NOW=$((NOW+90000)); FAKE_COUNT=0 FAKE_AGE=0 run; check "$(conf)" "deny=5 unlock_time=300 " "24 hours without a lock start over"
# a success puts it back at once
NOW=$((NOW+10)); FAKE_COUNT=5 FAKE_AGE=1 run; NOW=$((NOW+400)); FAKE_COUNT=5 FAKE_AGE=410 run; check "$(state_tier)" "1" "second round reached"
run success; check "$(state_tier)" "0" "a correct password resets the rounds"; check "$(conf)" "deny=5 unlock_time=300 " "and the numbers"
# settings
printf 'tries=7\nminutes=3\n' > "$T/s"; export ZOHARA_FAILLOCK_SETTINGS="$T/s"; rm -f "$T/state/tester"
NOW=$((NOW+10)); FAKE_COUNT=0 run; check "$(conf)" "deny=7 unlock_time=180 " "the first round follows the settings"
printf 'tries=99\nminutes=0\n' > "$T/s"; run; check "$(conf)" "deny=5 unlock_time=300 " "numbers out of range fall back to the defaults"
printf 'escalate=0\n' > "$T/s"; NOW=$((NOW+10)); FAKE_COUNT=5 FAKE_AGE=1 run; NOW=$((NOW+400)); FAKE_COUNT=5 FAKE_AGE=410 run; check "$(conf)" "deny=5 unlock_time=300 " "with escalation off the numbers stay fixed"
# a user name that is not a plain name is ignored and writes nothing
rm -rf "$T/state"; mkdir "$T/state"; ZOHARA_FAILLOCK_USER='../x' ZOHARA_FAILLOCK_NOW=$NOW bash "$F" before; check "$(ls "$T/state" | wc -l)" "0" "a strange user name is ignored"
# the other settings of faillock.conf are kept
grep -q '^silent' "$T/faillock.conf"; check $? 0 "other faillock.conf lines are kept"
[ "$fails" = 0 ] && echo "all passed" || { echo "$fails failed"; exit 1; }
