//! Notification settings — global do-not-disturb and per-app toggles.
//!
//! Backed by org.freedesktop.Notifications. We don't speak the protocol
//! directly; we just read / write gsettings keys under
//! `org.gnome.desktop.notifications` which most notification servers
//! (including the default Arch ones) honour.

use gtk4::prelude::*;
use libadwaita as adw;
use adw::prelude::*;
use std::process::Command;

fn gsettings_get(key: &str) -> Option<String> {
    Command::new("gsettings")
        .args(["get", "org.gnome.desktop.notifications", key])
        .output()
        .ok()
        .and_then(|o| if o.status.success() {
            Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
        } else { None })
}

fn gsettings_set(key: &str, value: &str) {
    let _ = Command::new("gsettings")
        .args(["set", "org.gnome.desktop.notifications", key, value])
        .status();
}

pub fn build() -> gtk4::Widget {
    let scroll = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .build();

    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
    root.set_margin_start(28);
    root.set_margin_end(28);
    root.set_margin_top(20);
    root.set_margin_bottom(32);

    let title = gtk4::Label::builder()
        .label("Notifications")
        .halign(gtk4::Align::Start)
        .css_classes(vec!["win11-page-title".to_string()])
        .build();
    root.append(&title);

    let rows = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    rows.set_css_classes(&["win11-card-group"]);

    // 1. Do not disturb
    let dnd = adw::SwitchRow::new();
    dnd.set_title("Do not disturb");
    dnd.set_subtitle("Silence all notifications (alarms still come through)");
    if let Some(v) = gsettings_get("show-banners") {
        dnd.set_active(v == "false");
    }
    dnd.connect_active_notify(|row| {
        let val = if row.is_active() { "false" } else { "true" };
        gsettings_set("show-banners", val);
    });
    rows.append(&dnd);

    // 2. Lock-screen notifications
    let lock = adw::SwitchRow::new();
    lock.set_title("Show notifications on lock screen");
    lock.set_subtitle("Notification content is hidden by default");
    if let Some(v) = gsettings_get("show-in-lock-screen") {
        lock.set_active(v == "true");
    }
    lock.connect_active_notify(|row| {
        let val = if row.is_active() { "true" } else { "false" };
        gsettings_set("show-in-lock-screen", val);
    });
    rows.append(&lock);

    // 3. Sounds
    let sounds = adw::SwitchRow::new();
    sounds.set_title("Play sounds for notifications");
    sounds.set_subtitle("Plays a chime with each notification");
    if let Some(v) = gsettings_get("enable-sound-alerts") {
        sounds.set_active(v != "false");
    }
    sounds.connect_active_notify(|row| {
        let val = if row.is_active() { "true" } else { "false" };
        gsettings_set("enable-sound-alerts", val);
    });
    rows.append(&sounds);

    // 4. Per-app section (placeholder, real impl walks .desktop files)
    let apps_exp = adw::ExpanderRow::new();
    apps_exp.set_title("Per-app notifications");
    apps_exp.set_subtitle("Choose which apps can notify you");
    apps_exp.add_prefix(&gtk4::Image::from_icon_name("applications-system-symbolic"));
    let placeholder = adw::ActionRow::new();
    placeholder.set_title("Coming soon");
    placeholder.set_subtitle("Per-app toggles will appear here once you've used each app once");
    apps_exp.add_row(&placeholder);
    rows.append(&apps_exp);

    root.append(&rows);
    scroll.set_child(Some(&root));
    scroll.upcast()
}
