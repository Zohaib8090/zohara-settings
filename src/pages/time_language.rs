//! Time & language: live clock, time zone and automatic time via systemd's
//! timedated (`timedatectl`), and regional formats via Plasma's
//! `plasma-localerc`. Every control starts from the system's real state.

use crate::backend::kconfig;
use crate::backend::worker::in_background;
use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use std::process::Command;

fn output(cmd: &str, args: &[&str]) -> Option<String> {
    let o = Command::new(cmd).args(args).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

fn timedate(prop: &str) -> Option<String> {
    output("timedatectl", &["show", "-p", prop, "--value"])
}

/// A DropDown over plain strings with working type-to-search.
fn searchable_dropdown(items: &[String]) -> gtk4::DropDown {
    let refs: Vec<&str> = items.iter().map(String::as_str).collect();
    let dd = gtk4::DropDown::from_strings(&refs);
    dd.set_expression(Some(gtk4::PropertyExpression::new(
        gtk4::StringObject::static_type(),
        None::<gtk4::Expression>,
        "string",
    )));
    dd.set_enable_search(true);
    dd.set_search_match_mode(gtk4::StringFilterMatchMode::Substring);
    dd.set_valign(gtk4::Align::Center);
    dd
}

fn show_error(widget: &impl IsA<gtk4::Widget>, title: &str, body: &str) {
    let d = adw::AlertDialog::new(Some(title), Some(body));
    d.add_response("ok", "OK");
    d.present(Some(widget));
}

fn clock_group() -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    let row = adw::ActionRow::new();
    row.add_css_class("property");
    row.set_activatable(false);
    let update = {
        let row = row.clone();
        move || {
            if let Ok(now) = glib::DateTime::now_local() {
                row.set_title(&now.format("%A, %e %B %Y").map(|s| s.to_string()).unwrap_or_default());
                row.set_subtitle(&now.format("%X").map(|s| s.to_string()).unwrap_or_default());
            }
        }
    };
    update();
    let weak = row.downgrade();
    glib::timeout_add_seconds_local(1, move || {
        if weak.upgrade().is_none() {
            return glib::ControlFlow::Break;
        }
        update();
        glib::ControlFlow::Continue
    });
    row.add_prefix(&gtk4::Image::from_icon_name("preferences-system-time-symbolic"));
    g.add(&row);
    g
}

fn time_group() -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("Date & time");

    let ntp = adw::SwitchRow::new();
    ntp.set_title("Set time automatically");
    ntp.set_subtitle("Keep the clock in sync with internet time servers");
    ntp.set_active(timedate("NTP").as_deref() == Some("yes"));
    if timedate("CanNTP").as_deref() == Some("no") {
        ntp.set_sensitive(false);
        ntp.set_subtitle("No time synchronization service is installed");
    }
    let reverting = std::rc::Rc::new(std::cell::Cell::new(false));
    ntp.connect_active_notify(move |r| {
        if reverting.get() {
            return;
        }
        let want = r.is_active();
        let (r, reverting) = (r.clone(), reverting.clone());
        in_background(
            move || output("timedatectl", &["set-ntp", if want { "true" } else { "false" }]).is_some(),
            move |ok| {
                if !ok {
                    // Authorization was cancelled or failed: show the real state again
                    // without re-triggering this handler.
                    reverting.set(true);
                    r.set_active(timedate("NTP").as_deref() == Some("yes"));
                    reverting.set(false);
                }
            },
        );
    });
    g.add(&ntp);

    let zones: Vec<String> = output("timedatectl", &["list-timezones"])
        .map(|s| s.lines().map(str::to_string).collect())
        .unwrap_or_default();
    let tz_row = adw::ActionRow::new();
    tz_row.set_title("Time zone");
    tz_row.set_activatable(false);
    if zones.is_empty() {
        tz_row.set_subtitle(&timedate("Timezone").unwrap_or_else(|| "Unknown".into()));
    } else {
        let dd = searchable_dropdown(&zones);
        let current = timedate("Timezone").unwrap_or_default();
        if let Some(i) = zones.iter().position(|z| *z == current) {
            dd.set_selected(i as u32);
        }
        let zones2 = zones.clone();
        let current = std::rc::Rc::new(std::cell::RefCell::new(current));
        dd.connect_selected_notify(move |d| {
            let Some(tz) = zones2.get(d.selected() as usize).cloned() else { return };
            if *current.borrow() == tz {
                return;
            }
            let (d, zones3, current) = (d.clone(), zones2.clone(), current.clone());
            in_background(
                move || output("timedatectl", &["set-timezone", &tz]).map(|_| tz),
                move |res| match res {
                    Some(tz) => *current.borrow_mut() = tz,
                    None => {
                        let real = timedate("Timezone").unwrap_or_default();
                        if let Some(i) = zones3.iter().position(|z| *z == real) {
                            d.set_selected(i as u32);
                        }
                        show_error(&d, "Time zone not changed", "Changing the time zone needs administrator approval.");
                    }
                },
            );
        });
        tz_row.add_suffix(&dd);
    }
    g.add(&tz_row);
    g
}

/// UTF-8 locales installed on this system, e.g. "en_US.UTF-8".
fn installed_locales() -> Vec<String> {
    let mut v: Vec<String> = output("locale", &["-a"])
        .unwrap_or_default()
        .lines()
        .filter(|l| l.contains('_') && (l.ends_with(".utf8") || l.ends_with(".UTF-8")))
        .map(|l| l.replace(".utf8", ".UTF-8"))
        .collect();
    v.sort();
    v.dedup();
    v
}

fn region_group() -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("Language & region");

    let system_lang = output("localectl", &["status"])
        .and_then(|s| s.lines().find_map(|l| l.trim().strip_prefix("System Locale: LANG=").map(str::to_string)))
        .unwrap_or_else(|| std::env::var("LANG").unwrap_or_default());
    let lang = adw::ActionRow::new();
    lang.set_title("System language");
    lang.set_subtitle(&system_lang);
    lang.set_activatable(false);
    lang.add_prefix(&gtk4::Image::from_icon_name("preferences-desktop-locale-symbolic"));
    g.add(&lang);

    let locales = installed_locales();
    let fmt = adw::ActionRow::new();
    fmt.set_title("Regional format");
    fmt.set_activatable(false);
    if locales.len() < 2 || !kconfig::available() {
        fmt.set_subtitle("Only one language is installed on this system");
    } else {
        fmt.set_subtitle("Dates, times, numbers and currency. Applies after you sign out");
        let current = kconfig::read("plasma-localerc", &["Formats"], "LANG").unwrap_or(system_lang.clone());
        let dd = searchable_dropdown(&locales);
        if let Some(i) = locales.iter().position(|l| *l == current) {
            dd.set_selected(i as u32);
        }
        dd.connect_selected_notify(move |d| {
            if let Some(l) = locales.get(d.selected() as usize).cloned() {
                kconfig::spawn(move || kconfig::write("plasma-localerc", &["Formats"], "LANG", &l));
            }
        });
        fmt.add_suffix(&dd);
    }
    g.add(&fmt);
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
            .label("Time & language")
            .halign(gtk4::Align::Start)
            .css_classes(vec!["win11-page-title".to_string()])
            .build(),
    );
    root.append(&clock_group());
    root.append(&time_group());
    root.append(&region_group());

    scroll.set_child(Some(&root));
    scroll.upcast()
}
