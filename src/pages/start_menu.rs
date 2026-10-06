//! Personalization > Start: settings of the Start menu (Plasma's launcher applet). The applet's settings are
//! reached through Plasma's own scripting interface, so no applet ID has to be guessed and a running panel changes at once.

use crate::backend::worker::in_background;
use gtk4::prelude::*;
use libadwaita as adw;
use adw::prelude::*;
use std::process::Command;
use std::rc::Rc;

/// Runs a Plasma script and returns whatever it prints.
pub fn plasma_script(script: &str) -> Option<String> {
    let o = Command::new("dbus-send")
        .args(["--session", "--print-reply", "--type=method_call", "--dest=org.kde.plasmashell", "/PlasmaShell", "org.kde.PlasmaShell.evaluateScript"])
        .arg(format!("string:{script}"))
        .output()
        .ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).to_string())
}

/// Pulls the returned text out of dbus-send's `--print-reply` output (`string "..."`).
pub fn reply_text(out: &str) -> String {
    out.lines()
        .find_map(|l| l.trim().strip_prefix("string \""))
        .map(|s| s.trim_end_matches('"').to_string())
        .unwrap_or_default()
}

/// The launcher applets on every panel: JS that runs `body` for each one (`w`).
fn each_launcher(body: &str) -> String {
    format!(
        r#"panels().forEach(function (p) {{ p.widgets().forEach(function (w) {{
  if (w.type.indexOf("org.kde.plasma.kickoff") == 0 || w.type.indexOf("org.kde.plasma.kicker") == 0 || w.type.indexOf("org.kde.plasma.kickerdash") == 0) {{ {body} }}
}}); }});"#
    )
}

/// JS that sets one boolean in the launcher's General group.
pub fn set_bool_script(key: &str, on: bool) -> String {
    each_launcher(&format!(r#"w.currentConfigGroup = ["General"]; w.writeConfig("{key}", {on});"#))
}

/// JS that prints the value of one boolean of the first launcher found.
pub fn read_bool_script(key: &str) -> String {
    format!(
        r#"var out = ""; panels().forEach(function (p) {{ p.widgets().forEach(function (w) {{
  if (out == "" && w.type.indexOf("org.kde.plasma.kickoff") == 0) {{ w.currentConfigGroup = ["General"]; out = String(w.readConfig("{key}", false)); }}
}}); }}); print(out);"#
    )
}

/// Switches (key, title, subtitle). Each is a boolean the Kickoff launcher stores in its General group.
const SWITCHES: [(&str, &str, &str); 3] = [
    ("compactMode", "Compact list", "Smaller rows, so more apps fit"),
    ("alphaSort", "Sort apps A to Z", "Keep the app list in alphabetical order"),
    ("switchCategoryOnHover", "Open categories by hovering", "Show a category just by pointing at it"),
];

/// Removes the "recent files" lists that Start and apps show.
fn clear_recent() -> bool {
    let home = std::env::var("HOME").unwrap_or_default();
    let _ = std::fs::write(format!("{home}/.local/share/recently-used.xbel"), "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<xbel version=\"1.0\"></xbel>\n");
    if let Ok(rd) = std::fs::read_dir(format!("{home}/.local/share/RecentDocuments")) {
        for e in rd.flatten() {
            let _ = std::fs::remove_file(e.path());
        }
    }
    true
}

pub fn open(parent_row: &adw::ActionRow) {
    let content = super::personalization::dialog_content_box();
    let status = gtk4::Label::new(Some("Changes apply to the Start menu right away."));
    status.set_halign(gtk4::Align::Start);
    status.set_wrap(true);
    status.add_css_class("dim-label");

    let menu = adw::PreferencesGroup::new();
    menu.set_title("Start menu");

    for (key, title, sub) in SWITCHES {
        let row = adw::SwitchRow::new();
        row.set_title(title);
        row.set_subtitle(sub);
        let ready = Rc::new(std::cell::Cell::new(false));
        // Show what the menu really has, not a guess.
        {
            let (row, ready) = (row.clone(), ready.clone());
            in_background(move || plasma_script(&read_bool_script(key)).map(|o| reply_text(&o)), move |v| {
                if let Some(v) = v {
                    row.set_active(v == "true");
                }
                ready.set(true);
            });
        }
        let status = status.clone();
        row.connect_active_notify(move |r| {
            if !ready.get() {
                return;
            }
            let on = r.is_active();
            let status = status.clone();
            in_background(
                move || plasma_script(&set_bool_script(key, on)).is_some(),
                move |ok| status.set_text(&if ok { format!("Saved: {} {}", title.to_lowercase(), if on { "on" } else { "off" }) } else { "Plasma didn't accept the change. The Start menu can only be changed from inside the desktop session.".to_string() }),
            );
        });
        menu.add(&row);
    }

    let open_menu = adw::ActionRow::new();
    open_menu.set_title("Show the Start menu");
    open_menu.set_subtitle("See the change");
    let ob = gtk4::Button::with_label("Open");
    ob.set_valign(gtk4::Align::Center);
    ob.connect_clicked(|_| {
        let _ = Command::new("dbus-send")
            .args(["--session", "--type=method_call", "--dest=org.kde.plasmashell", "/PlasmaShell", "org.kde.PlasmaShell.activateLauncherMenu"])
            .status();
    });
    open_menu.add_suffix(&ob);
    menu.add(&open_menu);

    let edit = adw::ActionRow::new();
    edit.set_title("Edit the app list");
    edit.set_subtitle("Rename, hide or reorder apps and folders in Start");
    let eb = gtk4::Button::with_label("Edit");
    eb.set_valign(gtk4::Align::Center);
    let missing = Command::new("sh").args(["-c", "command -v kmenuedit"]).output().map(|o| !o.status.success()).unwrap_or(true);
    if missing {
        edit.set_subtitle("The menu editor isn't installed on this computer");
        eb.set_sensitive(false);
    }
    eb.connect_clicked(|_| {
        let _ = Command::new("kmenuedit").spawn();
    });
    edit.add_suffix(&eb);
    menu.add(&edit);
    content.append(&menu);

    let privacy = adw::PreferencesGroup::new();
    privacy.set_title("Recent files");
    let clear = adw::ActionRow::new();
    clear.set_title("Clear recent files");
    clear.set_subtitle("Forget the files you opened lately in Start and in apps");
    let cb = gtk4::Button::with_label("Clear");
    cb.set_valign(gtk4::Align::Center);
    {
        let status = status.clone();
        cb.connect_clicked(move |_| {
            let status = status.clone();
            in_background(clear_recent, move |ok| status.set_text(if ok { "Recent files cleared." } else { "Couldn't clear them." }));
        });
    }
    clear.add_suffix(&cb);
    privacy.add(&clear);
    content.append(&privacy);
    content.append(&status);

    super::personalization::open_settings_window_sized(parent_row, "Start", &content, 560, 560);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reply_text_reads_the_dbus_string() {
        assert_eq!(reply_text("method return time=1.0 sender=:1.5 -> destination=:1.9 serial=3 reply_serial=2\n   string \"true\"\n"), "true");
        assert_eq!(reply_text("method return\n"), "");
    }

    #[test]
    fn scripts_name_the_key_and_the_value() {
        let s = set_bool_script("compactMode", true);
        assert!(s.contains("writeConfig(\"compactMode\", true)"));
        assert!(s.contains("org.kde.plasma.kickoff"));
        assert!(read_bool_script("alphaSort").contains("readConfig(\"alphaSort\""));
    }
}
