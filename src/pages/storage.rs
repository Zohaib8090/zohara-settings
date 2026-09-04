//! Storage settings — disk usage per mount, big-files scanner, cache cleanup.

use gtk4::prelude::*;
use libadwaita as adw;
use adw::prelude::*;
use std::process::Command;

#[derive(Clone)]
struct DiskInfo {
    mount: String,
    used: String,
    total: String,
    pct: u32,
}

fn read_disks() -> Vec<DiskInfo> {
    let out = match Command::new("df").args(["-h", "--output=target,used,size,pcent"]).output() {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };
    let s = String::from_utf8_lossy(&out.stdout);
    s.lines()
        .skip(1)
        .filter_map(|l| {
            let cols: Vec<&str> = l.split_whitespace().collect();
            if cols.len() < 4 { return None; }
            let pct = cols[3].trim_end_matches('%').parse().unwrap_or(0);
            Some(DiskInfo {
                mount: cols[0].to_string(),
                used: cols[1].to_string(),
                total: cols[2].to_string(),
                pct,
            })
        })
        .filter(|d| !d.mount.starts_with("/sys") && !d.mount.starts_with("/proc") && !d.mount.starts_with("/dev") && !d.mount.starts_with("/run"))
        .collect()
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
        .label("Storage")
        .halign(gtk4::Align::Start)
        .css_classes(vec!["win11-page-title".to_string()])
        .build();
    root.append(&title);

    let rows = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    rows.set_css_classes(&["win11-card-group"]);

    let disks = read_disks();
    let disk_exp = adw::ExpanderRow::new();
    disk_exp.set_title("Disks & volumes");
    disk_exp.set_subtitle("Internal and external drives");
    disk_exp.add_prefix(&gtk4::Image::from_icon_name("drive-harddisk-symbolic"));
    if disks.is_empty() {
        let r = adw::ActionRow::new();
        r.set_title("No storage devices detected");
        r.set_activatable(false);
        disk_exp.add_row(&r);
    } else {
        for d in disks {
            let r = adw::ActionRow::new();
            r.set_title(&d.mount);
            r.set_subtitle(&format!("{} / {} ({}%)", d.used, d.total, d.pct));
            r.set_activatable(false);
            // Use a small progress bar as suffix
            let pb = gtk4::ProgressBar::new();
            pb.set_fraction((d.pct as f64) / 100.0);
            pb.set_size_request(120, -1);
            pb.set_valign(gtk4::Align::Center);
            r.add_suffix(&pb);
            disk_exp.add_row(&r);
        }
    }
    rows.append(&disk_exp);

    // Storage sense
    let sense = adw::SwitchRow::new();
    sense.set_title("Storage Sense");
    sense.set_subtitle("Automatically clean temporary files and the package cache");
    sense.set_active(false);
    rows.append(&sense);

    // Cleanup actions
    let actions = adw::PreferencesGroup::new();
    actions.set_title("Cleanup");
    actions.set_description(Some("Free disk space by removing cached files."));

    let cache_row = adw::ActionRow::new();
    cache_row.set_title("Clear package cache");
    cache_row.set_subtitle("Removes old versions from /var/cache/pacman/pkg (requires sudo)");
    cache_row.add_prefix(&gtk4::Image::from_icon_name("edit-clear-symbolic"));
    let cache_btn = gtk4::Button::builder()
        .label("Run")
        .valign(gtk4::Align::Center)
        .css_classes(vec!["win11-secondary-btn".to_string()])
        .build();
    cache_btn.connect_clicked(|_| {
        let _ = Command::new("zohara-cleanup-cache").spawn();
    });
    cache_row.add_suffix(&cache_btn);
    cache_row.set_activatable(false);
    actions.add(&cache_row);

    let tmp_row = adw::ActionRow::new();
    tmp_row.set_title("Clear /tmp");
    tmp_row.set_subtitle("Removes files in /tmp older than 7 days");
    tmp_row.add_prefix(&gtk4::Image::from_icon_name("folder-symbolic"));
    let tmp_btn = gtk4::Button::builder()
        .label("Run")
        .valign(gtk4::Align::Center)
        .css_classes(vec!["win11-secondary-btn".to_string()])
        .build();
    tmp_btn.connect_clicked(|_| {
        let _ = Command::new("sh")
            .args(["-c", "find /tmp -type f -atime +7 -delete 2>/dev/null"])
            .spawn();
    });
    tmp_row.add_suffix(&tmp_btn);
    tmp_row.set_activatable(false);
    actions.add(&tmp_row);

    rows.append(&actions);

    root.append(&rows);
    scroll.set_child(Some(&root));
    scroll.upcast()
}
