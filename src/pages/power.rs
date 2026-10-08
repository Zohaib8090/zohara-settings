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
use std::rc::Rc;

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
    on_value: impl Fn() -> String + 'static,
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
                vec![(group, enable_key, Some(on_value())), (group, timeout_key, Some((m * 60).to_string()))]
            }
        };
        apply(profile, writes);
    });
    row
}

/// PowerDevil action codes (PowerButtonAction enum).
const ACTIONS: [(&str, &str); 8] = [
    ("Do nothing", "0"),
    ("Sleep", "1"),
    ("Hibernate", "2"),
    ("Shut down", "8"),
    ("Lock screen", "32"),
    ("Turn off screen", "64"),
    ("Hybrid sleep", "4"),
    ("Ask me what to do", "16"),
];

fn action_row(title: &str, profile: &'static str, key: &'static str, choices: &[usize]) -> adw::ComboRow {
    let row = adw::ComboRow::new();
    row.set_title(title);
    // Hibernate (2) and hybrid sleep (6) need swap and a resume option; hide them when the computer cannot do them.
    let hib = crate::backend::hibernate::can_hibernate();
    let choices: Vec<usize> = choices.iter().copied().filter(|i| hib || !matches!(ACTIONS[*i].1, "2" | "4")).collect();
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
    row.connect_selected_notify(move |r| {
        let value = match r.selected() {
            0 => None,
            n => choices.get(n as usize - 1).map(|i| ACTIONS[*i].1.to_string()),
        };
        apply(profile, vec![("SuspendAndShutdown", key, value)]);
    });
    row
}

/// "Sleep after" plus "When idle for that long": one timeout row and one row for what to do, sharing the chosen action
/// (PowerDevil keeps both in `AutoSuspendAction`: 0 = never, 1 = sleep, 2 = hibernate, 8 = shut down).
const IDLE_ACTIONS_ALL: [(&str, &str); 3] = [("Sleep", "1"), ("Hibernate", "2"), ("Shut down", "8")];

/// The idle actions this computer can really do (no Hibernate without swap, see `backend::hibernate`).
fn idle_actions() -> Vec<(&'static str, &'static str)> {
    let hib = crate::backend::hibernate::can_hibernate();
    IDLE_ACTIONS_ALL.iter().copied().filter(|(_, v)| hib || *v != "2").collect()
}

fn sleep_rows(profile: &'static str) -> (adw::ComboRow, adw::ComboRow) {
    const GROUP: &str = "SuspendAndShutdown";
    let current = read(profile, GROUP, "AutoSuspendAction").unwrap_or_default();
    let idle = Rc::new(idle_actions());
    let chosen = Rc::new(std::cell::Cell::new(idle.iter().position(|(_, v)| *v == current).unwrap_or(0)));

    let pick = chosen.clone();
    let timeout = idle_row("Sleep after", profile, GROUP, "AutoSuspendAction", "AutoSuspendIdleTimeoutSec", { let idle = idle.clone(); move || idle[pick.get()].1.to_string() }, "0");
    let what = adw::ComboRow::new();
    what.set_title("When idle for that long");
    what.set_subtitle("Used when \"Sleep after\" has a time");
    what.set_model(Some(&gtk4::StringList::new(&idle.iter().map(|a| a.0).collect::<Vec<_>>())));
    what.set_selected(chosen.get() as u32);
    {
        let chosen = chosen.clone();
        let idle = idle.clone();
        what.connect_selected_notify(move |r| {
            let i = (r.selected() as usize).min(idle.len() - 1);
            chosen.set(i);
            // Only change what is stored when a sleep time is set (otherwise it would switch idle sleep on).
            let on = matches!(read(profile, GROUP, "AutoSuspendAction").as_deref(), Some(v) if v != "0");
            if on {
                apply(profile, vec![(GROUP, "AutoSuspendAction", Some(idle[i].1.to_string()))]);
            }
        });
    }
    (timeout, what)
}

fn switch_row(title: &str, subtitle: &str, profile: &'static str, group: &'static str, key: &'static str, default: bool) -> adw::SwitchRow {
    let row = adw::SwitchRow::new();
    row.set_title(title);
    row.set_subtitle(subtitle);
    row.set_active(read(profile, group, key).map(|v| v != "false").unwrap_or(default));
    row.connect_active_notify(move |r| {
        let v = if r.is_active() { "true" } else { "false" };
        apply(profile, vec![(group, key, Some(v.to_string()))]);
    });
    row
}

fn profile_group(title: &str, profile: &'static str, laptop: bool) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(title);
    g.add(&idle_row("Dim screen after", profile, "Display", "DimDisplayWhenIdle", "DimDisplayIdleTimeoutSec", || "true".to_string(), "false"));
    g.add(&idle_row("Turn off screen after", profile, "Display", "TurnOffDisplayWhenIdle", "TurnOffDisplayIdleTimeoutSec", || "true".to_string(), "false"));
    let (timeout, what) = sleep_rows(profile);
    g.add(&timeout);
    g.add(&what);
    if laptop {
        g.add(&action_row("When the lid is closed", profile, "LidAction", &[0, 1, 2, 3, 4, 5, 6]));
        g.add(&switch_row(
            "Keep running when the lid is closed and a screen is plugged in",
            "Do nothing on lid close while an external monitor is connected",
            profile,
            "SuspendAndShutdown",
            "InhibitLidActionWhenExternalMonitorPresent",
            true,
        ));
    }
    g.add(&action_row("When the power button is pressed", profile, "PowerButtonAction", &[0, 1, 2, 3, 4, 5, 6, 7]));
    g.add(&action_row("When the power button is held", profile, "PowerDownAction", &[0, 1, 2, 3, 4, 5, 6, 7]));
    g
}

// ── Locking (the lock screen's own settings, same file Personalization > Lock screen writes) ──────────────────────

const LOCK_RC: &str = "kscreenlockerrc";

fn lock_group() -> adw::PreferencesGroup {
    use super::lockscreen::{grace_choice_for, lock_choice_for, GRACE_CHOICES, LOCK_CHOICES};
    let g = adw::PreferencesGroup::new();
    g.set_title("Locking");
    g.set_description(Some("Applies to every power state"));
    let bool_of = |key: &str, default: bool| kconfig::read(LOCK_RC, &["Daemon"], key).map(|v| v != "false").unwrap_or(default);

    let auto = bool_of("Autolock", true);
    let minutes: u32 = kconfig::read(LOCK_RC, &["Daemon"], "Timeout").and_then(|v| v.parse().ok()).unwrap_or(5);
    let timeout = adw::ComboRow::new();
    timeout.set_title("Lock the screen after");
    timeout.set_subtitle("Lock when the computer has been left alone");
    timeout.set_model(Some(&gtk4::StringList::new(&LOCK_CHOICES.map(|c| c.0))));
    timeout.set_selected(lock_choice_for(auto, minutes));
    timeout.connect_selected_notify(|r| {
        let (_, m) = LOCK_CHOICES[(r.selected() as usize).min(LOCK_CHOICES.len() - 1)];
        kconfig::spawn(move || match m {
            Some(m) => {
                kconfig::write_typed(LOCK_RC, &["Daemon"], "Autolock", "bool", "true");
                kconfig::write(LOCK_RC, &["Daemon"], "Timeout", &m.to_string());
            }
            None => kconfig::write_typed(LOCK_RC, &["Daemon"], "Autolock", "bool", "false"),
        });
    });
    g.add(&timeout);

    let resume = adw::SwitchRow::new();
    resume.set_title("Lock after waking from sleep");
    resume.set_subtitle("Ask for the password when the computer wakes up");
    resume.set_active(bool_of("LockOnResume", true));
    resume.connect_active_notify(|s| {
        let v = if s.is_active() { "true" } else { "false" };
        kconfig::spawn(move || kconfig::write_typed(LOCK_RC, &["Daemon"], "LockOnResume", "bool", v));
    });
    g.add(&resume);

    let grace = adw::ComboRow::new();
    grace.set_title("Ask for the password");
    grace.set_subtitle("How long after locking before the password is needed");
    grace.set_model(Some(&gtk4::StringList::new(&GRACE_CHOICES.map(|c| c.0))));
    let now: u32 = kconfig::read(LOCK_RC, &["Daemon"], "LockGrace").and_then(|v| v.parse().ok()).unwrap_or(5);
    grace.set_selected(grace_choice_for(now));
    grace.connect_selected_notify(|r| {
        let (_, secs) = GRACE_CHOICES[(r.selected() as usize).min(GRACE_CHOICES.len() - 1)];
        kconfig::spawn(move || kconfig::write(LOCK_RC, &["Daemon"], "LockGrace", &secs.to_string()));
    });
    g.add(&grace);

    let now_row = adw::ActionRow::new();
    now_row.set_title("Lock the screen now");
    let b = gtk4::Button::with_label("Lock now");
    b.set_valign(gtk4::Align::Center);
    b.connect_clicked(|_| {
        let _ = Command::new("loginctl").arg("lock-session").status();
    });
    now_row.add_suffix(&b);
    g.add(&now_row);
    g
}

// ── Low battery (shared by both power states) ──────────────────────────────────────────────────────────────────

const LOW_LEVELS: [u32; 5] = [10, 15, 20, 25, 30];
const CRITICAL_LEVELS: [u32; 5] = [3, 5, 7, 10, 15];
const CRITICAL_ACTIONS_ALL: [(&str, &str); 4] = [("Sleep", "1"), ("Hibernate", "2"), ("Shut down", "8"), ("Do nothing", "0")];

fn battery_levels_group() -> adw::PreferencesGroup {
    const GROUP: &str = "BatteryManagement";
    let g = adw::PreferencesGroup::new();
    g.set_title("Low battery");

    let level_row = |title: &str, subtitle: &str, key: &'static str, levels: &'static [u32], default: u32| {
        let row = adw::ComboRow::new();
        row.set_title(title);
        row.set_subtitle(subtitle);
        let labels: Vec<String> = levels.iter().map(|l| format!("{l}%")).collect();
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        row.set_model(Some(&gtk4::StringList::new(&refs)));
        let cur: u32 = kconfig::read(RC, &[GROUP], key).and_then(|v| v.parse().ok()).unwrap_or(default);
        row.set_selected(levels.iter().position(|l| *l >= cur).unwrap_or(levels.len() - 1) as u32);
        row.connect_selected_notify(move |r| {
            let v = levels[(r.selected() as usize).min(levels.len() - 1)];
            kconfig::spawn(move || {
                kconfig::write(RC, &[GROUP], key, &v.to_string());
                reload_powerdevil();
            });
        });
        row
    };
    g.add(&level_row("Low battery warning at", "Show a warning at this charge", "BatteryLowLevel", &LOW_LEVELS, 10));
    g.add(&level_row("Critical level at", "Act at this charge", "BatteryCriticalLevel", &CRITICAL_LEVELS, 5));

    let hib = crate::backend::hibernate::can_hibernate();
    let critical: Vec<(&'static str, &'static str)> = CRITICAL_ACTIONS_ALL.iter().copied().filter(|(_, v)| hib || *v != "2").collect();
    let act = adw::ComboRow::new();
    act.set_title("At the critical level");
    act.set_model(Some(&gtk4::StringList::new(&critical.iter().map(|a| a.0).collect::<Vec<_>>())));
    let cur = kconfig::read(RC, &[GROUP], "BatteryCriticalAction").unwrap_or_default();
    act.set_selected(critical.iter().position(|(_, v)| *v == cur).unwrap_or(0) as u32);
    act.connect_selected_notify(move |r| {
        let v = critical[(r.selected() as usize).min(critical.len() - 1)].1;
        kconfig::spawn(move || {
            kconfig::write(RC, &[GROUP], "BatteryCriticalAction", v);
            reload_powerdevil();
        });
    });
    g.add(&act);
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
                content.append(&battery_levels_group());
            } else {
                content.append(&profile_group("Screen and sleep", "AC", false));
            }
            content.append(&lock_group());
        },
    );

    scroll.set_child(Some(&root));
    scroll.upcast()
}
