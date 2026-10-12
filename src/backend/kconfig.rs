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

/// Like `write`, and tells the running programs that watch the file (Plasma's notification server does) that it changed.
/// Without this a change only shows up after they next read the file.
pub fn write_notify(file: &str, groups: &[&str], key: &str, value: &str) {
    let _ = Command::new("kwriteconfig6")
        .args(["--file", file])
        .args(group_args(groups))
        .args(["--key", key, "--notify", value])
        .status();
}

/// Like `delete`, announcing the change (see `write_notify`).
pub fn delete_notify(file: &str, groups: &[&str], key: &str) {
    let _ = Command::new("kwriteconfig6")
        .args(["--file", file])
        .args(group_args(groups))
        .args(["--key", key, "--delete", "--notify"])
        .status();
}

/// Tells Plasma and KDE apps that cursor settings changed (`KGlobalSettings.notifyChange(CursorChanged, 0)`): the
/// running desktop only picks up a new cursor size when it hears this, a changed config file is not enough.
pub fn notify_cursor_changed() {
    let _ = Command::new("dbus-send")
        .args(["--session", "--type=signal", "/KGlobalSettings", "org.kde.KGlobalSettings.notifyChange", "int32:5", "int32:0"])
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
