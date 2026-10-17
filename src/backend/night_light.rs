//! Night light: KWin's Night Light (Plasma 6.7), which warms the screen's colours.
//!
//! What was wrong before: the old switch only wrote `Active=true` into `kwinrc` and asked KWin to "reconfigure". KWin's Night
//! Light does not listen to that: it listens to the desktop's config-change message, which `kwriteconfig6 --notify` sends. So the
//! switch changed a file and nothing else (proved in the test VM: KWin kept `enabled: false`). With `--notify` the screen warms
//! within a few seconds.
//!
//! Plasma 6.7 has two schedules (`kdeglobals`-independent, see `/usr/share/config.kcfg/nightlightsettings.kcfg`):
//! * `DarkLight`: follows the system's day and night (KDE's own schedule), the default;
//! * `Constant`: always warm, at the chosen temperature.
//! The old `Location` and `Times` modes no longer exist.

use std::process::{Command, Stdio};

pub const MIN_TEMPERATURE: u32 = 2000;
pub const MAX_TEMPERATURE: u32 = 6000;
pub const DEFAULT_TEMPERATURE: u32 = 4500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Schedule {
    /// Warm at night, following the system's day and night.
    DayNight,
    /// Warm all the time.
    Always,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub active: bool,
    pub schedule: Schedule,
    pub temperature: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { active: false, schedule: Schedule::DayNight, temperature: DEFAULT_TEMPERATURE }
    }
}

pub fn clamp_temperature(t: u32) -> u32 {
    t.clamp(MIN_TEMPERATURE, MAX_TEMPERATURE)
}

pub fn schedule_from_mode(mode: &str) -> Schedule {
    if mode.trim() == "Constant" {
        Schedule::Always
    } else {
        Schedule::DayNight
    }
}

pub fn mode_name(s: Schedule) -> &'static str {
    match s {
        Schedule::Always => "Constant",
        Schedule::DayNight => "DarkLight",
    }
}

/// The `kwriteconfig6` arguments, one list per key. Every key is written with `--notify` so KWin hears about it; `Active` is last, so
/// that the schedule and temperature are already right when it turns on.
pub fn config_commands(s: &Settings) -> Vec<Vec<String>> {
    let key = |k: &str, v: String| -> Vec<String> {
        ["--notify", "--file", "kwinrc", "--group", "NightColor", "--key", k].iter().map(|x| x.to_string()).chain(std::iter::once(v)).collect()
    };
    vec![
        key("Mode", mode_name(s.schedule).to_string()),
        key("NightTemperature", clamp_temperature(s.temperature).to_string()),
        key("Active", s.active.to_string()),
    ]
}

fn kread(key: &str) -> Option<String> {
    let out = Command::new("kreadconfig6").args(["--file", "kwinrc", "--group", "NightColor", "--key", key]).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// What is saved now (defaults for anything missing).
pub fn read() -> Settings {
    let d = Settings::default();
    Settings {
        active: kread("Active").as_deref() == Some("true"),
        schedule: kread("Mode").map(|m| schedule_from_mode(&m)).unwrap_or(d.schedule),
        temperature: kread("NightTemperature").and_then(|t| t.parse().ok()).map(clamp_temperature).unwrap_or(d.temperature),
    }
}

/// Saves the settings and tells KWin. True when every write worked.
pub fn write(s: &Settings) -> bool {
    config_commands(s).iter().all(|args| Command::new("kwriteconfig6").args(args).status().map(|st| st.success()).unwrap_or(false))
}

pub fn available() -> bool {
    Command::new("kwriteconfig6").arg("--help").stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
}

/// What KWin says it is doing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub available: bool,
    pub enabled: bool,
    pub running: bool,
    pub daylight: bool,
    pub current: u32,
    pub target: u32,
}

/// Reads `gdbus call … GetAll` output such as `({'available': <true>, 'currentTemperature': <uint32 6500>, …},)`.
pub fn parse_status(text: &str) -> Option<Status> {
    let find = |name: &str| -> Option<&str> {
        let key = format!("'{name}': <");
        let start = text.find(&key)? + key.len();
        let end = text[start..].find('>')? + start;
        Some(text[start..end].trim())
    };
    let boolean = |name: &str| find(name).map(|v| v == "true").unwrap_or(false);
    let number = |name: &str| find(name).and_then(|v| v.rsplit(' ').next().and_then(|n| n.parse::<u32>().ok())).unwrap_or(0);
    find("currentTemperature")?;
    Some(Status {
        available: boolean("available"),
        enabled: boolean("enabled"),
        running: boolean("running"),
        daylight: boolean("daylight"),
        current: number("currentTemperature"),
        target: number("targetTemperature"),
    })
}

pub fn status() -> Option<Status> {
    let out = Command::new("gdbus")
        .args(["call", "--session", "--dest", "org.kde.KWin", "--object-path", "/org/kde/KWin/NightLight", "--method", "org.freedesktop.DBus.Properties.GetAll", "org.kde.KWin.NightLight"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status.success().then(|| parse_status(&String::from_utf8_lossy(&out.stdout))).flatten()
}

/// One line for the person.
pub fn status_text(s: &Settings, st: Option<&Status>) -> String {
    let Some(st) = st else { return "KWin did not answer".to_string() };
    if !st.available {
        return "Not available on this screen".to_string();
    }
    if !s.active {
        return "Off".to_string();
    }
    if st.running && st.current < 6400 {
        return format!("Warming the screen now ({} K)", st.current);
    }
    if st.running {
        return "On. The screen is getting warmer".to_string();
    }
    match s.schedule {
        Schedule::DayNight if st.daylight => "On. Waiting for the evening".to_string(),
        _ => "On".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_write_tells_kwin_and_the_switch_comes_last() {
        let cmds = config_commands(&Settings { active: true, schedule: Schedule::Always, temperature: 3500 });
        assert!(cmds.iter().all(|c| c[0] == "--notify" && c.contains(&"NightColor".to_string())));
        let keys: Vec<&str> = cmds.iter().map(|c| c[6].as_str()).collect();
        assert_eq!(keys, ["Mode", "NightTemperature", "Active"]);
        assert_eq!(cmds[0].last().unwrap(), "Constant");
        assert_eq!(cmds[1].last().unwrap(), "3500");
        assert_eq!(cmds[2].last().unwrap(), "true");
    }

    #[test]
    fn the_temperature_stays_in_the_range_kde_takes() {
        assert_eq!(clamp_temperature(100), MIN_TEMPERATURE);
        assert_eq!(clamp_temperature(99999), MAX_TEMPERATURE);
        let cmds = config_commands(&Settings { active: false, schedule: Schedule::DayNight, temperature: 100 });
        assert_eq!(cmds[1].last().unwrap(), &MIN_TEMPERATURE.to_string());
        assert_eq!(cmds[0].last().unwrap(), "DarkLight");
    }

    #[test]
    fn old_or_unknown_modes_read_as_the_day_night_schedule() {
        assert_eq!(schedule_from_mode("Constant"), Schedule::Always);
        for m in ["DarkLight", "", "Location", "Times", "Automatic"] {
            assert_eq!(schedule_from_mode(m), Schedule::DayNight, "{m}");
        }
    }

    const GETALL: &str = "({'available': <true>, 'currentTemperature': <uint32 4000>, 'daylight': <false>, 'enabled': <true>, 'inhibited': <false>, 'mode': <uint32 1>, 'running': <true>, 'targetTemperature': <uint32 4000>},)";

    #[test]
    fn kwins_answer_is_read() {
        let st = parse_status(GETALL).unwrap();
        assert_eq!(st, Status { available: true, enabled: true, running: true, daylight: false, current: 4000, target: 4000 });
        assert!(parse_status("nonsense").is_none());
    }

    #[test]
    fn the_status_line_says_what_is_happening() {
        let on = Settings { active: true, schedule: Schedule::DayNight, temperature: 4000 };
        let warm = parse_status(GETALL).unwrap();
        assert!(status_text(&on, Some(&warm)).contains("4000 K"));
        let waiting = Status { available: true, enabled: true, running: false, daylight: true, current: 6500, target: 6500 };
        assert!(status_text(&on, Some(&waiting)).contains("Waiting for the evening"));
        assert_eq!(status_text(&Settings::default(), Some(&waiting)), "Off");
        assert!(status_text(&on, None).contains("did not answer"));
        assert!(status_text(&on, Some(&Status::default())).contains("Not available"));
    }
}
