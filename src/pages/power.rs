//! Power & battery settings.
//!
//! Plasma's PowerDevil owns idle, sleep, lid and power-button handling (it
//! takes logind's lid/button inhibitors), so these settings are written to
//! its config, `powerdevilrc`, per profile (AC / Battery), and PowerDevil is
//! asked to reload. Battery state comes from UPower; power modes from
//! power-profiles-daemon.

use crate::backend::kconfig;
use crate::backend::worker::in_background;
use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use std::process::Command;

const RC: &str = "powerdevilrc";

fn output(cmd: &str, args: &[&str]) -> Option<String> {
    let o = Command::new(cmd).args(args).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

// ── Battery ────────────────────────────────────────────────────────────────

struct Battery {
    percent: f64,
    state: String,
    time: Option<String>,
}

fn read_battery() -> Option<Battery> {
    let path = output("upower", &["-e"])?.lines().find(|l| l.contains("battery_"))?.to_string();
    let info = output("upower", &["-i", &path])?;
    let field = |name: &str| {
        info.lines()
            .map(str::trim)
            .find_map(|l| l.strip_prefix(name).map(|v| v.trim_start_matches(':').trim().to_string()))
    };
    Some(Battery {
        percent: field("percentage")?.trim_end_matches('%').replace(',', ".").parse().ok()?,
        state: field("state").unwrap_or_default(),
        time: field("time to empty").or_else(|| field("time to full")),
    })
}

fn battery_group(b: &Battery) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("Battery");
    let row = adw::ActionRow::new();
    row.set_title(&format!("{:.0}%", b.percent));
    let state = match b.state.as_str() {
        "charging" => "Charging",
        "fully-charged" => "Fully charged",
        "pending-charge" => "Plugged in, not charging",
        "discharging" => "On battery",
        _ => "Unknown state",
    };
    row.set_subtitle(&match &b.time {
        Some(t) if b.state == "charging" => format!("{state} · full in {t}"),
        Some(t) if b.state == "discharging" => format!("{state} · {t} remaining"),
        _ => state.to_string(),
    });
    let level = ((b.percent / 10.0).round() as u32 * 10).min(100);
    let charging = if b.state == "charging" { "-charging" } else { "" };
    row.add_prefix(&gtk4::Image::from_icon_name(&format!("battery-level-{level}{charging}-symbolic")));
    let bar = gtk4::LevelBar::for_interval(0.0, 100.0);
    bar.set_value(b.percent);
    bar.set_size_request(160, -1);
    bar.set_valign(gtk4::Align::Center);
    row.add_suffix(&bar);
    row.set_activatable(false);
    g.add(&row);
    g
}

// ── Power mode ─────────────────────────────────────────────────────────────

fn power_mode_group() -> Option<adw::PreferencesGroup> {
    let current = output("powerprofilesctl", &["get"])?;
    let available: Vec<String> = output("powerprofilesctl", &["list"])
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let name = l.trim().trim_start_matches('*').trim().strip_suffix(':')?;
            (!name.contains(' ')).then(|| name.to_string())
        })
        .collect();
    let profiles: Vec<(&str, &str)> = [
        ("power-saver", "Power saver"),
        ("balanced", "Balanced"),
        ("performance", "Performance"),
    ]
    .into_iter()
    .filter(|(id, _)| available.is_empty() || available.iter().any(|a| a == id))
    .collect();

    let g = adw::PreferencesGroup::new();
    let row = adw::ComboRow::new();
    row.set_title("Power mode");
    row.set_subtitle("Trade battery life for performance");
    row.add_prefix(&gtk4::Image::from_icon_name("power-profile-balanced-symbolic"));
    let labels: Vec<&str> = profiles.iter().map(|(_, l)| *l).collect();
    row.set_model(Some(&gtk4::StringList::new(&labels)));
    if let Some(i) = profiles.iter().position(|(id, _)| *id == current) {
        row.set_selected(i as u32);
    }
    let ids: Vec<String> = profiles.iter().map(|(id, _)| id.to_string()).collect();
    row.connect_selected_notify(move |r| {
        if let Some(id) = ids.get(r.selected() as usize).cloned() {
            std::thread::spawn(move || {
                let _ = Command::new("powerprofilesctl").args(["set", &id]).status();
            });
        }
    });
    g.add(&row);
    Some(g)
}

// ── PowerDevil profile settings ────────────────────────────────────────────

fn reload_powerdevil() {
    let _ = Command::new("dbus-send")
        .args([
            "--session",
            "--type=method_call",
            "--dest=org.kde.Solid.PowerManagement",
            "/org/kde/Solid/PowerManagement",
            "org.kde.Solid.PowerManagement.refreshStatus",
        ])
        .status();
}

/// One PowerDevil key write (or delete when `value` is None).
type Write = (&'static str, &'static str, Option<String>);

fn apply(profile: &'static str, writes: Vec<Write>) {
    kconfig::spawn(move || {
        for (group, key, value) in writes {
            match value {
                Some(v) => kconfig::write(RC, &[profile, group], key, &v),
                None => kconfig::delete(RC, &[profile, group], key),
            }
        }
        reload_powerdevil();
    });
}

fn read(profile: &str, group: &str, key: &str) -> Option<String> {
    kconfig::read(RC, &[profile, group], key)
}

const MINUTES: [u32; 7] = [1, 2, 5, 10, 15, 30, 60];

fn minutes_label(m: u32) -> String {
    if m == 60 { "1 hour".into() } else if m == 1 { "1 minute".into() } else { format!("{m} minutes") }
}

/// "System default" / "Never" / N minutes, for a when-idle bool + timeout pair.
fn idle_row(
    title: &str,
    profile: &'static str,
    group: &'static str,
    enable_key: &'static str,
    timeout_key: &'static str,
    on_value: &'static str,
    off_value: &'static str,
) -> adw::ComboRow {
    let row = adw::ComboRow::new();
    row.set_title(title);
    let mut labels = vec!["System default".to_string(), "Never".to_string()];
    labels.extend(MINUTES.iter().map(|m| minutes_label(*m)));
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    row.set_model(Some(&gtk4::StringList::new(&refs)));

    let enabled = read(profile, group, enable_key);
    let secs: Option<u32> = read(profile, group, timeout_key).and_then(|v| v.parse().ok());
    let selected = if enabled.as_deref() == Some(off_value) {
        1
    } else {
        match secs.and_then(|s| MINUTES.iter().position(|m| m * 60 == s)) {
            Some(i) if enabled.is_some() || secs.is_some() => i + 2,
            _ => 0,
        }
    };
    row.set_selected(selected as u32);

    row.connect_selected_notify(move |r| {
        let writes: Vec<Write> = match r.selected() {
            0 => vec![(group, enable_key, None), (group, timeout_key, None)],
            1 => vec![(group, enable_key, Some(off_value.into()))],
            n => {
                let m = MINUTES[(n as usize - 2).min(MINUTES.len() - 1)];
                vec![(group, enable_key, Some(on_value.into())), (group, timeout_key, Some((m * 60).to_string()))]
            }
        };
        apply(profile, writes);
    });
    row
}

/// PowerDevil action codes (PowerButtonAction enum).
const ACTIONS: [(&str, &str); 6] = [
    ("Do nothing", "0"),
    ("Sleep", "1"),
    ("Hibernate", "2"),
    ("Shut down", "8"),
    ("Lock screen", "32"),
    ("Turn off screen", "64"),
];

fn action_row(title: &str, profile: &'static str, key: &'static str, choices: &[usize]) -> adw::ComboRow {
    let row = adw::ComboRow::new();
    row.set_title(title);
    let mut labels = vec!["System default"];
    labels.extend(choices.iter().map(|i| ACTIONS[*i].0));
    row.set_model(Some(&gtk4::StringList::new(&labels)));
    let current = read(profile, "SuspendAndShutdown", key);
    let idx = current
        .as_deref()
        .and_then(|v| choices.iter().position(|i| ACTIONS[*i].1 == v))
        .map(|i| i + 1)
        .unwrap_or(0);
    row.set_selected(idx as u32);
    let choices = choices.to_vec();
    row.connect_selected_notify(move |r| {
        let value = match r.selected() {
            0 => None,
            n => choices.get(n as usize - 1).map(|i| ACTIONS[*i].1.to_string()),
        };
        apply(profile, vec![("SuspendAndShutdown", key, value)]);
    });
    row
}

fn profile_group(title: &str, profile: &'static str, laptop: bool) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(title);
    g.add(&idle_row("Dim screen after", profile, "Display", "DimDisplayWhenIdle", "DimDisplayIdleTimeoutSec", "true", "false"));
    g.add(&idle_row(
        "Turn off screen after",
        profile,
        "Display",
        "TurnOffDisplayWhenIdle",
        "TurnOffDisplayIdleTimeoutSec",
        "true",
        "false",
    ));
    // AutoSuspendAction: 1 = sleep, 0 = do nothing.
    g.add(&idle_row(
        "Sleep after",
        profile,
        "SuspendAndShutdown",
        "AutoSuspendAction",
        "AutoSuspendIdleTimeoutSec",
        "1",
        "0",
    ));
    if laptop {
        g.add(&action_row("When the lid is closed", profile, "LidAction", &[0, 1, 2, 3, 4, 5]));
    }
    g.add(&action_row("When the power button is pressed", profile, "PowerButtonAction", &[0, 1, 2, 3, 4, 5]));
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
            .label("Power & battery")
            .halign(gtk4::Align::Start)
            .css_classes(vec!["win11-page-title".to_string()])
            .build(),
    );

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 24);
    root.append(&content);

    // upower/powerprofilesctl calls can take a moment; keep the page responsive.
    in_background(
        || (read_battery(), output("powerprofilesctl", &["get"]).is_some()),
        move |(battery, has_profiles)| {
            if let Some(b) = &battery {
                content.append(&battery_group(b));
            }
            if has_profiles {
                if let Some(g) = power_mode_group() {
                    content.append(&g);
                }
            }
            if !kconfig::available() {
                content.append(
                    &adw::StatusPage::builder()
                        .icon_name("dialog-information-symbolic")
                        .title("Power settings unavailable")
                        .description("Screen, sleep and lid settings are managed by Plasma's power service.")
                        .build(),
                );
                return;
            }
            let laptop = battery.is_some();
            if laptop {
                content.append(&profile_group("When plugged in", "AC", true));
                content.append(&profile_group("On battery", "Battery", true));
            } else {
                content.append(&profile_group("Screen and sleep", "AC", false));
            }
        },
    );

    scroll.set_child(Some(&root));
    scroll.upcast()
}
