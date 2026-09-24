//! Mouse settings -- per-device pointer speed, acceleration, scrolling and
//! handedness (via KWin), plus the desktop-wide double-click interval.

use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use std::process::Command;

fn double_click_group() -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("General");
    let row = adw::SpinRow::with_range(100.0, 2000.0, 50.0);
    row.set_title("Double-click interval");
    row.set_subtitle("Maximum time between clicks, in milliseconds");
    let current = Command::new("kreadconfig6")
        .args(["--file", "kdeglobals", "--group", "KDE", "--key", "DoubleClickInterval", "--default", "400"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<f64>().ok())
        .unwrap_or(400.0);
    row.set_value(current);
    row.connect_value_notify(|r| {
        let v = (r.value() as i64).to_string();
        std::thread::spawn(move || {
            let _ = Command::new("kwriteconfig6")
                .args(["--file", "kdeglobals", "--group", "KDE", "--key", "DoubleClickInterval", &v])
                .status();
        });
    });
    g.add(&row);
    g
}

pub fn build() -> gtk4::Widget {
    let scroll = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .build();

    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 24);
    root.set_margin_start(28);
    root.set_margin_end(28);
    root.set_margin_top(20);
    root.set_margin_bottom(32);

    root.append(
        &gtk4::Label::builder()
            .label("Mouse")
            .halign(gtk4::Align::Start)
            .css_classes(vec!["win11-page-title".to_string()])
            .build(),
    );

    let devices = gtk4::Box::new(gtk4::Orientation::Vertical, 24);
    super::input_devices::populate(&devices, false);
    root.append(&devices);
    root.append(&double_click_group());

    scroll.set_child(Some(&root));
    scroll.upcast()
}
