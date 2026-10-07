//! The on-screen keyboard for touch screens: a button in the taskbar that shows or hides KDE's touch keyboard
//! (`plasma-keyboard`), and a first-login setup for computers that have a touch screen.
//!
//! * `zohara-settings --toggle-keyboard` (what the taskbar button runs): asks KWin to show the keyboard, or hide it if it
//!   is showing. KWin's `org.kde.KWin.VirtualKeyboard` interface is the same one Plasma's own lock-screen button uses.
//! * `zohara-settings --touch-setup` (autostart at login): on a computer with a touch screen, once, turn on the keyboard
//!   and add the taskbar button; if the keyboard is not installed, say so in a notification that opens Settings.

use std::path::PathBuf;
use std::process::Command;

pub const INPUT_METHOD: &str = "/usr/share/applications/org.kde.plasma.keyboard.desktop";
/// The desktop entry the taskbar button launches.
pub const BUTTON_ENTRY: &str = "file:///usr/share/applications/zohara-keyboard.desktop";

fn busctl(args: &[&str]) -> Option<String> {
    let o = Command::new("busctl").arg("--user").args(args).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

const KWIN: [&str; 3] = ["org.kde.KWin", "/VirtualKeyboard", "org.kde.kwin.VirtualKeyboard"];

/// `b true` / `b false` from busctl -> the bool.
pub fn parse_bool(out: &str) -> Option<bool> {
    match out.trim().strip_prefix("b ")?.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// Whether the keyboard is showing right now.
pub fn is_showing() -> bool {
    busctl(&["get-property", KWIN[0], KWIN[1], KWIN[2], "active"]).and_then(|o| parse_bool(&o)).unwrap_or(false)
}

/// Whether KWin has a keyboard to show (the input method is set and installed).
pub fn is_available() -> bool {
    busctl(&["get-property", KWIN[0], KWIN[1], KWIN[2], "available"]).and_then(|o| parse_bool(&o)).unwrap_or(false)
}

/// Show the keyboard, or hide it when it is showing. Exit code for the command line.
pub fn toggle() -> i32 {
    if is_showing() {
        return if busctl(&["set-property", KWIN[0], KWIN[1], KWIN[2], "active", "b", "false"]).is_some() { 0 } else { 1 };
    }
    if !is_available() {
        let _ = Command::new("notify-send")
            .args(["-a", "Zohara Settings", "-i", "input-keyboard-virtual", "On-screen keyboard", "It is not set up yet. Open Settings > Personalization > Text input and press Install."])
            .status();
        return 1;
    }
    if busctl(&["call", KWIN[0], KWIN[1], KWIN[2], "forceActivate"]).is_some() { 0 } else { 1 }
}

pub fn installed() -> bool {
    std::path::Path::new(INPUT_METHOD).exists()
}

/// Make KWin use the touch keyboard.
pub fn enable_input_method() {
    crate::backend::kconfig::write("kwinrc", &["Wayland"], "InputMethod", INPUT_METHOD);
    crate::backend::kconfig::kwin_reconfigure();
}

/// True when udev says the computer has a touch screen (not a touchpad).
pub fn has_touchscreen() -> bool {
    Command::new("udevadm")
        .args(["info", "--export-db"])
        .output()
        .map(|o| db_has_touchscreen(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or(false)
}

pub fn db_has_touchscreen(db: &str) -> bool {
    db.lines().any(|l| l.trim() == "E: ID_INPUT_TOUCHSCREEN=1")
}

// ── The taskbar button ─────────────────────────────────────────────────────

/// JS for the panel that holds the task manager: runs `body` with `p`.
fn on_taskbar_panel(body: &str) -> String {
    format!(
        r#"panels().forEach(function (p) {{
  var has = false;
  p.widgets().forEach(function (w) {{ if (w.type == "org.kde.plasma.icontasks" || w.type == "org.kde.plasma.taskmanager") has = true; }});
  if (!has) return;
  {body}
}});"#
    )
}

/// Adds the keyboard button just before the system tray (once).
pub fn add_button_script() -> String {
    on_taskbar_panel(&format!(
        r#"var found = false, tray = -1, ws = p.widgets();
  for (var i = 0; i < ws.length; i++) {{
    if (ws[i].type == "org.kde.plasma.icon") {{ ws[i].currentConfigGroup = ["General"]; if (String(ws[i].readConfig("url", "")).indexOf("zohara-keyboard") >= 0) found = true; }}
    if (ws[i].type == "org.kde.plasma.systemtray" && tray < 0) tray = i;
  }}
  if (!found) {{
    var b = p.addWidget("org.kde.plasma.icon");
    b.currentConfigGroup = ["General"];
    b.writeConfig("url", "{BUTTON_ENTRY}");
    if (tray >= 0) b.index = tray;
  }}"#
    ))
}

pub fn remove_button_script() -> String {
    on_taskbar_panel(
        r#"p.widgets().forEach(function (w) {
    if (w.type == "org.kde.plasma.icon") { w.currentConfigGroup = ["General"]; if (String(w.readConfig("url", "")).indexOf("zohara-keyboard") >= 0) w.remove(); }
  });"#,
    )
}

pub fn button_present_script() -> String {
    r#"var out = "no"; panels().forEach(function (p) { p.widgets().forEach(function (w) {
  if (w.type == "org.kde.plasma.icon") { w.currentConfigGroup = ["General"]; if (String(w.readConfig("url", "")).indexOf("zohara-keyboard") >= 0) out = "yes"; }
}); }); print(out);"#
        .to_string()
}

pub fn button_present() -> bool {
    crate::pages::start_menu::plasma_script(&button_present_script()).map(|o| crate::pages::start_menu::reply_text(&o) == "yes").unwrap_or(false)
}

pub fn set_button(on: bool) -> bool {
    let script = if on { add_button_script() } else { remove_button_script() };
    crate::pages::start_menu::plasma_script(&script).is_some()
}

// ── First-login setup on a touch screen ────────────────────────────────────

fn flag_path() -> PathBuf {
    std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config"))
        .join("zohara/touch-setup")
}

/// "done" once the keyboard and the button are set up, "asked" once the person was told the keyboard is missing.
fn flag() -> String {
    std::fs::read_to_string(flag_path()).unwrap_or_default().trim().to_string()
}

fn set_flag(v: &str) {
    let p = flag_path();
    let _ = std::fs::create_dir_all(p.parent().unwrap_or(&p));
    let _ = std::fs::write(p, format!("{v}\n"));
}

/// What the autostart run should do, from the facts (kept pure so it can be tested).
#[derive(Debug, PartialEq)]
pub enum Plan {
    Nothing,
    SetUp,
    TellToInstall,
}

pub fn plan(touchscreen: bool, installed: bool, flag: &str) -> Plan {
    if !touchscreen || flag == "done" {
        Plan::Nothing
    } else if installed {
        Plan::SetUp
    } else if flag == "asked" {
        Plan::Nothing
    } else {
        Plan::TellToInstall
    }
}

pub fn touch_setup() -> i32 {
    match plan(has_touchscreen(), installed(), &flag()) {
        Plan::Nothing => 0,
        Plan::SetUp => {
            enable_input_method();
            // The panel may still be starting right after login: try for a little while.
            for _ in 0..12 {
                if set_button(true) {
                    set_flag("done");
                    return 0;
                }
                std::thread::sleep(std::time::Duration::from_secs(5));
            }
            1
        }
        Plan::TellToInstall => {
            set_flag("asked");
            let o = Command::new("notify-send")
                .args(["-a", "Zohara Settings", "-i", "input-keyboard-virtual", "-A", "open=Set it up", "--wait", "This computer has a touch screen", "Install the on-screen keyboard to type by tapping. It adds a keyboard button to the taskbar."])
                .output();
            if let Ok(o) = o {
                if String::from_utf8_lossy(&o.stdout).trim() == "open" {
                    let _ = Command::new("zohara-settings").args(["--page", "Personalization"]).spawn();
                }
            }
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn busctl_booleans() {
        assert_eq!(parse_bool("b true"), Some(true));
        assert_eq!(parse_bool("b false\n"), Some(false));
        assert_eq!(parse_bool("s \"x\""), None);
        assert_eq!(parse_bool(""), None);
    }

    #[test]
    fn touchscreens_are_found_in_the_udev_database() {
        let touch = "P: /devices/x\nE: ID_INPUT=1\nE: ID_INPUT_TOUCHSCREEN=1\n\nP: /devices/y\nE: ID_INPUT_TOUCHPAD=1\n";
        assert!(db_has_touchscreen(touch));
        assert!(!db_has_touchscreen("E: ID_INPUT_TOUCHPAD=1\nE: ID_INPUT_MOUSE=1\n"));
        assert!(!db_has_touchscreen(""));
    }

    #[test]
    fn setup_plan() {
        assert_eq!(plan(false, true, ""), Plan::Nothing); // no touch screen
        assert_eq!(plan(true, true, "done"), Plan::Nothing); // already set up
        assert_eq!(plan(true, true, ""), Plan::SetUp);
        assert_eq!(plan(true, true, "asked"), Plan::SetUp); // installed since: set up now
        assert_eq!(plan(true, false, ""), Plan::TellToInstall);
        assert_eq!(plan(true, false, "asked"), Plan::Nothing); // do not nag at every login
    }

    #[test]
    fn panel_scripts_name_the_button() {
        let add = add_button_script();
        assert!(add.contains("zohara-keyboard.desktop"));
        assert!(add.contains("addWidget(\"org.kde.plasma.icon\")"));
        assert!(add.contains("org.kde.plasma.systemtray"));
        assert!(remove_button_script().contains(".remove()"));
        assert!(button_present_script().contains("print(out)"));
    }
}
