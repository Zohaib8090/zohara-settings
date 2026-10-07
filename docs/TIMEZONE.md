# Time zone and automatic time zone

Settings > Time & language. The clock itself is kept right by systemd's time sync (the "Set time automatically" switch, `timedatectl set-ntp`);
this page also owns the **time zone**.

## The UTC bug (fixed 2026-10-07)
Older ISOs shipped `/etc/tmpfiles.d/zohara-localtime.conf` containing `L+ /etc/localtime - - - - /usr/share/zoneinfo/UTC`. `L+` replaces the
path every time systemd-tmpfiles runs: at every boot and, through Arch's `21-systemd-tmpfiles.hook`, after package updates. So whatever zone the
installer or the person chose went back to UTC. `zohara-settings.install` (`fix_localtime`) overwrites that file with a comment on an installed
system (never on the live ISO, `/run/archiso`), and repairs `/etc/localtime` if it is a plain text file. The ISO source no longer ships the rule
and makes `/etc/localtime` a real link in `customize_airootfs.sh`.

## Automatic time zone
* `src/backend/autotz.rs`. Config: `~/.config/zohara/auto-timezone` (`on`/`off`; no file = on).
* `zohara-settings --auto-timezone` (run by the user timer `zohara-settings-timezone.timer`: 30 s after login, then every hour):
  does nothing if off or if Location is off in Privacy (geoclue masked); otherwise asks, in order, `https://ipwho.is/?fields=success,timezone`,
  `https://ipapi.co/timezone/`, `https://worldtimeapi.org/api/ip` (4 tries, 15 s apart, because the network may still be coming up), accepts only
  a name that exists under `/usr/share/zoneinfo`, and runs `timedatectl set-timezone` when it differs. A notification says what changed.
* `data/50-zohara-timezone.rules` (polkit): the active local user may call `org.freedesktop.timedate1.set-timezone` without a password. This is a
  deliberate small relaxation so the background job can work; remove it and the job fails quietly.
* The page: "Set time zone automatically" switch. Turning it on looks up the zone at once and selects it in the list (same path as a manual pick, so
  errors are shown the same way); choosing a zone by hand turns the switch off.
* The lookup sends the connection's public address to those websites. It is approximate (city level), not GPS, and not GeoClue.

## Tested (VM, installed system, 2026-10-07)
Started on UTC; after the Settings update the old rule was switched off; `systemctl --user start zohara-settings-timezone.service` set
Asia/Karachi with no password; after a reboot the login screen showed local time. Not tested: behind a VPN, all lookup sites down (it leaves the zone
unchanged), a travelling laptop.
