//! Automatic time zone. The clock itself is kept right by systemd's time sync; this finds the time zone from the
//! internet connection (an approximate place looked up from the connection's address, no GPS and no account) and
//! sets it through systemd's `timedatectl`. A user timer runs `zohara-settings --auto-timezone` after login and then
//! every hour, so a laptop that travels changes zone by itself. It is on unless the person turns it off on the
//! Time & language page, and it stays out of the way when Location is switched off under Privacy.

use std::path::PathBuf;
use std::process::Command;

fn config_dir() -> PathBuf {
    std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config"))
        .join("zohara")
}

fn flag_path() -> PathBuf {
    config_dir().join("auto-timezone")
}

/// On unless the person turned it off (no file means on).
pub fn is_enabled() -> bool {
    std::fs::read_to_string(flag_path()).map(|t| t.trim() != "off").unwrap_or(true)
}

pub fn set_enabled(on: bool) {
    let _ = std::fs::create_dir_all(config_dir());
    let _ = std::fs::write(flag_path(), if on { "on\n" } else { "off\n" });
}

/// A zone name worth passing to `timedatectl`: plain characters, and a file that really exists in the zone database.
pub fn valid_zone(z: &str) -> bool {
    let z = z.trim();
    !z.is_empty()
        && z.len() <= 64
        && !z.starts_with('/')
        && !z.contains("..")
        && z.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-' | '/'))
        && std::path::Path::new("/usr/share/zoneinfo").join(z).is_file()
}

/// Pulls a zone out of the answer of a lookup service: `{"timezone":{"id":"Asia/Karachi"}}` (ipwho.is),
/// `{"timezone":"Asia/Karachi"}` (worldtimeapi.org) or plain text `Asia/Karachi` (ipapi.co). `check` decides if the
/// name is acceptable (the real check is `valid_zone`; tests use a stand-in).
pub fn parse_zone(text: &str, check: impl Fn(&str) -> bool) -> Option<String> {
    let t = text.trim();
    let name = if t.starts_with('{') {
        let v: serde_json::Value = serde_json::from_str(t).ok()?;
        if v.get("success").and_then(|s| s.as_bool()) == Some(false) {
            return None;
        }
        let tz = v.get("timezone")?;
        tz.as_str().or_else(|| tz.get("id").and_then(|i| i.as_str()))?.to_string()
    } else {
        t.to_string()
    };
    let name = name.trim().to_string();
    (!name.is_empty() && check(&name)).then_some(name)
}

/// Location services switched off in Privacy means the geoclue service is masked.
pub fn location_off() -> bool {
    Command::new("systemctl")
        .args(["is-enabled", "geoclue.service"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "masked")
        .unwrap_or(false)
}

pub fn current() -> Option<String> {
    let o = Command::new("timedatectl").args(["show", "-p", "Timezone", "--value"]).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

/// Asks the lookup services, one after another, until one answers with a real zone.
pub fn detect() -> Result<String, String> {
    const SERVICES: [&str; 3] = [
        "https://ipwho.is/?fields=success,timezone",
        "https://ipapi.co/timezone/",
        "https://worldtimeapi.org/api/ip",
    ];
    for url in SERVICES {
        let out = Command::new("curl").args(["-sf", "--http1.1", "-m", "10", "-A", "zohara-settings", url]).output();
        if let Ok(o) = out {
            if o.status.success() {
                if let Some(z) = parse_zone(&String::from_utf8_lossy(&o.stdout), valid_zone) {
                    return Ok(z);
                }
            }
        }
    }
    Err("Couldn't find your location. Check that you are online.".into())
}

/// Sets the system time zone (polkit lets the person at the desktop do this without a password).
pub fn apply(zone: &str) -> Result<(), String> {
    if !valid_zone(zone) {
        return Err("Not a time zone".into());
    }
    let o = Command::new("timedatectl").args(["set-timezone", zone]).output().map_err(|e| e.to_string())?;
    if o.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&o.stderr).trim().to_string())
    }
}

/// The background run: nothing to do when it is off or Location is off; otherwise a few tries (the network may
/// still be coming up right after login), then set the zone if it differs.
pub fn run_once() -> i32 {
    if !is_enabled() || location_off() {
        return 0;
    }
    let mut found = Err(String::new());
    for attempt in 0..4 {
        found = detect();
        if found.is_ok() {
            break;
        }
        if attempt < 3 {
            std::thread::sleep(std::time::Duration::from_secs(15));
        }
    }
    let Ok(zone) = found else { return 0 };
    if current().as_deref() == Some(zone.as_str()) {
        return 0;
    }
    match apply(&zone) {
        Ok(()) => {
            let _ = Command::new("notify-send")
                .args(["-a", "Zohara Settings", "-i", "preferences-system-time", "Time zone updated", &format!("Your time zone is now {zone}, found from your internet connection.")])
                .status();
            0
        }
        Err(e) => {
            eprintln!("could not set the time zone to {zone}: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn any(_: &str) -> bool {
        true
    }

    #[test]
    fn zones_are_read_from_each_service() {
        assert_eq!(parse_zone("{\"success\":true,\"timezone\":{\"id\":\"Asia/Karachi\",\"utc\":\"+05:00\"}}", any).as_deref(), Some("Asia/Karachi"));
        assert_eq!(parse_zone("{\"timezone\":\"Europe/Paris\",\"ip\":\"1.2.3.4\"}", any).as_deref(), Some("Europe/Paris"));
        assert_eq!(parse_zone("America/New_York\n", any).as_deref(), Some("America/New_York"));
    }

    #[test]
    fn failed_lookups_give_nothing() {
        assert_eq!(parse_zone("{\"success\":false,\"message\":\"limit\"}", any), None);
        assert_eq!(parse_zone("", any), None);
        assert_eq!(parse_zone("{\"error\":true}", any), None);
        assert_eq!(parse_zone("Undefined", |_| false), None);
    }

    #[test]
    fn odd_names_are_refused() {
        assert!(!valid_zone(""));
        assert!(!valid_zone("../etc/passwd"));
        assert!(!valid_zone("/etc/passwd"));
        assert!(!valid_zone("Asia/Karachi; rm -rf /"));
        assert!(!valid_zone("No/Such_Place_Here"));
    }

    #[test]
    fn utc_is_a_real_zone_where_the_database_exists() {
        if std::path::Path::new("/usr/share/zoneinfo/UTC").is_file() {
            assert!(valid_zone("UTC"));
            assert!(valid_zone("Etc/UTC"));
        }
    }
}
