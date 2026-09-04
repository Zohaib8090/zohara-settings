//! Power & battery settings.
//!
//! Talks to UPower (over D-Bus, but we use the `upower` CLI to avoid
//! linking the lib) and to logind for lid / power button actions.

use gtk4::prelude::*;
use libadwaita as adw;
use adw::prelude::*;
use std::process::Command;

fn read_upower() -> Option<(u32, bool, String)> {
    // upower -i /org/freedesktop/UPower/devices/battery_BAT0 | head -30
    let out = Command::new("upower")
        .args(["-i", "/org/freedesktop/UPower/devices/battery_BAT0"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let mut pct: u32 = 100;
    let mut charging = false;
    let mut state = String::from("discharging");
    for line in s.lines() {
        let line = line.trim();
        if line.starts_with("percentage:") {
            if let Some(num) = line.split_whitespace().nth(1) {
                pct = num.trim_end_matches('%').parse().unwrap_or(100);
            }
        } else if line.starts_with("state:") {
            state = line.split_whitespace().nth(1).unwrap_or("discharging").to_string();
        } else if line.starts_with("icon-name:") {
            // last
        }
    }
    charging = state == "charging" || state == "fully-charged";
    Some((pct, charging, state))
}

fn set_logind(key: &str, value: &str) {
    let _ = Command::new("sudo")
        .args(["-n", "tee", "-a", "/etc/systemd/logind.conf"])
        .arg(format!("/dev/stdin"))
        .spawn();
    // Simpler: just write to a file under /etc/systemd/logind.conf.d/
    // which doesn't require sudo at runtime for the read but does for the write.
    // For now we just log that the user should run `systemctl edit systemd-logind`.
    log::info!("to change {}, run: sudo systemctl edit systemd-logind", key);
    let _ = value;
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
        .label("Power & battery")
        .halign(gtk4::Align::Start)
        .css_classes(vec!["win11-page-title".to_string()])
        .build();
    root.append(&title);

    let rows = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    rows.set_css_classes(&["win11-card-group"]);

    // 1. Battery card (only if a battery is present)
    if let Some((pct, charging, state)) = read_upower() {
        let bat = adw::ActionRow::new();
        bat.set_title(&format!("Battery at {pct}%"));
        bat.set_subtitle(&format!(
            "{} {}",
            if charging { "Charging" } else { "On battery" },
            state
        ));
        bat.set_activatable(false);
        bat.add_prefix(&gtk4::Image::from_icon_name(
            if pct > 80 { "battery-level-100-symbolic" }
            else if pct > 40 { "battery-level-80-symbolic" }
            else if pct > 20 { "battery-level-40-symbolic" }
            else { "battery-level-20-symbolic" },
        ));
        rows.append(&bat);
    }

    // 2. Power profile (reuses the gaming.rs logic)
    let pp = adw::ComboRow::new();
    pp.set_title("Power mode");
    let pp_list = gtk4::StringList::new(&["Power saver", "Balanced (recommended)", "Performance"]);
    pp.set_model(Some(&pp_list));
    pp.set_selected(1);
    pp.connect_selected_notify(|row| {
        let profile = match row.selected() {
            0 => "power-saver",
            2 => "performance",
            _ => "balanced",
        };
        let _ = Command::new("powerprofilesctl").arg("set").arg(profile).spawn();
    });
    let pp_exp = adw::ExpanderRow::new();
    pp_exp.set_title("Power mode");
    pp_exp.set_subtitle("Switches the whole system between performance and battery life");
    pp_exp.add_prefix(&gtk4::Image::from_icon_name("battery-level-80-symbolic"));
    pp_exp.add_row(&pp);
    rows.append(&pp_exp);

    // 3. Screen timeout
    let screen = adw::ComboRow::new();
    screen.set_title("Turn off screen after");
    let screen_list = gtk4::StringList::new(&[
        "1 minute", "2 minutes", "5 minutes", "10 minutes", "15 minutes", "30 minutes", "Never",
    ]);
    screen.set_model(Some(&screen_list));
    screen.set_selected(3);
    let screen_exp = adw::ExpanderRow::new();
    screen_exp.set_title("Screen and sleep");
    screen_exp.set_subtitle("Idle time before the screen turns off and the system sleeps");
    screen_exp.add_prefix(&gtk4::Image::from_icon_name("video-display-symbolic"));
    screen_exp.add_row(&screen);
    rows.append(&screen_exp);

    // 4. Lid close action
    let lid = adw::ComboRow::new();
    lid.set_title("When plugged in");
    let lid_list = gtk4::StringList::new(&["Do nothing", "Suspend", "Hibernate", "Shut down"]);
    lid.set_model(Some(&lid_list));
    lid.set_selected(1);
    let lid_battery = adw::ComboRow::new();
    lid_battery.set_title("On battery");
    let lid_b_list = gtk4::StringList::new(&["Do nothing", "Suspend", "Hibernate", "Shut down"]);
    lid_battery.set_model(Some(&lid_b_list));
    lid_battery.set_selected(1);
    let lid_exp = adw::ExpanderRow::new();
    lid_exp.set_title("Lid close action");
    lid_exp.set_subtitle("What happens when you close the laptop lid");
    lid_exp.add_prefix(&gtk4::Image::from_icon_name("laptop-symbolic"));
    lid_exp.add_row(&lid);
    lid_exp.add_row(&lid_battery);
    rows.append(&lid_exp);

    let _ = set_logind; // reserved for future use

    root.append(&rows);
    scroll.set_child(Some(&root));
    scroll.upcast()
}
