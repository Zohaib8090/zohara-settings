//! Notification settings for Plasma: do-not-disturb, popup behaviour, and
//! per-app control. Everything lives in `plasmanotifyrc`, which Plasma's
//! notification server watches, so changes take effect without a restart.

use crate::backend::kconfig;
use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;

const RC: &str = "plasmanotifyrc";
// "Until" far in the future is how Plasma itself represents open-ended DND. KConfig stores a date and time as
// `year,month,day,hour,minute,second.millisecond` (local time), not as an ISO date: Plasma ignores any other spelling,
// which is why this switch once did nothing.
const DND_FOREVER: &str = "2100,1,1,0,0,0";

/// (year, month, day, hour, minute, second) from KConfig's `2027,10,9,3,8,49.962`. None for anything else.
fn parse_until(v: &str) -> Option<[i64; 6]> {
    let mut out = [0i64; 6];
    let mut parts = v.trim().split(',');
    for slot in out.iter_mut() {
        let part = parts.next()?.trim();
        *slot = part.split('.').next()?.parse().ok()?; // seconds may carry milliseconds
    }
    parts.next().is_none().then_some(out)
}

/// Whether a stored `Until` is still in the future at `now` (same field order, local time).
fn dnd_until_active(value: &str, now: [i64; 6]) -> bool {
    parse_until(value).is_some_and(|until| until > now)
}

fn dnd_active() -> bool {
    let Some(v) = kconfig::read(RC, &["DoNotDisturb"], "Until") else { return false };
    let Ok(t) = glib::DateTime::now_local() else { return false };
    let now = [t.year() as i64, t.month() as i64, t.day_of_month() as i64, t.hour() as i64, t.minute() as i64, t.second() as i64];
    dnd_until_active(&v, now)
}

fn bool_key(groups: &[&str], key: &str, default: bool) -> bool {
    match kconfig::read(RC, groups, key).as_deref() {
        Some("true") => true,
        Some("false") => false,
        _ => default,
    }
}

fn general_group() -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("General");

    let dnd = adw::SwitchRow::new();
    dnd.set_title("Do not disturb");
    dnd.set_subtitle("Stop notification pop-ups until you turn this off");
    dnd.add_prefix(&gtk4::Image::from_icon_name("notifications-disabled-symbolic"));
    dnd.set_active(dnd_active());
    dnd.connect_active_notify(|r| {
        let on = r.is_active();
        kconfig::spawn(move || {
            if on {
                kconfig::write_notify(RC, &["DoNotDisturb"], "Until", DND_FOREVER);
            } else {
                kconfig::delete_notify(RC, &["DoNotDisturb"], "Until");
            }
        });
    });
    g.add(&dnd);

    let critical = adw::SwitchRow::new();
    critical.set_title("Allow critical notifications in do not disturb");
    critical.set_subtitle("Urgent alerts such as low battery still appear");
    critical.set_active(bool_key(&["Notifications"], "CriticalInDndMode", true));
    critical.connect_active_notify(|r| {
        let v = if r.is_active() { "true" } else { "false" };
        kconfig::spawn(move || kconfig::write(RC, &["Notifications"], "CriticalInDndMode", v));
    });
    g.add(&critical);

    let timeout = adw::SpinRow::with_range(1.0, 60.0, 1.0);
    timeout.set_title("Pop-up duration");
    timeout.set_subtitle("Seconds before a notification disappears");
    let secs = kconfig::read(RC, &["Notifications"], "PopupTimeout")
        .and_then(|v| v.parse::<f64>().ok())
        .map(|ms| (ms / 1000.0).clamp(1.0, 60.0))
        .unwrap_or(5.0);
    timeout.set_value(secs);
    timeout.connect_value_notify(|r| {
        let ms = ((r.value() as i64) * 1000).to_string();
        kconfig::spawn(move || kconfig::write(RC, &["Notifications"], "PopupTimeout", &ms));
    });
    g.add(&timeout);

    let history = adw::SwitchRow::new();
    history.set_title("Keep low-priority notifications in history");
    history.set_active(bool_key(&["Notifications"], "LowPriorityHistory", true));
    history.connect_active_notify(|r| {
        let v = if r.is_active() { "true" } else { "false" };
        kconfig::spawn(move || kconfig::write(RC, &["Notifications"], "LowPriorityHistory", v));
    });
    g.add(&history);

    g
}

/// Application ids Plasma has seen notify: the `[Applications][<id>]` groups.
fn known_apps() -> Vec<String> {
    let home = std::env::var("HOME").unwrap_or_default();
    let cfg = std::env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| format!("{home}/.config"));
    let text = std::fs::read_to_string(format!("{cfg}/{RC}")).unwrap_or_default();
    let mut apps: Vec<String> = text
        .lines()
        .filter_map(|l| l.strip_prefix("[Applications][")?.strip_suffix(']'))
        .map(str::to_string)
        .collect();
    apps.sort();
    apps.dedup();
    apps
}

fn app_display_name(id: &str) -> String {
    for dir in ["/usr/share/applications", "/usr/local/share/applications"] {
        if let Ok(text) = std::fs::read_to_string(format!("{dir}/{id}.desktop")) {
            if let Some(name) = text.lines().find_map(|l| l.strip_prefix("Name=")) {
                return name.to_string();
            }
        }
    }
    id.to_string()
}

fn apps_group() -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("Applications");
    g.set_description(Some("Apps appear here after they have sent a notification"));

    let apps = known_apps();
    if apps.is_empty() {
        let row = adw::ActionRow::new();
        row.set_title("No applications yet");
        row.set_activatable(false);
        g.add(&row);
        return g;
    }

    for id in apps {
        let row = adw::ExpanderRow::new();
        row.set_title(&glib::markup_escape_text(&app_display_name(&id)));
        row.set_subtitle(&glib::markup_escape_text(&id));

        for (key, title) in [
            ("ShowPopups", "Show pop-ups"),
            ("ShowInHistory", "Show in notification history"),
            ("ShowBadges", "Show badges on the app icon"),
        ] {
            let sw = adw::SwitchRow::new();
            sw.set_title(title);
            sw.set_active(bool_key(&["Applications", &id], key, true));
            let id2 = id.clone();
            sw.connect_active_notify(move |r| {
                let v = if r.is_active() { "true" } else { "false" };
                let id3 = id2.clone();
                kconfig::spawn(move || kconfig::write(RC, &["Applications", &id3], key, v));
            });
            row.add_row(&sw);
        }
        g.add(&row);
    }
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
            .label("Notifications")
            .halign(gtk4::Align::Start)
            .css_classes(vec!["win11-page-title".to_string()])
            .build(),
    );

    if !kconfig::available() {
        let s = adw::StatusPage::builder()
            .icon_name("dialog-information-symbolic")
            .title("Notification settings unavailable")
            .description("These settings need Plasma's configuration tools (kwriteconfig6).")
            .build();
        root.append(&s);
    } else {
        root.append(&general_group());
        root.append(&apps_group());
    }

    scroll.set_child(Some(&root));
    scroll.upcast()
}

#[cfg(test)]
mod dnd_tests {
    use super::*;

    const NOW: [i64; 6] = [2026, 10, 9, 3, 30, 0];

    #[test]
    fn plasmas_own_value_is_read_back() {
        assert_eq!(parse_until("2027,10,9,3,8,49.962"), Some([2027, 10, 9, 3, 8, 49]));
        assert!(dnd_until_active("2027,10,9,3,8,49.962", NOW));
        assert!(dnd_until_active(DND_FOREVER, NOW));
    }

    #[test]
    fn a_time_in_the_past_is_not_active() {
        assert!(!dnd_until_active("2026,10,9,3,29,59.5", NOW));
        assert!(!dnd_until_active("2025,12,31,23,59,59", NOW));
        assert!(dnd_until_active("2026,10,9,3,30,1", NOW));
    }

    #[test]
    fn other_spellings_are_not_active() {
        // What an older Settings wrote: Plasma never understood it.
        assert!(!dnd_until_active("2100-01-01T00:00:00", NOW));
        assert!(!dnd_until_active("", NOW));
        assert!(!dnd_until_active("2100,1,1", NOW));
        assert!(!dnd_until_active("2100,1,1,0,0,0,9", NOW));
    }

    fn plasma_inhibited() -> bool {
        let o = std::process::Command::new("busctl")
            .args(["--user", "get-property", "org.freedesktop.Notifications", "/org/freedesktop/Notifications", "org.freedesktop.Notifications", "Inhibited"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&o.stdout).contains("true")
    }

    /// Needs a running Plasma session. `cargo test live_dnd -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_dnd_switch_really_turns_plasma_do_not_disturb_on_and_off() {
        kconfig::delete_notify(RC, &["DoNotDisturb"], "Until");
        std::thread::sleep(std::time::Duration::from_secs(2));
        assert!(!plasma_inhibited(), "starts off");
        assert!(!dnd_active());

        kconfig::write_notify(RC, &["DoNotDisturb"], "Until", DND_FOREVER);
        std::thread::sleep(std::time::Duration::from_secs(2));
        let (plasma_on, page_on) = (plasma_inhibited(), dnd_active());

        kconfig::delete_notify(RC, &["DoNotDisturb"], "Until");
        std::thread::sleep(std::time::Duration::from_secs(2));
        let (plasma_off, page_off) = (plasma_inhibited(), dnd_active());

        assert!(plasma_on && page_on, "turned on: plasma={plasma_on} page={page_on}");
        assert!(!plasma_off && !page_off, "turned off: plasma={plasma_off} page={page_off}");
    }
}
