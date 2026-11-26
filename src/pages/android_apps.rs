//! Apps > Android apps (Waydroid). Two switches that the helper `/usr/lib/zohara/zohara-waydroid-fix` carries out as root:
//!  * "Look like a normal phone": the container reports a release build instead of "userdebug / test-keys", which is what
//!    apps that say "can not run on rooted devices" read. On by default and applied by a service before each container start.
//!  * "Apps made for ARM phones": downloads a translation library (Intel: libhoudini, AMD: libndk) onto this computer. Off
//!    until the person turns it on, because it is downloaded from the internet and is not Zohara's to hand out.
//! See docs/WAYDROID-ARM-AND-ROOT.md.

use crate::backend::worker::in_background;
use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use std::process::Command;
use std::rc::Rc;

const HELPER: &str = "/usr/lib/zohara/zohara-waydroid-fix";
const STATUS_FILE: &str = "/var/lib/zohara/waydroid/status";
const CONF_FILE: &str = "/etc/zohara/waydroid.conf";
const IMAGE: &str = "/var/lib/waydroid/images/system.img";

/// What the helper last wrote to its status file (`key=value` lines).
#[derive(Debug, Default, PartialEq, Clone)]
pub struct Status {
    pub installed: bool,
    pub disguise_applied: bool,
    pub arm_present: bool,
    pub translator: String,
    pub restart_needed: bool,
    pub error: String,
}

pub fn parse_status(text: &str) -> Status {
    let mut s = Status::default();
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        match k.trim() {
            "waydroid" => s.installed = v.trim() == "installed",
            "disguise" => s.disguise_applied = v.trim() == "applied",
            "arm" => s.arm_present = v.trim() == "present",
            "translator" => s.translator = v.trim().to_string(),
            "restart" => s.restart_needed = v.trim() == "needed",
            "error" => s.error = v.trim().to_string(),
            _ => {}
        }
    }
    s
}

/// How the menu opens Android apps (`~/.config/zohara/android.conf`, `mode=`): one Android window with the app in front (the
/// default, works everywhere) or each app in its own desktop window (only where Waydroid's window mode works).
pub fn mode_text(windows: bool) -> String {
    format!("mode={}\n", if windows { "windows" } else { "window" })
}

/// `text` (the lines of android.conf) with `key` set to `value`, or removed when `value` is None; the other lines stay.
pub fn conf_set(text: &str, key: &str, value: Option<&str>) -> String {
    let prefix = format!("{key}=");
    let mut out: String = text.lines().filter(|l| !l.trim().starts_with(&prefix)).map(|l| format!("{l}\n")).collect();
    if let Some(v) = value {
        out.push_str(&format!("{key}={v}\n"));
    }
    out
}

/// The `shape=` value of android.conf for a shape: "450x960", or None for automatic.
pub fn shape_conf_value(index: usize) -> Option<String> {
    let (_, w, h) = SHAPES.get(index).copied().unwrap_or(SHAPES[0]);
    (w > 0 && h > 0).then(|| format!("{w}x{h}"))
}

/// Writes one key of android.conf, keeping the others (the window mode and the screen shape live in the same file).
fn conf_write(key: &str, value: Option<&str>) {
    let path = android_conf_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    let _ = std::fs::write(&path, conf_set(&old, key, value));
}

pub fn shape_in_conf(conf: &str) -> Option<String> {
    conf.lines().rev().find_map(|l| l.trim().strip_prefix("shape=")).map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

pub fn mode_is_windows(conf: &str) -> bool {
    conf.lines().rev().find_map(|l| l.trim().strip_prefix("mode=")).map(|v| v.trim() == "windows").unwrap_or(false)
}

/// The shapes of the Android screen in the single Android window: (label, width, height); automatic is 0x0 (follows the window).
pub const SHAPES: [(&str, u32, u32); 4] = [("Automatic (follows the window)", 0, 0), ("Phone", 450, 960), ("Tablet", 1100, 700), ("Large", 1600, 960)];

/// The `waydroid prop set` arguments for a shape: the two properties and their values ("" clears one).
pub fn shape_props(index: usize) -> [(&'static str, String); 2] {
    let (_, w, h) = SHAPES.get(index).copied().unwrap_or(SHAPES[0]);
    let v = |n: u32| if n == 0 { String::new() } else { n.to_string() };
    [("persist.waydroid.width", v(w)), ("persist.waydroid.height", v(h))]
}

fn android_conf_path() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config"))).unwrap_or_default();
    base.join("zohara/android.conf")
}

fn prop_set(name: &str, value: &str) {
    let _ = Command::new("waydroid").args(["prop", "set", name, value]).stderr(std::process::Stdio::null()).status();
}

/// A value from /etc/zohara/waydroid.conf (`key=value`), or `default`.
pub fn conf_value(text: &str, key: &str, default: bool) -> bool {
    text.lines().rev().find_map(|l| l.trim().strip_prefix(&format!("{key}="))).map(|v| v.trim() == "1").unwrap_or(default)
}

/// The root script that stores `$1` (0 or 1) as `key` in the config. Only the two known keys are accepted.
pub fn conf_script(key: &str) -> Option<String> {
    matches!(key, "disguise" | "arm").then(|| format!("mkdir -p /etc/zohara && touch {CONF_FILE} && sed -i '/^{key}=/d' {CONF_FILE} && echo \"{key}=$1\" >> {CONF_FILE}"))
}

fn read_status() -> Status {
    let mut s = parse_status(&std::fs::read_to_string(STATUS_FILE).unwrap_or_default());
    s.installed = std::path::Path::new(IMAGE).exists();
    s
}

fn wanted(key: &str, default: bool) -> bool {
    conf_value(&std::fs::read_to_string(CONF_FILE).unwrap_or_default(), key, default)
}

fn arm_running() -> bool {
    Command::new("systemctl").args(["is-active", "--quiet", "zohara-waydroid-arm.service"]).status().map(|s| s.success()).unwrap_or(false)
}

/// The line shown under the ARM switch.
pub fn arm_text(s: &Status, wanted: bool, running: bool) -> String {
    if !s.installed {
        return "Android apps are not installed on this computer".into();
    }
    if running {
        return "Downloading and installing. This can take a few minutes".into();
    }
    if s.arm_present {
        return "On: apps made for ARM phones can run".into();
    }
    if wanted {
        if s.error.is_empty() {
            return "Waiting to install (needs the internet)".into();
        }
        return format!("Not installed yet: {}", s.error);
    }
    if s.translator == "none" {
        return "This processor does not need or support it".into();
    }
    "Off. Turning it on downloads a translation library from the internet to this computer".into()
}

pub fn open(parent: &gtk4::Widget) {
    let win = adw::Window::builder().default_width(560).default_height(560).title("Android apps").build();
    if let Some(p) = parent.root().and_downcast::<gtk4::Window>() {
        win.set_transient_for(Some(&p));
        win.set_modal(true);
    }
    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::new();
    group.set_title("Android apps (Waydroid)");
    group.set_description(Some("Settings for apps that run in Zohara's Android container. They apply the next time Android starts"));

    // Where and how Android apps open
    let windows = adw::SwitchRow::new();
    windows.set_title("Each Android app in its own window");
    windows.set_subtitle("Off (recommended): the menu opens one Android window with the app in front. On: every app gets a desktop window of its own. It only works where Waydroid's window mode works on your graphics, and it applies the next time Android starts");
    let shape = adw::ComboRow::new();
    shape.set_title("Android screen shape");
    shape.set_subtitle("The size Android thinks its screen is: Phone makes apps look like on a phone. Applies the next time Android starts");
    shape.set_model(Some(&gtk4::StringList::new(&SHAPES.map(|s| s.0))));
    group.add(&windows);
    group.add(&shape);
    {
        let conf = android_conf_path();
        let text = std::fs::read_to_string(&conf).unwrap_or_default();
        windows.set_active(mode_is_windows(&text));
        if let Some(i) = (0..SHAPES.len() as u32).find(|&i| shape_conf_value(i as usize) == shape_in_conf(&text)) {
            shape.set_selected(i);
        }
        windows.connect_active_notify(move |r| {
            let on = r.is_active();
            conf_write("mode", Some(if on { "windows" } else { "window" }));
            let v = if on { "true" } else { "false" };
            std::thread::spawn(move || prop_set("persist.waydroid.multi_windows", v));
        });
        shape.connect_selected_notify(|r| {
            // saved in the config (the launcher applies it every time Android starts) and set now if Android runs
            conf_write("shape", shape_conf_value(r.selected() as usize).as_deref());
            let props = shape_props(r.selected() as usize);
            std::thread::spawn(move || {
                for (n, v) in props {
                    prop_set(n, &v);
                }
            });
        });
    }

    let disguise = adw::SwitchRow::new();
    disguise.set_title("Look like a normal phone");
    disguise.set_subtitle("Apps that refuse to run on rooted devices stop seeing a developer build. It does not hide real root, and it cannot pass Google's Play Integrity check");
    let arm = adw::SwitchRow::new();
    arm.set_title("Apps made for ARM phones");
    let note = gtk4::Label::new(None);
    note.set_wrap(true);
    note.set_halign(gtk4::Align::Start);
    note.add_css_class("dim-label");
    group.add(&disguise);
    group.add(&arm);
    let restart = adw::ActionRow::new();
    restart.set_title("Restart Android to finish");
    restart.set_subtitle("The change is made. Android needs to be stopped once so it starts with it");
    let rb = gtk4::Button::with_label("Stop Android");
    rb.set_valign(gtk4::Align::Center);
    rb.connect_clicked(|_| {
        let _ = Command::new("waydroid").args(["session", "stop"]).spawn();
    });
    restart.add_suffix(&rb);
    restart.set_visible(false);
    group.add(&restart);
    page.add(&group);
    let g2 = adw::PreferencesGroup::new();
    g2.add(&note);
    page.add(&g2);

    let header = adw::HeaderBar::new();
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&page));
    win.set_content(Some(&view));

    // switches are set from the stored choice; handlers ignore the changes made here
    let busy = Rc::new(std::cell::Cell::new(true));
    let refresh: Rc<dyn Fn()> = {
        let (disguise, arm, note, restart, busy) = (disguise.clone(), arm.clone(), note.clone(), restart.clone(), busy.clone());
        Rc::new(move || {
            let s = read_status();
            let (dw, aw) = (wanted("disguise", true), wanted("arm", false));
            busy.set(true);
            disguise.set_active(dw);
            arm.set_active(aw);
            busy.set(false);
            disguise.set_sensitive(s.installed);
            arm.set_sensitive(s.installed && s.translator != "none");
            arm.set_subtitle(&arm_text(&s, aw, arm_running()));
            restart.set_visible(s.installed && s.restart_needed);
            note.set_text(if s.installed { "" } else { "Install Android apps from Zohara Store first." });
        })
    };
    refresh();

    let toggle = |key: &'static str, extra: String, row: &adw::SwitchRow, busy: &Rc<std::cell::Cell<bool>>, refresh: &Rc<dyn Fn()>| {
        let (busy, refresh, extra) = (busy.clone(), refresh.clone(), extra);
        row.connect_active_notify(move |r| {
            if busy.get() {
                return;
            }
            let on = r.is_active();
            let Some(script) = conf_script(key) else { return };
            let full = if on { format!("{script}; {extra}") } else { format!("{script}; {HELPER} {}", if key == "disguise" { "undo-disguise" } else { "status" }) };
            let (value, refresh) = (if on { "1" } else { "0" }, refresh.clone());
            in_background(move || crate::pages::accounts::pkexec_sh(&full, &[value], None), move |_| refresh());
        });
    };
    toggle("disguise", format!("{HELPER} disguise"), &disguise, &busy, &refresh);
    toggle("arm", "systemctl start --no-block zohara-waydroid-arm.service".to_string(), &arm, &busy, &refresh);

    // while the window is open, follow the helper's progress
    {
        let (win, refresh) = (win.clone(), refresh.clone());
        gtk4::glib::timeout_add_seconds_local(3, move || {
            if !win.is_visible() {
                return gtk4::glib::ControlFlow::Break;
            }
            refresh();
            gtk4::glib::ControlFlow::Continue
        });
    }
    win.present();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_mode_and_screen_shape_are_stored_as_the_launcher_reads_them() {
        assert_eq!(mode_text(true), "mode=windows\n");
        assert_eq!(mode_text(false), "mode=window\n");
        assert!(mode_is_windows("mode=windows\n") && !mode_is_windows("mode=window\n") && !mode_is_windows(""));
        assert!(mode_is_windows("mode=window\nmode=windows\n"), "the last line wins");
        let phone = shape_props(1);
        assert_eq!((phone[0].1.as_str(), phone[1].1.as_str()), ("450", "960"));
        let auto = shape_props(0);
        assert!(auto[0].1.is_empty() && auto[1].1.is_empty(), "automatic clears both");
        assert_eq!(shape_props(99)[0].1, "", "an unknown shape means automatic");
        assert_eq!(shape_conf_value(1).as_deref(), Some("450x960"));
        assert_eq!(shape_conf_value(0), None);
        assert_eq!(conf_set("mode=windows\nshape=450x960\n", "shape", Some("1100x700")), "mode=windows\nshape=1100x700\n", "one key changes, the other stays");
        assert_eq!(conf_set("mode=windows\nshape=450x960\n", "shape", None), "mode=windows\n", "automatic removes the shape");
        assert_eq!(conf_set("", "mode", Some("window")), "mode=window\n");
        assert_eq!(shape_in_conf("mode=window\nshape=450x960\n").as_deref(), Some("450x960"));
        assert_eq!(shape_in_conf("mode=window\n"), None);
    }

    #[test]
    fn the_status_file_is_read() {
        let s = parse_status("waydroid=installed\ndisguise=applied\narm=absent\narm_wanted=1\ntranslator=libhoudini\nrestart=needed\nerror=no internet\n");
        assert!(s.installed && s.disguise_applied && !s.arm_present && s.restart_needed);
        assert_eq!((s.translator.as_str(), s.error.as_str()), ("libhoudini", "no internet"));
        assert_eq!(parse_status(""), Status::default());
    }

    #[test]
    fn the_config_is_read_with_defaults() {
        assert!(conf_value("", "disguise", true), "the build fix is on unless turned off");
        assert!(!conf_value("", "arm", false), "ARM is off until the person turns it on");
        assert!(!conf_value("disguise=0\n", "disguise", true));
        assert!(conf_value("arm=0\narm=1\n", "arm", false), "the last line wins");
    }

    #[test]
    fn only_the_two_known_keys_can_be_written() {
        assert!(conf_script("disguise").unwrap().contains("sed -i '/^disguise=/d'"));
        assert!(conf_script("arm").unwrap().contains("echo \"arm=$1\""));
        assert_eq!(conf_script("x; rm -rf /"), None);
        assert_eq!(conf_script("../etc"), None);
    }

    #[test]
    fn the_arm_line_says_what_is_going_on() {
        let base = Status { installed: true, translator: "libhoudini".into(), ..Default::default() };
        assert!(arm_text(&Status::default(), false, false).contains("not installed"));
        assert!(arm_text(&base, false, false).starts_with("Off"));
        assert!(arm_text(&base, true, true).contains("Downloading"));
        assert!(arm_text(&Status { arm_present: true, ..base.clone() }, true, false).starts_with("On"));
        assert!(arm_text(&Status { error: "no internet".into(), ..base.clone() }, true, false).contains("no internet"));
        assert!(arm_text(&Status { translator: "none".into(), ..base }, false, false).contains("does not need"));
    }
}
