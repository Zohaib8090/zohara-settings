//! Virtual desktops ("dynamic windows"): how many there are, and the Dynamic desktops mode, where every new app gets a
//! desktop of its own and the desktop goes away when the app closes. The count is changed through KWin's D-Bus interface
//! (`org.kde.KWin.VirtualDesktopManager`); Dynamic desktops is a small KWin script (`zoharadynamicdesktops`, shipped in
//! `/usr/share/kwin/scripts`) that Settings turns on with kwinrc and configures in `[Script-zoharadynamicdesktops]`.

use super::kconfig;
use std::path::Path;
use std::process::Command;

pub const MIN: u32 = 1;
pub const MAX: u32 = 20;
const SCRIPT: &str = "zoharadynamicdesktops";
const GROUP: [&str; 1] = ["Script-zoharadynamicdesktops"];

fn manager(args: &[&str]) -> Option<String> {
    let o = Command::new("gdbus")
        .args(["call", "--session", "--dest", "org.kde.KWin", "--object-path", "/VirtualDesktopManager"])
        .args(args)
        .output()
        .ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).into_owned())
}

/// The desktops in order: `(position, id)`.
pub fn list() -> Vec<(u32, String)> {
    manager(&["--method", "org.freedesktop.DBus.Properties.Get", "org.kde.KWin.VirtualDesktopManager", "desktops"]).map(|a| parse_desktops(&a)).unwrap_or_default()
}

/// `(<[(uint32 0, 'id-a', 'name'), (1, 'id-b', 'name')]>,)` -> positions and ids. The first `(` is the answer's own.
pub fn parse_desktops(answer: &str) -> Vec<(u32, String)> {
    answer
        .match_indices('(')
        .skip(1)
        .filter_map(|(i, _)| {
            let tuple = &answer[i + 1..];
            let tuple = &tuple[..tuple.find(')')?];
            let mut parts = tuple.splitn(3, ',');
            let pos = parts.next()?.trim().trim_start_matches("uint32").trim().parse().ok()?;
            let id = parts.next()?.trim().trim_matches('\'').to_string();
            Some((pos, id))
        })
        .collect()
}

pub fn count() -> u32 {
    list().len() as u32
}

#[derive(Debug, PartialEq, Eq)]
pub enum Change {
    Nothing,
    Add(u32),
    Remove(u32),
}

pub fn plan(current: u32, want: u32) -> Change {
    let want = want.clamp(MIN, MAX);
    match want.cmp(&current) {
        std::cmp::Ordering::Equal => Change::Nothing,
        std::cmp::Ordering::Greater => Change::Add(want - current),
        std::cmp::Ordering::Less => Change::Remove(current - want),
    }
}

/// Makes the number of desktops `want` by adding at the end or removing from the end (KWin moves the windows of a
/// removed desktop to another one, nothing is closed).
pub fn set_count(want: u32) -> bool {
    let mut ok = true;
    match plan(count(), want) {
        Change::Nothing => {}
        Change::Add(n) => {
            for _ in 0..n {
                let at = count();
                ok &= manager(&["--method", "org.kde.KWin.VirtualDesktopManager.createDesktop", &at.to_string(), &format!("Desktop {}", at + 1)]).is_some();
            }
        }
        Change::Remove(n) => {
            for _ in 0..n {
                if let Some((_, id)) = list().pop() {
                    ok &= manager(&["--method", "org.kde.KWin.VirtualDesktopManager.removeDesktop", &id]).is_some();
                }
            }
        }
    }
    ok
}

#[derive(Clone, Debug, PartialEq)]
pub struct Dynamic {
    pub enabled: bool,
    /// true: every new app gets a desktop of its own. false (manual): apps open where they open and you send a window to a
    /// new desktop yourself (title bar menu or Meta+Shift+N).
    pub automatic: bool,
    /// Windows of an app that already has a desktop open there, instead of making another one.
    pub group_by_app: bool,
    pub switch_to_new: bool,
    pub close_empty: bool,
    pub max_desktops: u32,
    /// App names (comma separated) that never get a desktop of their own.
    pub ignore: String,
}

impl Default for Dynamic {
    fn default() -> Self {
        Dynamic { enabled: false, automatic: false, group_by_app: true, switch_to_new: true, close_empty: true, max_desktops: 12, ignore: String::new() }
    }
}

pub fn installed() -> bool {
    let home = std::env::var("HOME").unwrap_or_default();
    Path::new("/usr/share/kwin/scripts/zoharadynamicdesktops").exists() || Path::new(&format!("{home}/.local/share/kwin/scripts/zoharadynamicdesktops")).exists()
}

/// "Firefox,  KATE ,," -> "firefox,kate" (the script compares lower case).
pub fn clean_ignore(text: &str) -> String {
    text.split(',').map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(",")
}

pub fn read() -> Dynamic {
    let d = Dynamic::default();
    let flag = |k: &str, default: bool| kconfig::read("kwinrc", &GROUP, k).map(|v| v != "false").unwrap_or(default);
    Dynamic {
        enabled: kconfig::read("kwinrc", &["Plugins"], &format!("{SCRIPT}Enabled")).map(|v| v == "true").unwrap_or(false),
        automatic: kconfig::read("kwinrc", &GROUP, "Mode").map(|v| v == "auto").unwrap_or(d.automatic),
        group_by_app: flag("GroupByApp", d.group_by_app),
        switch_to_new: flag("SwitchToNew", d.switch_to_new),
        close_empty: flag("CloseEmpty", d.close_empty),
        max_desktops: kconfig::read("kwinrc", &GROUP, "MaxDesktops").and_then(|v| v.parse().ok()).map(|v: u32| v.clamp(2, MAX)).unwrap_or(d.max_desktops),
        ignore: kconfig::read("kwinrc", &GROUP, "Ignore").unwrap_or_default(),
    }
}

fn reconfigure() {
    let _ = Command::new("qdbus6").args(["org.kde.KWin", "/KWin", "reconfigure"]).status();
}

/// Saves the options and, when the mode is on, restarts the script so it reads them (a script reads its options once).
pub fn write(d: &Dynamic) {
    let w = |k: &str, v: String| kconfig::write("kwinrc", &GROUP, k, &v);
    w("Mode", if d.automatic { "auto" } else { "manual" }.to_string());
    w("GroupByApp", d.group_by_app.to_string());
    w("SwitchToNew", d.switch_to_new.to_string());
    w("CloseEmpty", d.close_empty.to_string());
    w("MaxDesktops", d.max_desktops.clamp(2, MAX).to_string());
    w("Ignore", clean_ignore(&d.ignore));
    let key = format!("{SCRIPT}Enabled");
    if d.enabled {
        // off and on again: KWin starts the script afresh and it reads the new options
        kconfig::write("kwinrc", &["Plugins"], &key, "false");
        reconfigure();
        kconfig::write("kwinrc", &["Plugins"], &key, "true");
    } else {
        kconfig::write("kwinrc", &["Plugins"], &key, "false");
    }
    reconfigure();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktops_are_read_from_the_shells_answer() {
        let a = "(<[(uint32 0, 'ef5f', '1'), (1, 'db1d', 'Desktop 2'), (2, '7c34', 'Desktop 3')]>,)";
        assert_eq!(parse_desktops(a), vec![(0, "ef5f".to_string()), (1, "db1d".to_string()), (2, "7c34".to_string())]);
        assert!(parse_desktops("").is_empty());
    }

    #[test]
    fn the_number_of_desktops_is_kept_in_range_and_changed_at_the_end() {
        assert_eq!(plan(3, 3), Change::Nothing);
        assert_eq!(plan(3, 5), Change::Add(2));
        assert_eq!(plan(5, 2), Change::Remove(3));
        assert_eq!(plan(3, 0), Change::Remove(2), "never fewer than one desktop");
        assert_eq!(plan(3, 99), Change::Add(MAX - 3));
    }

    #[test]
    fn the_ignore_list_is_tidied() {
        assert_eq!(clean_ignore("Firefox,  KATE ,,"), "firefox,kate");
        assert_eq!(clean_ignore(""), "");
    }

    #[test]
    fn the_script_in_this_repo_reads_every_option_used_here() {
        let js = include_str!("../../data/dynamic-desktops/contents/code/main.js");
        for key in ["Mode", "GroupByApp", "SwitchToNew", "CloseEmpty", "MaxDesktops", "Ignore"] {
            assert!(js.contains(&format!("readConfig(\"{key}\"")), "main.js does not read {key}");
        }
        assert!(include_str!("../../data/dynamic-desktops/metadata.json").contains(SCRIPT));
    }
}

#[cfg(test)]
mod live {
    use super::*;

    /// Adds one desktop to the real session and removes it again.
    #[test]
    #[ignore]
    fn live_desktops_can_be_added_and_removed_again() {
        let before = list();
        println!("desktops now: {before:?}");
        assert!(!before.is_empty());
        let n = before.len() as u32;
        assert!(set_count(n + 1));
        std::thread::sleep(std::time::Duration::from_millis(500));
        assert_eq!(count(), n + 1);
        assert!(set_count(n));
        std::thread::sleep(std::time::Duration::from_millis(500));
        assert_eq!(list(), before, "the original desktops must be back, same ids");
    }
}
