# Login lock that gets stricter (2026-11-22)

Stock Arch (`pambase` + `pam_faillock`, not Zohara's) refuses the right password for 10 minutes after 3 wrong ones. That is hard on the owner: on 2026-11-22 a Settings dialog and a mistyped password locked the only admin account and made it look as if both the old and the new password were wrong. Zohara now uses a gentler first round and gets stricter only when the lock is abused:

| round | tries | lock |
| --- | --- | --- |
| 1 (default) | 5 | 5 minutes |
| 2 (after a lock ended and the person failed again) | 4 | 10 minutes |
| 3 | 3 | 20 minutes |
| 4 | 2 | 40 minutes |
| 5 and every later one | 2 | 60 minutes (never longer, so a mistake never locks you out for good) |

A correct password puts everything back to round 1, and so do 24 hours without a lock.

## How
- `/usr/lib/zohara/zohara-faillock-policy` is called by PAM (`pam_exec`, control `optional`, `seteuid`) just before `pam_faillock` reads its settings and after a correct password. It counts the recent failures like pam_faillock does, decides the round, and rewrites only `deny`, `unlock_time` and `fail_interval` in `/etc/security/faillock.conf`. State per user: `/var/lib/zohara/faillock/<user>`.
- `/usr/lib/zohara/zohara-faillock-install apply|remove` adds or removes the two `optional` lines in `/etc/pam.d/system-auth` (only if it still has the three stock faillock lines; nothing is changed if both lines cannot be placed). `pambase` owns that file, so the pacman hook `zohara-faillock.hook` applies it again after each pambase update; the package install does it too.
- Numbers can be changed in `/etc/zohara/faillock.conf`: `tries=` (3-10, first round), `minutes=` (1-30, first round), `escalate=0` for fixed numbers. Out-of-range values fall back to 5 and 5.
- The lock screen (`/etc/pam.d/kde`) never locks (see its comment); sudo, the administrator pop-up, the sign-in screen and ssh use this.
- Undo: `sudo /usr/lib/zohara/zohara-faillock-install remove` (back to stock Arch; the numbers stay in faillock.conf). If a login is ever refused wrongly: boot menu, `e`, add `rw init=/bin/bash` to the `linux` line, Ctrl+X, `passwd <user>`, `faillock --user <user> --reset`, `sync; reboot -f`.

## Tested
- `tests/faillock-policy-test.sh`: the rounds, the one-hour cap, the daily reset, success reset, settings, bad numbers, a strange user name, other faillock.conf lines kept.
- Real PAM in an Arch container with sudo, a test user, small numbers (3 tries, 1 minute): the helper ran under pam_exec, the second round was 2 tries/2 minutes after the first lock ended, the correct password worked once the lock had ended, `remove` took the lines out again.

## Limits
- One faillock.conf serves every account: with several people failing at the same time, the last one to try decides the numbers.
- ssh failures count like any other; a remote guesser can push the owner's account into a stricter round (capped at one hour).
- A correct password given during a lock is still refused (stock behaviour) but resets the rounds.
- Not tested on a real desktop login (SDDM) or the administrator pop-up; a Settings page for the numbers is not built.

## 2026-11-26: tested with real PAM in the VM (ssh as a throwaway user)

* 5 wrong passwords lock the account; the right password is then refused until the lock ends.
* After the 5 minutes: the next round is 4 tries / 10 minutes (`deny = 4`, `unlock_time = 600`), checked in `faillock.conf`.
* **Bug found and fixed**: PAM runs the `success` step even when the right password is typed *during* a lock (pam_faillock refuses the login but the stack still passes through `authsucc`). That reset the escalation to round 1. The helper now ignores a `success` while the account is locked (a real success has no failures left in the tally, so it still resets).
* `su` does not use `system-auth` at all (it only calls `pam_unix`), so it is not locked by this; test with `ssh`, `sudo`, `login` or the sign-in screen.
