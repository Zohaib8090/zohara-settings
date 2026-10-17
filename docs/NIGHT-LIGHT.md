# Night light

`src/backend/night_light.rs` (logic, tested) and `build_night_light()` in `src/pages/display.rs`.

**What was broken (2026-10-17):** the switch only wrote `Active=true` into `kwinrc` and sent `org.kde.KWin.reconfigure`. KWin's Night Light does not
listen to that: it listens to the config-change message that `kwriteconfig6 --notify` sends. So KWin kept `enabled: false` and nothing happened.
Proved in the test VM (KWin's `org.kde.KWin.NightLight` properties before and after).

**Plasma 6.7 has two schedules only** (`/usr/share/config.kcfg/nightlightsettings.kcfg`): `Mode=DarkLight` (follows the system's day and night, the
default) and `Mode=Constant`. The old `Location`/`Times` modes and their keys are gone. Keys: `Active`, `Mode`, `NightTemperature` (`DayTemperature` stays 6500).

**Now:** a Night light group (switch, "When it gets dark / All the time", Warmth 2000 to 6000 K). Every write uses `--notify`, `Active` last. The line under the
switch is read from KWin every two seconds (`gdbus … org.kde.KWin.NightLight GetAll`): "Warming the screen now (4000 K)", "On. Waiting for the evening", "Off".
Verified in the VM by screenshot: on = the whole screen turns warm, off = normal.

Trap: KWin only computes day or night while Night Light is enabled (`daylight: true` and no schedule are normal while it is off).
