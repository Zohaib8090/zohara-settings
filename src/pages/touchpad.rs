//! Touchpad settings — tap-to-click, natural scroll, palm detection.

use gtk4::prelude::*;
use libadwaita as adw;
use adw::prelude::*;
use std::process::Command;

const SCHEMA: &str = "org.gnome.desktop.peripherals.touchpad";

fn gset(key: &str, value: &str) {
    let _ = Command::new("gsettings")
        .args(["set", SCHEMA, key, value])
        .status();
}

fn gget(key: &str) -> Option<String> {
    let o = Command::new("gsettings")
        .args(["get", SCHEMA, key])
        .output()
        .ok()?;
    if o.status.success() {
        Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
    } else {
        None
    }
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
        .label("Touchpad")
        .halign(gtk4::Align::Start)
        .css_classes(vec!["win11-page-title".to_string()])
        .build();
    root.append(&title);

    let rows = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    rows.set_css_classes(&["win11-card-group"]);

    let tap = adw::SwitchRow::new();
    tap.set_title("Tap to click");
    tap.set_subtitle("Tap the touchpad to register a left click");
    if let Some(v) = gget("tap-to-click") { tap.set_active(v == "true"); }
    tap.connect_active_notify(|r| gset("tap-to-click", if r.is_active() { "true" } else { "false" }));
    rows.append(&tap);

    let natural = adw::SwitchRow::new();
    natural.set_title("Natural scrolling");
    natural.set_subtitle("Scroll in the opposite direction (two-finger scroll moves content not view)");
    if let Some(v) = gget("natural-scroll") { natural.set_active(v == "true"); }
    natural.connect_active_notify(|r| gset("natural-scroll", if r.is_active() { "true" } else { "false" }));
    rows.append(&natural);

    let speed = adw::Scale::with_range(gtk4::Orientation::Horizontal, -1.0, 1.0, 0.1);
    if let Some(v) = gget("speed") {
        if let Ok(n) = v.parse::<f64>() { speed.set_value(n); }
    }
    speed.set_size_request(280, -1);
    speed.set_value_pos(gtk4::PositionType::Right);
    speed.connect_value_changed(|s| gset("speed", &format!("{}", s.value())));
    let speed_row = adw::ActionRow::new();
    speed_row.set_title("Pointer speed");
    speed_row.add_suffix(&speed);
    speed_row.set_activatable(false);
    let speed_exp = adw::ExpanderRow::new();
    speed_exp.set_title("Speed");
    speed_exp.add_row(&speed_row);
    rows.append(&speed_exp);

    root.append(&rows);
    scroll.set_child(Some(&root));
    scroll.upcast()
}
