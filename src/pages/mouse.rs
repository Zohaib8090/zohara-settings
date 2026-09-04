//! Mouse settings — pointer speed, scroll speed, double-click time, primary button.

use gtk4::prelude::*;
use libadwaita as adw;
use adw::prelude::*;
use std::process::Command;

fn gset(schema: &str, key: &str, value: &str) {
    let _ = Command::new("gsettings")
        .args(["set", schema, key, value])
        .status();
}

fn gget(schema: &str, key: &str) -> Option<String> {
    let o = Command::new("gsettings")
        .args(["get", schema, key])
        .output()
        .ok()?;
    if o.status.success() {
        Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
    } else {
        None
    }
}

const SCHEMA: &str = "org.gnome.desktop.peripherals.mouse";

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
        .label("Mouse")
        .halign(gtk4::Align::Start)
        .css_classes(vec!["win11-page-title".to_string()])
        .build();
    root.append(&title);

    let rows = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    rows.set_css_classes(&["win11-card-group"]);

    // 1. Pointer speed
    let speed_exp = adw::ExpanderRow::new();
    speed_exp.set_title("Pointer speed");
    speed_exp.set_subtitle("How fast the pointer moves on screen");
    speed_exp.add_prefix(&gtk4::Image::from_icon_name("input-mouse-symbolic"));
    let speed_scale = gtk4::Scale::with_range(gtk4::Orientation::Horizontal, -1.0, 1.0, 0.1);
    speed_scale.set_value(-0.5);
    speed_scale.set_size_request(280, -1);
    speed_scale.set_hexpand(true);
    speed_scale.set_value_pos(gtk4::PositionType::Right);
    if let Some(v) = gget(SCHEMA, "speed") {
        if let Some(n) = v.parse::<f64>().ok() {
            speed_scale.set_value(n);
        }
    }
    speed_scale.connect_value_changed(|s| {
        gset(SCHEMA, "speed", &format!("{}", s.value()));
    });
    let speed_row = adw::ActionRow::new();
    speed_row.set_title("Speed");
    speed_row.add_suffix(&speed_scale);
    speed_row.set_activatable(false);
    speed_exp.add_row(&speed_row);
    rows.append(&speed_exp);

    // 2. Natural scroll
    let nat = adw::SwitchRow::new();
    nat.set_title("Scroll in the opposite direction");
    nat.set_subtitle("Scroll down moves the page down (natural scrolling)");
    if let Some(v) = gget(SCHEMA, "natural-scroll") {
        nat.set_active(v == "true");
    }
    nat.connect_active_notify(|row| {
        gset(SCHEMA, "natural-scroll", if row.is_active() { "true" } else { "false" });
    });
    rows.append(&nat);

    // 3. Primary button
    let primary = adw::ComboRow::new();
    primary.set_title("Primary mouse button");
    let p_list = gtk4::StringList::new(&["Left", "Right"]);
    primary.set_model(Some(&p_list));
    if let Some(v) = gget(SCHEMA, "primary-button") {
        if v == "1" { primary.set_selected(1); } else { primary.set_selected(0); }
    }
    primary.connect_selected_notify(|r| {
        gset(SCHEMA, "primary-button", if r.selected() == 1 { "1" } else { "0" });
    });
    let prim_exp = adw::ExpanderRow::new();
    prim_exp.set_title("Primary button");
    prim_exp.set_subtitle("Which button is the main click");
    prim_exp.add_prefix(&gtk4::Image::from_icon_name("input-mouse-symbolic"));
    prim_exp.add_row(&primary);
    rows.append(&prim_exp);

    // 4. Double-click speed
    let dc_scale = gtk4::Scale::with_range(gtk4::Orientation::Horizontal, 100.0, 1000.0, 50.0);
    if let Some(v) = gget(SCHEMA, "double-click") {
        if let Some(n) = v.parse::<f64>().ok() {
            dc_scale.set_value(n);
        } else {
            dc_scale.set_value(400.0);
        }
    } else {
        dc_scale.set_value(400.0);
    }
    dc_scale.set_size_request(280, -1);
    dc_scale.set_value_pos(gtk4::PositionType::Right);
    dc_scale.connect_value_changed(|s| {
        gset(SCHEMA, "double-click", &format!("{}", s.value() as i64));
    });
    let dc_row = adw::ActionRow::new();
    dc_row.set_title("Double-click time");
    dc_row.add_suffix(&dc_scale);
    dc_row.set_activatable(false);
    let dc_exp = adw::ExpanderRow::new();
    dc_exp.set_title("Double-click");
    dc_exp.set_subtitle("Maximum time between clicks (ms)");
    dc_exp.add_row(&dc_row);
    rows.append(&dc_exp);

    root.append(&rows);
    scroll.set_child(Some(&root));
    scroll.upcast()
}
