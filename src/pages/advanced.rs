//! "Advanced" page — replaced the old `systemsettings` shortcut with a
//! real "About this device" panel. Also keeps an "Open full desktop
//! settings" row that spawns `systemsettings` only as an escape hatch
//! for power users who want KDE's full panel.
//!
//! This is the same approach Android uses: the in-app About page for
//! 95% of users, with a link to the system Settings app for the rest.

use gtk4::prelude::*;
use libadwaita as adw;
use adw::prelude::*;
use std::process::Command;

fn read_os_release() -> Vec<(String, String)> {
    let out = match std::fs::read_to_string("/etc/os-release") {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    out.lines()
        .filter_map(|l| {
            let mut parts = l.splitn(2, '=');
            let k = parts.next()?.to_string();
            let v = parts.next()?.trim_matches('"').to_string();
            Some((k, v))
        })
        .collect()
}

fn read_kernel() -> String {
    let out = Command::new("uname").arg("-r").output().ok();
    out.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn read_de() -> String {
    for k in ["XDG_CURRENT_DESKTOP", "DESKTOP_SESSION"] {
        if let Ok(v) = std::env::var(k) {
            if !v.is_empty() {
                return v;
            }
        }
    }
    "unknown".to_string()
}

fn read_wm() -> String {
    std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown".to_string())
}

fn read_uptime() -> String {
    let content = std::fs::read_to_string("/proc/uptime").unwrap_or_default();
    let secs: f64 = content
        .split_whitespace()
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);
    let h = (secs / 3600.0) as u32;
    let m = ((secs % 3600.0) / 60.0) as u32;
    format!("{h} hours, {m} minutes")
}

fn add_info_row(group: &adw::PreferencesGroup, title: &str, value: &str) {
    let row = adw::ActionRow::new();
    row.set_title(title);
    row.set_subtitle(value);
    row.set_activatable(false);
    group.add(&row);
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
        .label("About this device")
        .halign(gtk4::Align::Start)
        .css_classes(vec!["win11-page-title".to_string()])
        .build();
    root.append(&title);

    let rows = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    rows.set_css_classes(&["win11-card-group"]);

    // 1. Device identity
    let rel = read_os_release();
    let pretty = rel
        .iter()
        .find(|(k, _)| k == "PRETTY_NAME")
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| "Zohara OS".to_string());
    let hostname = std::fs::read_to_string("/etc/hostname")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "zohara".to_string());

    let dev_group = adw::PreferencesGroup::builder()
        .title("Device")
        .build();
    add_info_row(&dev_group, "Device name", &hostname);
    add_info_row(&dev_group, "Operating system", &pretty);
    add_info_row(&dev_group, "Kernel", &read_kernel());
    add_info_row(&dev_group, "Desktop environment", &read_de());
    add_info_row(&dev_group, "Session type", &read_wm());
    add_info_row(&dev_group, "Uptime", &read_uptime());
    rows.append(&dev_group);

    // 2. Hardware identity (basic)
    let hw_group = adw::PreferencesGroup::builder()
        .title("Hardware")
        .build();
    if let Ok(cpuinfo) = std::fs::read_to_string("/proc/cpuinfo") {
        let model = cpuinfo
            .lines()
            .find(|l| l.starts_with("model name"))
            .and_then(|l| l.split(':').nth(1))
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "unknown".to_string());
        add_info_row(&hw_group, "Processor", &model);
    }
    if let Ok(meminfo) = std::fs::read_to_string("/proc/meminfo") {
        let total = meminfo
            .lines()
            .find(|l| l.starts_with("MemTotal"))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|n| n.parse::<u64>().ok())
            .unwrap_or(0);
        let total_mb = total / 1024;
        add_info_row(&hw_group, "Memory", &format!("{total_mb} MB"));
    }
    rows.append(&hw_group);

    // 3. Power-user escape hatch: open KDE/Plasma full settings
    let more = adw::PreferencesGroup::builder()
        .title("More settings")
        .description("For options not covered by Zohara Settings, the desktop's full control panel is also available.")
        .build();
    let open_row = adw::ActionRow::new();
    open_row.set_title("Open full desktop settings");
    open_row.set_subtitle("KDE Plasma's systemsettings (advanced, use with care)");
    open_row.add_prefix(&gtk4::Image::from_icon_name("preferences-system-symbolic"));
    open_row.add_suffix(&gtk4::Image::from_icon_name("go-next-symbolic"));
    open_row.set_activatable(true);
    open_row.connect_activated(|_| {
        // Spawns the desktop's full settings panel — only if installed.
        let _ = Command::new("systemsettings").spawn();
    });
    more.add(&open_row);
    rows.append(&more);

    root.append(&rows);
    scroll.set_child(Some(&root));
    scroll.upcast()
}
