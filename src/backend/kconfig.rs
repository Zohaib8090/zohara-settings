//! Thin wrappers over kreadconfig6/kwriteconfig6 (Plasma's config CLI).
//! Nested groups are given outermost first, e.g. `&["Applications", "org.kde.dolphin"]`.

use std::process::Command;

fn group_args(groups: &[&str]) -> Vec<String> {
    groups.iter().flat_map(|g| ["--group".to_string(), g.to_string()]).collect()
}

pub fn read(file: &str, groups: &[&str], key: &str) -> Option<String> {
    let o = Command::new("kreadconfig6")
        .args(["--file", file])
        .args(group_args(groups))
        .args(["--key", key])
        .output()
        .ok()?;
    let v = String::from_utf8_lossy(&o.stdout).trim().to_string();
    (o.status.success() && !v.is_empty()).then_some(v)
}

pub fn write(file: &str, groups: &[&str], key: &str, value: &str) {
    let _ = Command::new("kwriteconfig6")
        .args(["--file", file])
        .args(group_args(groups))
        .args(["--key", key, value])
        .status();
}

pub fn write_typed(file: &str, groups: &[&str], key: &str, ty: &str, value: &str) {
    let _ = Command::new("kwriteconfig6")
        .args(["--file", file])
        .args(group_args(groups))
        .args(["--key", key, "--type", ty, value])
        .status();
}

pub fn delete(file: &str, groups: &[&str], key: &str) {
    let _ = Command::new("kwriteconfig6")
        .args(["--file", file])
        .args(group_args(groups))
        .args(["--key", key, "--delete"])
        .status();
}

pub fn available() -> bool {
    Command::new("kwriteconfig6").arg("--help").output().is_ok()
}

/// Ask KWin to reread its config (animations, effects, ...).
pub fn kwin_reconfigure() {
    let _ = Command::new("dbus-send")
        .args(["--session", "--type=method_call", "--dest=org.kde.KWin", "/KWin", "org.kde.KWin.reconfigure"])
        .status();
}

/// Run config writes off the UI thread.
pub fn spawn(f: impl FnOnce() + Send + 'static) {
    std::thread::spawn(f);
}
