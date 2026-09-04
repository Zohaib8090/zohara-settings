//! Keyboard settings — repeat delay/rate, layouts.

use gtk4::prelude::*;
use libadwaita as adw;
use adw::prelude::*;
use std::process::Command;

const SCHEMA: &str = "org.gnome.desktop.peripherals.keyboard";

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
        .label("Keyboard")
        .halign(gtk4::Align::Start)
        .css_classes(vec!["win11-page-title".to_string()])
        .build();
    root.append(&title);

    let rows = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    rows.set_css_classes(&["win11-card-group"]);

    // Repeat delay
    let delay = gtk4::Scale::with_range(gtk4::Orientation::Horizontal, 100.0, 1000.0, 25.0);
    if let Some(v) = gget("delay") {
        if let Ok(n) = v.parse::<f64>() { delay.set_value(n); }
    }
    delay.set_size_request(280, -1);
    delay.set_value_pos(gtk4::PositionType::Right);
    delay.connect_value_changed(|s| gset("delay", &format!("{}", s.value() as i64)));
    let delay_row = adw::ActionRow::new();
    delay_row.set_title("Repeat delay");
    delay_row.add_suffix(&delay);
    delay_row.set_activatable(false);
    let delay_exp = adw::ExpanderRow::new();
    delay_exp.set_title("Typing");
    delay_exp.set_subtitle("Delay before a key starts repeating (ms)");
    delay_exp.add_prefix(&gtk4::Image::from_icon_name("input-keyboard-symbolic"));
    delay_exp.add_row(&delay_row);
    rows.append(&delay_exp);

    // Repeat rate
    let rate = gtk4::Scale::with_range(gtk4::Orientation::Horizontal, 10.0, 100.0, 1.0);
    if let Some(v) = gget("repeat") {
        // stored as fraction: 1/rate seconds; convert to "rate" (chars/sec)
        if let Ok(secs) = v.parse::<f64>() {
            if secs > 0.0 { rate.set_value(1.0 / secs); }
        }
    }
    rate.set_size_request(280, -1);
    rate.set_value_pos(gtk4::PositionType::Right);
    rate.connect_value_changed(|s| {
        let secs = if s.value() > 0.0 { 1.0 / s.value() } else { 1.0 };
        gset("repeat", &format!("{secs}"));
    });
    let rate_row = adw::ActionRow::new();
    rate_row.set_title("Repeat rate");
    rate_row.add_suffix(&rate);
    rate_row.set_activatable(false);
    let rate_exp = adw::ExpanderRow::new();
    rate_exp.set_title("Typing rate");
    rate_exp.set_subtitle("Characters per second when a key is held");
    rate_exp.add_row(&rate_row);
    rows.append(&rate_exp);

    // Layout
    let layout_exp = adw::ExpanderRow::new();
    layout_exp.set_title("Input sources");
    layout_exp.set_subtitle("Manage keyboard layouts (uses localectl)");
    layout_exp.add_prefix(&gtk4::Image::from_icon_name("input-keyboard-symbolic"));
    let layout_btn = gtk4::Button::builder()
        .label("Open layout settings")
        .valign(gtk4::Align::Center)
        .css_classes(vec!["win11-secondary-btn".to_string()])
        .build();
    layout_btn.connect_clicked(|_| {
        // The standard CLI to list current layout.
        let _ = Command::new("sh")
            .args(["-c", "localectl list-x11-keymap-layouts 2>/dev/null | head -1"])
            .spawn();
    });
    let layout_row = adw::ActionRow::new();
    layout_row.set_title("Show available layouts");
    layout_row.set_activatable(false);
    layout_row.add_suffix(&layout_btn);
    layout_exp.add_row(&layout_row);
    rows.append(&layout_exp);

    root.append(&rows);
    scroll.set_child(Some(&root));
    scroll.upcast()
}
