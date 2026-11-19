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
