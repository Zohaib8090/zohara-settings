//! Privacy & security: firewall (ufw), device access (camera, microphone,
//! location), file search indexing (Baloo) and Plasma's usage feedback.
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
    root.append(&devices_group());
    root.append(&search_group());
    if kconfig::available() {
        root.append(&feedback_group());
    }

    scroll.set_child(Some(&root));
    scroll.upcast()
}
