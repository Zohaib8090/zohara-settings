//! X11 support. KDE removed Plasma's X11 session in Plasma 6.8 (X11 *programs* still run through Xwayland). A person who needs a
//! real X11 session therefore gets another desktop that still has one: Cinnamon or Xfce, added from Settings on request. On
//! Plasma 6.7 and older, Plasma's own X11 session (`plasma-x11-session`) is the first choice, so this works on both sides of the move.
//!
//! The install is one shell script run with one administrator prompt (`pkexec sh -c`), built from fixed package names only, never
//! from text the person typed. The package lists are refreshed (`-Sy`) only when the computer's mirror is pinned to an approved day
//! (then a refresh changes nothing in the official repositories, see `sysupdate::manifest::sync_args_safe`).

use std::fs;
use std::process::Command;

pub const XSESSIONS: &str = "/usr/share/xsessions";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Desktop {
    Cinnamon,
    Xfce,
    /// Plasma's own X11 session: only exists up to Plasma 6.7.
    PlasmaX11,
}

impl Desktop {
    pub fn name(self) -> &'static str {
        match self {
            Desktop::Cinnamon => "Cinnamon",
            Desktop::Xfce => "Xfce",
            Desktop::PlasmaX11 => "Plasma (X11)",
        }
    }
    /// The Arch packages (fixed text).
    pub fn packages(self) -> &'static str {
        match self {
            Desktop::Cinnamon => "cinnamon",
            Desktop::Xfce => "xfce4",
            Desktop::PlasmaX11 => "plasma-x11-session",
        }
    }
    /// The login entry that must exist afterwards.
    pub fn session_file(self) -> &'static str {
        match self {
            Desktop::Cinnamon => "cinnamon.desktop",
            Desktop::Xfce => "xfce.desktop",
            Desktop::PlasmaX11 => "plasmax11.desktop",
        }
    }
}

/// `6.7.5-3` -> `(6, 7)`.
pub fn plasma_series(version: &str) -> Option<(u32, u32)> {
    // an epoch ("1:6.10.2-1") comes first and is not part of the version
    let v = version.trim().rsplit(':').next().unwrap_or("");
    let v = v.trim_start_matches(|c: char| !c.is_ascii_digit());
    let mut it = v.split(|c: char| !c.is_ascii_digit());
    let major = it.next()?.parse().ok()?;
    let minor = it.next()?.parse().ok()?;
    Some((major, minor))
}

/// Plasma has an X11 session up to and including 6.7.
pub fn plasma_has_x11(series: (u32, u32)) -> bool {
    series < (6, 8)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub plasma: Option<(u32, u32)>,
    /// Login entries for X11 that exist (`cinnamon.desktop`, ...).
    pub sessions: Vec<String>,
}

impl Status {
    pub fn has(&self, d: Desktop) -> bool {
        self.sessions.iter().any(|s| s == d.session_file())
    }
    /// Plasma's own X11 session can be added or is there.
    pub fn plasma_x11_possible(&self) -> bool {
        self.plasma.map(plasma_has_x11).unwrap_or(false)
    }
    /// Every X11 desktop that is ready to pick on the sign-in screen, by name.
    pub fn ready(&self) -> Vec<&'static str> {
        [Desktop::PlasmaX11, Desktop::Cinnamon, Desktop::Xfce].into_iter().filter(|d| self.has(*d)).map(Desktop::name).collect()
    }
}

/// The file names in the X11 sessions folder.
fn session_files() -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(XSESSIONS)
        .map(|d| d.flatten().filter_map(|e| e.file_name().into_string().ok()).filter(|n| n.ends_with(".desktop")).collect())
        .unwrap_or_default();
    v.sort();
    v
}

pub fn status() -> Status {
    let plasma = Command::new("pacman")
        .args(["-Q", "plasma-workspace"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).split_whitespace().nth(1).map(str::to_string))
        .and_then(|v| plasma_series(&v));
    Status { plasma, sessions: session_files() }
}

// ── Install ────────────────────────────────────────────────────────────────

/// Exit code of the script: the packages are in but no login entry appeared.
pub const EXIT_NO_SESSION: i32 = 3;

pub fn explain_exit(code: Option<i32>) -> Option<String> {
    match code {
        Some(0) => None,
        Some(126) | Some(127) => Some("The password prompt was closed, so nothing was changed.".into()),
        Some(EXIT_NO_SESSION) => Some("The desktop was installed but did not add a login entry. Please tell Zohara support.".into()),
        _ => Some("It didn't work. Check that you are online. Nothing is left half-installed: pacman installs all or nothing.".into()),
    }
}

/// The administrator script that adds `d`. `pinned`: this computer's mirror is fixed to an approved day, so refreshing the package
/// lists is safe; otherwise the lists are used as they are.
pub fn install_script(d: Desktop, pinned: bool) -> String {
    let refresh = if pinned { "pacman -Sy --noconfirm\n" } else { "" };
    format!(
        "set -e\n{refresh}pacman -S --needed --noconfirm {pkgs}\n[ -f {XSESSIONS}/{file} ] || exit {EXIT_NO_SESSION}\n",
        pkgs = d.packages(),
        file = d.session_file()
    )
}

/// Runs the install; `None` when it worked, otherwise what to tell the person. Slow: call off the UI thread.
pub fn install(d: Desktop) -> Option<String> {
    let pinned = crate::sysupdate::manifest::current_pinned_date().is_some();
    let script = install_script(d, pinned);
    match Command::new("pkexec").args(["sh", "-c", &script]).status() {
        Ok(s) => explain_exit(s.code()),
        Err(_) => Some("Couldn't ask for the password.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plasma_versions_are_read() {
        assert_eq!(plasma_series("6.7.5-3"), Some((6, 7)));
        assert_eq!(plasma_series("6.8.0-1"), Some((6, 8)));
        assert_eq!(plasma_series("1:6.10.2-1"), Some((6, 10)));
        assert_eq!(plasma_series("unknown"), None);
        assert_eq!(plasma_series(""), None);
    }

    #[test]
    fn only_plasma_up_to_6_7_has_an_x11_session() {
        assert!(plasma_has_x11((6, 7)));
        assert!(plasma_has_x11((6, 4)));
        assert!(!plasma_has_x11((6, 8)));
        assert!(!plasma_has_x11((6, 10)));
        assert!(!plasma_has_x11((7, 0)), "a later major version has none either");
    }

    #[test]
    fn status_knows_what_is_ready() {
        let s = Status { plasma: Some((6, 7)), sessions: vec!["cinnamon.desktop".into(), "plasmax11.desktop".into(), "other.desktop".into()] };
        assert!(s.has(Desktop::Cinnamon) && s.has(Desktop::PlasmaX11) && !s.has(Desktop::Xfce));
        assert_eq!(s.ready(), vec!["Plasma (X11)", "Cinnamon"]);
        assert!(s.plasma_x11_possible());
        let new = Status { plasma: Some((6, 8)), sessions: vec![] };
        assert!(!new.plasma_x11_possible() && new.ready().is_empty());
        assert!(!Status { plasma: None, sessions: vec![] }.plasma_x11_possible());
    }

    #[test]
    fn the_script_uses_fixed_names_and_checks_the_login_entry() {
        let c = install_script(Desktop::Cinnamon, true);
        assert!(c.starts_with("set -e\npacman -Sy --noconfirm\n"));
        assert!(c.contains("pacman -S --needed --noconfirm cinnamon\n"));
        assert!(c.contains("/usr/share/xsessions/cinnamon.desktop"));
        let x = install_script(Desktop::Xfce, false);
        assert!(!x.contains("-Sy"), "no refresh on a system that is not pinned");
        assert!(x.contains("xfce4") && x.contains("xfce.desktop"));
        for d in [Desktop::Cinnamon, Desktop::Xfce, Desktop::PlasmaX11] {
            let s = install_script(d, true);
            let ok = Command::new("bash").args(["-n", "-c", &s]).status().map(|s| s.success()).unwrap_or(false);
            assert!(ok, "bash -n rejected the script for {}", d.name());
            // the refresh is only ever used together with a pinned mirror
            assert!(crate::sysupdate::manifest::sync_args_safe(&["-Sy", "--noconfirm"], true));
        }
    }

    #[test]
    fn exit_codes_are_explained() {
        assert!(explain_exit(Some(0)).is_none());
        assert!(explain_exit(Some(126)).unwrap().contains("password"));
        assert!(explain_exit(Some(EXIT_NO_SESSION)).unwrap().contains("login entry"));
        assert!(explain_exit(Some(1)).unwrap().contains("online"));
    }
}
