//! Privacy & security: firewall (ufw), device access (camera, microphone,
//! location) with tray indicators and a live "in use" view, file search
//! indexing (Baloo) and Plasma's usage feedback.
//! Every switch starts from the system's real state and snaps back if the
//! change is cancelled or fails.

use crate::backend::kconfig;
use crate::backend::worker::in_background;
use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use std::cell::Cell;
use std::process::Command;
use std::rc::Rc;

const CAMERA_BLOCK: &str = "/etc/modprobe.d/zohara-camera-off.conf";

fn run_ok(cmd: &str, args: &[&str]) -> bool {
    Command::new(cmd).args(args).status().map(|s| s.success()).unwrap_or(false)
}

/// A switch whose change runs `apply(on)` off the UI thread; if that fails
/// (e.g. the admin prompt was cancelled) the switch is reset from `read()`.
fn guarded_switch(
    title: &str,
    subtitle: &str,
    icon: &str,
    read: fn() -> bool,
    apply: fn(bool) -> bool,
) -> adw::SwitchRow {
    let row = adw::SwitchRow::new();
    row.set_title(title);
    row.set_subtitle(subtitle);
    row.add_prefix(&gtk4::Image::from_icon_name(icon));
    row.set_active(read());
    let reverting = Rc::new(Cell::new(false));
    row.connect_active_notify(move |r| {
        if reverting.get() {
            return;
        }
        let on = r.is_active();
        r.set_sensitive(false);
        let (r, reverting) = (r.clone(), reverting.clone());
        in_background(
            move || apply(on),
            move |ok| {
                if !ok {
                    reverting.set(true);
                    r.set_active(read());
                    reverting.set(false);
                }
                r.set_sensitive(true);
            },
        );
    });
    row
}

// ── Firewall ───────────────────────────────────────────────────────────────

fn firewall_on() -> bool {
    std::fs::read_to_string("/etc/ufw/ufw.conf")
        .map(|s| s.lines().any(|l| l.trim() == "ENABLED=yes"))
        .unwrap_or(false)
}

fn set_firewall(on: bool) -> bool {
    let script = if on {
        "systemctl enable --now ufw.service && ufw --force enable"
    } else {
        "ufw disable"
    };
    run_ok("pkexec", &["sh", "-c", script])
}

// ── Camera: blocking the UVC driver covers built-in and USB webcams ────────

fn camera_on() -> bool {
    !std::path::Path::new(CAMERA_BLOCK).exists()
}

fn set_camera(on: bool) -> bool {
    let script = if on {
        format!("rm -f {CAMERA_BLOCK} && modprobe uvcvideo")
    } else {
        // Removal fails while an app has the camera open; the blacklist still
        // applies from the next boot, so report success either way.
        format!("echo 'blacklist uvcvideo' > {CAMERA_BLOCK} && (modprobe -r uvcvideo || true)")
    };
    run_ok("pkexec", &["sh", "-c", &script])
}

// ── Microphone: mute state of every real input (not sink monitors) ─────────

fn mic_sources() -> Vec<(String, bool)> {
    Command::new("pactl")
        .args(["-f", "json", "list", "sources"])
        .output()
        .ok()
        .and_then(|o| serde_json::from_slice::<Vec<serde_json::Value>>(&o.stdout).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|s| s["monitor_of_sink"].is_null())
        .filter_map(|s| Some((s["name"].as_str()?.to_string(), s["mute"].as_bool().unwrap_or(false))))
        .collect()
}

fn mic_on() -> bool {
    let sources = mic_sources();
    !sources.is_empty() && sources.iter().any(|(_, muted)| !muted)
}

fn set_mic(on: bool) -> bool {
    let mute = if on { "0" } else { "1" };
    let sources = mic_sources();
    !sources.is_empty() && sources.iter().all(|(name, _)| run_ok("pactl", &["set-source-mute", name, mute]))
}

// ── Location: the geoclue service apps and the location portal ask ─────────

fn location_on() -> bool {
    // `is-enabled` exits non-zero for a masked unit, so read its output regardless of status.
    let state = Command::new("systemctl")
        .args(["is-enabled", "geoclue.service"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    geoclue_installed() && state != "masked"
}

fn set_location(on: bool) -> bool {
    let script = if on { "systemctl unmask geoclue.service" } else { "systemctl mask --now geoclue.service" };
    run_ok("pkexec", &["sh", "-c", script])
}

fn geoclue_installed() -> bool {
    std::path::Path::new("/usr/share/dbus-1/system-services/org.freedesktop.GeoClue2.service").exists()
}

// ── Search (Baloo) ─────────────────────────────────────────────────────────

fn indexing_on() -> bool {
    kconfig::read("baloofilerc", &["Basic Settings"], "Indexing-Enabled").as_deref() != Some("false")
}

fn set_indexing(on: bool) -> bool {
    run_ok("balooctl6", &[if on { "enable" } else { "disable" }])
}

fn contents_on() -> bool {
    kconfig::read("baloofilerc", &["General"], "only basic indexing").as_deref() != Some("true")
}

fn set_contents(on: bool) -> bool {
    kconfig::write("baloofilerc", &["General"], "only basic indexing", if on { "false" } else { "true" });
    // Baloo only picks this up on restart; harmless if indexing is off.
    if indexing_on() {
        let _ = Command::new("balooctl6").arg("restart").status();
    }
    true
}

// ── Page ───────────────────────────────────────────────────────────────────

fn security_group() -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("Security");
    let fw = guarded_switch(
        "Firewall",
        "Block unsolicited incoming connections. Outgoing connections are not affected",
        "security-high-symbolic",
        firewall_on,
        set_firewall,
    );
    if !std::path::Path::new("/usr/bin/ufw").exists() {
        fw.set_sensitive(false);
        fw.set_subtitle("The ufw firewall is not installed");
    }
    g.add(&fw);
    g
}

// ── Firewall rules ─────────────────────────────────────────────────────────

/// A port or range like `8080` or `8000:8100`.
fn valid_port(p: &str) -> bool {
    let ok = |s: &str| !s.is_empty() && s.len() <= 5 && s.bytes().all(|b| b.is_ascii_digit()) && s.parse::<u32>().map(|n| (1..=65535).contains(&n)).unwrap_or(false);
    match p.split_once(':') {
        Some((a, b)) => ok(a) && ok(b) && a.parse::<u32>().unwrap_or(0) < b.parse::<u32>().unwrap_or(0),
        None => ok(p),
    }
}

/// Numbered rules from `ufw status numbered`: (number, description).
fn parse_rules(text: &str) -> Vec<(u32, String)> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            let rest = l.strip_prefix('[')?;
            let (num, desc) = rest.split_once(']')?;
            Some((num.trim().parse().ok()?, desc.split_whitespace().collect::<Vec<_>>().join(" ")))
        })
        .collect()
}

/// Runs ufw as administrator. `Ok(None)` means the password prompt was dismissed.
fn ufw(args: &[&str]) -> Result<Option<String>, String> {
    let o = Command::new("pkexec").arg("ufw").args(args).output().map_err(|e| format!("pkexec: {e}"))?;
    match o.status.code() {
        Some(0) => Ok(Some(String::from_utf8_lossy(&o.stdout).into_owned())),
        Some(126) | Some(127) => Ok(None),
        _ => {
            let e = String::from_utf8_lossy(&o.stderr).trim().to_string();
            Err(if e.is_empty() { "ufw failed".into() } else { e })
        }
    }
}

type Reload = Rc<std::cell::RefCell<Option<Rc<dyn Fn()>>>>;

fn firewall_rules_group() -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("Firewall rules");
    g.set_description(Some("Allow or block traffic on a port. Viewing and changing rules asks for your password."));
    if !std::path::Path::new("/usr/bin/ufw").exists() {
        return g;
    }

    let list = adw::ExpanderRow::new();
    list.set_title("Current rules");
    list.set_subtitle("Load them to see what's allowed or blocked");
    list.add_prefix(&gtk4::Image::from_icon_name("view-list-symbolic"));
    g.add(&list);
    let shown: Rc<std::cell::RefCell<Vec<gtk4::Widget>>> = Default::default();

    // Deleting a rule renumbers the rest, so the list is reloaded after each change.
    let load: Reload = Default::default();
    let reload: Rc<dyn Fn()> = {
        let (list, shown, load) = (list.clone(), shown.clone(), load.clone());
        Rc::new(move || {
            let (list, shown, load) = (list.clone(), shown.clone(), load.clone());
            in_background(
                || ufw(&["status", "numbered"]),
                move |r| {
                    for w in shown.borrow_mut().drain(..) {
                        list.remove(&w);
                    }
                    let note = |text: &str| {
                        let row = adw::ActionRow::new();
                        row.set_title(text);
                        list.add_row(&row);
                        shown.borrow_mut().push(row.upcast());
                    };
                    match r {
                        Ok(Some(out)) if out.contains("inactive") => note("The firewall is off. Turn it on above to use rules."),
                        Ok(Some(out)) => {
                            let rules = parse_rules(&out);
                            if rules.is_empty() {
                                note("No rules yet");
                            }
                            for (n, desc) in rules {
                                let row = adw::ActionRow::new();
                                row.set_title(&glib::markup_escape_text(&desc));
                                let del = gtk4::Button::from_icon_name("user-trash-symbolic");
                                del.add_css_class("flat");
                                del.set_valign(gtk4::Align::Center);
                                del.set_tooltip_text(Some("Delete this rule"));
                                let load2 = load.clone();
                                del.connect_clicked(move |b| {
                                    b.set_sensitive(false);
                                    let load3 = load2.clone();
                                    in_background(
                                        move || ufw(&["--force", "delete", &n.to_string()]),
                                        move |_| {
                                            let f = load3.borrow().clone();
                                            if let Some(f) = f {
                                                f();
                                            }
                                        },
                                    );
                                });
                                row.add_suffix(&del);
                                list.add_row(&row);
                                shown.borrow_mut().push(row.upcast());
                            }
                            list.set_subtitle("");
                            list.set_expanded(true);
                        }
                        Ok(None) => list.set_subtitle("Password prompt cancelled"),
                        Err(e) => list.set_subtitle(&glib::markup_escape_text(&e)),
                    }
                },
            );
        })
    };
    *load.borrow_mut() = Some(reload.clone());
    let refresh = gtk4::Button::with_label("Load rules");
    refresh.set_valign(gtk4::Align::Center);
    let reload2 = reload.clone();
    refresh.connect_clicked(move |_| reload2());
    list.add_suffix(&refresh);

    // Add a rule.
    let port = adw::EntryRow::new();
    port.set_title("Port or range, like 8080 or 8000:8100");
    let proto = adw::ComboRow::new();
    proto.set_title("Protocol");
    proto.set_model(Some(&gtk4::StringList::new(&["TCP and UDP", "TCP", "UDP"])));
    let action = adw::ComboRow::new();
    action.set_title("Action");
    action.set_model(Some(&gtk4::StringList::new(&["Allow", "Block"])));
    let add = gtk4::Button::with_label("Add rule");
    add.add_css_class("suggested-action");
    add.set_valign(gtk4::Align::Center);
    port.add_suffix(&add);
    g.add(&port);
    g.add(&proto);
    g.add(&action);

    let (port2, proto2, action2) = (port.clone(), proto.clone(), action.clone());
    add.connect_clicked(move |b| {
        let p = port2.text().trim().to_string();
        if !valid_port(&p) {
            port2.add_css_class("error");
            return;
        }
        port2.remove_css_class("error");
        let target = match proto2.selected() {
            1 => format!("{p}/tcp"),
            2 => format!("{p}/udp"),
            _ => p,
        };
        let verb = if action2.selected() == 1 { "deny" } else { "allow" };
        b.set_sensitive(false);
        let (b2, port3, reload3) = (b.clone(), port2.clone(), reload.clone());
        in_background(
            move || ufw(&[verb, &target]),
            move |r| {
                b2.set_sensitive(true);
                match r {
                    Ok(Some(_)) => {
                        port3.set_text("");
                        reload3();
                    }
                    Ok(None) => {}
                    Err(e) => {
                        let d = adw::AlertDialog::new(Some("Couldn't add the rule"), Some(&e));
                        d.add_response("ok", "OK");
                        d.present(Some(&b2));
                    }
                }
            },
        );
    });
    g
}

fn devices_group() -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("Device access");
    g.set_description(Some("Turning one of these off blocks it for every app."));

    g.add(&guarded_switch(
        "Camera",
        "Built-in and USB webcams",
        "camera-web-symbolic",
        camera_on,
        set_camera,
    ));
    let mic = guarded_switch("Microphone", "All microphones and audio inputs", "audio-input-microphone-symbolic", mic_on, set_mic);
    if mic_sources().is_empty() {
        mic.set_sensitive(false);
        mic.set_subtitle("No microphone detected");
    }
    g.add(&mic);

    let loc = guarded_switch(
        "Location services",
        "Lets apps estimate where you are from nearby Wi-Fi networks",
        "find-location-symbolic",
        location_on,
        set_location,
    );
    if !geoclue_installed() {
        loc.set_sensitive(false);
        loc.set_subtitle("Location services are not installed");
    }
    g.add(&loc);
    g
}

// ── Access indicators ──────────────────────────────────────────────────────

fn use_text(apps: &[String], none: &str) -> String {
    if apps.is_empty() {
        none.to_string()
    } else {
        format!("In use by {}", apps.join(", "))
    }
}

fn device_label(d: &str) -> &'static str {
    match d {
        "microphone" => "Microphone",
        "camera" => "Camera",
        _ => "Location",
    }
}

fn indicators_group() -> adw::PreferencesGroup {
    use crate::backend::privacy_indicator as pi;
    let g = adw::PreferencesGroup::new();
    g.set_title("Access indicators");
    g.set_description(Some("Shows an icon in the system tray while an app is using your camera or location (Plasma shows the microphone one itself). Every use is also kept in Recent activity below."));
    let cfg = pi::load_config();

    let master = adw::SwitchRow::new();
    master.set_title("Show indicators");
    master.add_prefix(&gtk4::Image::from_icon_name("view-reveal-symbolic"));
    master.set_active(cfg.enabled);
    g.add(&master);

    let mut switches: Vec<adw::SwitchRow> = Vec::new();
    for (key, title, sub, icon, on) in [
        ("Microphone", "Microphone", "Plasma already shows its own microphone icon. Turn this on to add a Zohara one too", "audio-input-microphone-symbolic", cfg.microphone),
        ("Camera", "Camera", "Show an icon while an app is using the camera", "camera-web-symbolic", cfg.camera),
        ("Location", "Location", "Show an icon while an app is using your location", "find-location-symbolic", cfg.location),
    ] {
        let r = adw::SwitchRow::new();
        r.set_title(title);
        r.set_subtitle(sub);
        r.add_prefix(&gtk4::Image::from_icon_name(icon));
        r.set_active(on);
        r.connect_active_notify(move |r| {
            let v = r.is_active();
            kconfig::spawn(move || pi::save_config(key, v));
        });
        master.bind_property("active", &r, "sensitive").sync_create().build();
        g.add(&r);
        switches.push(r);
    }

    // The tray program starts at sign-in; offer to start it if it isn't running.
    let warn = adw::ActionRow::new();
    warn.set_title("The indicator isn't running");
    warn.set_subtitle("It starts automatically when you sign in");
    warn.add_prefix(&gtk4::Image::from_icon_name("dialog-warning-symbolic"));
    let start = gtk4::Button::with_label("Start now");
    start.set_valign(gtk4::Align::Center);
    warn.add_suffix(&start);
    warn.set_visible(false);
    g.add(&warn);
    {
        let warn2 = warn.clone();
        start.connect_clicked(move |_| {
            pi::start_daemon();
            warn2.set_visible(false);
        });
    }
    {
        let warn2 = warn.clone();
        master.connect_active_notify(move |r| {
            let v = r.is_active();
            kconfig::spawn(move || pi::save_config("Enabled", v));
            if v && !pi::daemon_running() {
                pi::start_daemon();
            }
            warn2.set_visible(false);
        });
    }
    let (warn3, master3) = (warn.clone(), master.clone());
    in_background(pi::daemon_running, move |running| warn3.set_visible(!running && master3.is_active()));
    g
}

/// Live "in use right now" rows and the recent activity list.
fn activity_group() -> adw::PreferencesGroup {
    use crate::backend::privacy_indicator as pi;
    let g = adw::PreferencesGroup::new();
    g.set_title("Right now");

    let mk = |title: &str, icon: &str| {
        let r = adw::ActionRow::new();
        r.set_title(title);
        r.set_subtitle("Checking…");
        r.add_prefix(&gtk4::Image::from_icon_name(icon));
        g.add(&r);
        r
    };
    let mic = mk("Microphone", "audio-input-microphone-symbolic");
    let cam = mk("Camera", "camera-web-symbolic");
    let loc = mk("Location", "find-location-symbolic");

    let history = adw::ExpanderRow::new();
    history.set_title("Recent activity");
    history.set_subtitle("When apps started and stopped using these");
    history.add_prefix(&gtk4::Image::from_icon_name("document-open-recent-symbolic"));
    g.add(&history);
    let history_rows: std::rc::Rc<std::cell::RefCell<Vec<gtk4::Widget>>> = Default::default();

    let fill_history = {
        let (history, rows) = (history.clone(), history_rows.clone());
        move || {
            for w in rows.borrow_mut().drain(..) {
                history.remove(&w);
            }
            let entries = pi::recent_activity(15);
            if entries.is_empty() {
                let r = adw::ActionRow::new();
                r.set_title("Nothing yet");
                history.add_row(&r);
                rows.borrow_mut().push(r.upcast());
                return;
            }
            for e in entries {
                let r = adw::ActionRow::new();
                r.set_title(&glib::markup_escape_text(&format!("{} {}", e.app, if e.started { "started using" } else { "stopped using" })));
                r.set_subtitle(&glib::markup_escape_text(&format!("{} · {}", device_label(&e.device), e.time)));
                history.add_row(&r);
                rows.borrow_mut().push(r.upcast());
            }
            let clear = adw::ActionRow::new();
            clear.set_title("Clear history");
            clear.set_activatable(true);
            clear.add_prefix(&gtk4::Image::from_icon_name("edit-clear-all-symbolic"));
            let rows2 = rows.clone();
            let history2 = history.clone();
            clear.connect_activated(move |_| {
                pi::clear_activity();
                for w in rows2.borrow_mut().drain(..) {
                    history2.remove(&w);
                }
            });
            history.add_row(&clear);
            rows.borrow_mut().push(clear.upcast());
        }
    };
    fill_history();

    let refresh = {
        let (mic, cam, loc) = (mic.clone(), cam.clone(), loc.clone());
        move || {
            let (mic, cam, loc) = (mic.clone(), cam.clone(), loc.clone());
            in_background(pi::scan_now, move |s| {
                mic.set_subtitle(&glib::markup_escape_text(&use_text(&s.microphone, "Not in use")));
                cam.set_subtitle(&glib::markup_escape_text(&use_text(&s.camera, "Not in use")));
                loc.set_subtitle(if s.location { "In use by an app" } else { "Not in use" });
            });
        }
    };
    refresh();
    // Keep it live while this page is on screen.
    let weak = g.downgrade();
    glib::timeout_add_local(std::time::Duration::from_secs(3), move || {
        let Some(g) = weak.upgrade() else { return glib::ControlFlow::Break };
        if g.is_mapped() {
            refresh();
            fill_history();
        }
        glib::ControlFlow::Continue
    });
    g
}

fn search_group() -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("Search");
    g.set_description(Some("File indexing makes files searchable from the launcher and file manager."));
    if !std::path::Path::new("/usr/bin/balooctl6").exists() {
        let row = adw::ActionRow::new();
        row.set_title("File indexing is not installed");
        g.add(&row);
        return g;
    }
    let contents = guarded_switch(
        "Index file contents",
        "Also search inside documents, not just file names",
        "text-x-generic-symbolic",
        contents_on,
        set_contents,
    );
    let indexing = guarded_switch("File indexing", "Keep an index of your files", "system-search-symbolic", indexing_on, set_indexing);
    contents.set_sensitive(indexing.is_active());
    {
        let contents = contents.clone();
        indexing.connect_active_notify(move |r| contents.set_sensitive(r.is_active()));
    }
    g.add(&indexing);
    g.add(&contents);
    g
}

/// Plasma's usage feedback (KUserFeedback telemetry modes). Off unless the user opts in.
fn feedback_group() -> adw::PreferencesGroup {
    const LEVELS: [(&str, &str); 5] = [
        ("Off", "0"),
        ("Basic system information", "16"),
        ("Basic usage statistics", "32"),
        ("Detailed system information", "48"),
        ("Detailed usage statistics", "64"),
    ];
    let g = adw::PreferencesGroup::new();
    g.set_title("Diagnostics & feedback");
    let row = adw::ComboRow::new();
    row.set_title("Share usage data with KDE");
    row.set_subtitle("Anonymous statistics that help improve Plasma. Takes effect after you sign in again");
    row.add_prefix(&gtk4::Image::from_icon_name("utilities-system-monitor-symbolic"));
    let labels: Vec<&str> = LEVELS.iter().map(|(l, _)| *l).collect();
    row.set_model(Some(&gtk4::StringList::new(&labels)));
    let current = kconfig::read("PlasmaUserFeedback", &["Global"], "FeedbackLevel").unwrap_or_else(|| "0".into());
    row.set_selected(LEVELS.iter().position(|(_, v)| *v == current).unwrap_or(0) as u32);
    row.connect_selected_notify(|r| {
        if let Some((_, v)) = LEVELS.get(r.selected() as usize) {
            let v = v.to_string();
            kconfig::spawn(move || kconfig::write("PlasmaUserFeedback", &["Global"], "FeedbackLevel", &v));
        }
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
            .label("Privacy & security")
            .halign(gtk4::Align::Start)
            .css_classes(vec!["win11-page-title".to_string()])
            .build(),
    );
    root.append(&security_group());
    root.append(&firewall_rules_group());
    root.append(&devices_group());
    root.append(&indicators_group());
    root.append(&activity_group());
    root.append(&search_group());
    if kconfig::available() {
        root.append(&feedback_group());
    }

    scroll.set_child(Some(&root));
    scroll.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ports() {
        for ok in ["80", "65535", "8000:8100"] {
            assert!(valid_port(ok), "{ok}");
        }
        for bad in ["", "0", "65536", "abc", "80:", "9000:8000", "8000:8000", "1 2", "-1", "80/tcp"] {
            assert!(!valid_port(bad), "{bad}");
        }
    }

    #[test]
    fn rules() {
        let out = "Status: active\n\n     To                         Action      From\n     --                         ------      ----\n[ 1] 22/tcp                     ALLOW IN    Anywhere\n[ 2] 8080                       DENY IN     Anywhere\n";
        assert_eq!(
            parse_rules(out),
            vec![(1, "22/tcp ALLOW IN Anywhere".to_string()), (2, "8080 DENY IN Anywhere".to_string())]
        );
    }
}
