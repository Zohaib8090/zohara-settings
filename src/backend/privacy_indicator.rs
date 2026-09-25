//! Privacy indicators: a tray icon while the microphone, camera or location is
//! in use, so nothing listens or watches without you being able to see it.
//!
//! Runs as `zohara-settings --privacy-indicator`, started at sign-in by
//! `zohara-privacy-indicator.desktop`. It publishes three StatusNotifier items
//! (microphone, camera, location). An item is `Active` (shown in the system
//! tray, with the names of the apps in its tooltip) only while something is
//! using that device, and `Passive` (hidden) otherwise. Clicking one opens
//! Settings › Privacy & security.
//!
//! How each is detected:
//! - microphone: PulseAudio/PipeWire recording streams (`pactl`), ignoring
//!   paused streams, monitors of speakers and Plasma's own level meter;
//! - camera: processes holding `/dev/video*` open, plus PipeWire video
//!   capture streams (camera access through the portal);
//!
//! Plasma already shows its own microphone icon in the system tray, so the
//! Zohara microphone icon is off by default (Settings can turn it on). Usage of
//! all three is still recorded in the activity history.
//! - location: GeoClue's `InUse` property.
//!
//! Each time an app starts or stops using a device it is appended to
//! `~/.local/state/zohara/privacy-access.log`, shown under Recent activity.

use crate::backend::{diag, worker};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::ObjectPath;

const BUS_NAME: &str = "org.zohara.PrivacyIndicator";
const CONFIG: &str = "zoharaprivacyrc";
const POLL: Duration = Duration::from_secs(2);
const MAX_LOG_LINES: usize = 500;

// ── Settings ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Config {
    pub enabled: bool,
    pub microphone: bool,
    pub camera: bool,
    pub location: bool,
}

impl Default for Config {
    fn default() -> Self {
        // Plasma's Audio Volume applet already shows a microphone icon, so ours is opt-in.
        Config { enabled: true, microphone: false, camera: true, location: true }
    }
}

fn config_path() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config"));
    base.join(CONFIG)
}

/// Reads the `[Indicators]` group of the KConfig-style file (all default to on).
pub fn parse_config(text: &str) -> Config {
    let mut c = Config::default();
    let mut in_group = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_group = line == "[Indicators]";
        } else if in_group {
            if let Some((k, v)) = line.split_once('=') {
                let on = v.trim() != "false";
                match k.trim() {
                    "Enabled" => c.enabled = on,
                    "Microphone" => c.microphone = on,
                    "Camera" => c.camera = on,
                    "Location" => c.location = on,
                    _ => {}
                }
            }
        }
    }
    c
}

pub fn load_config() -> Config {
    fs::read_to_string(config_path()).map(|t| parse_config(&t)).unwrap_or_default()
}

/// Saves one key (`Enabled`, `Microphone`, `Camera`, `Location`).
pub fn save_config(key: &str, on: bool) {
    crate::backend::kconfig::write(CONFIG, &["Indicators"], key, if on { "true" } else { "false" });
}

// ── Detection ──────────────────────────────────────────────────────────────

#[derive(Clone, Default, PartialEq, Debug)]
pub struct Snapshot {
    pub microphone: Vec<String>,
    pub camera: Vec<String>,
    pub location: bool,
}

fn tidy(name: &str) -> String {
    let name = name.trim();
    let mut c = name.chars();
    match c.next() {
        Some(f) if f.is_lowercase() => f.to_uppercase().collect::<String>() + c.as_str(),
        _ => name.to_string(),
    }
}

fn dedupe(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v.dedup();
    v
}

fn pactl_json(what: &str) -> Option<Value> {
    let o = Command::new("pactl").args(["-f", "json", "list", what]).env("LC_ALL", "C").output().ok()?;
    o.status.success().then(|| serde_json::from_slice(&o.stdout).ok()).flatten()
}

/// Apps recording from a real microphone, from `pactl -f json` output.
pub fn parse_recording_apps(outputs: &Value, sources: &Value) -> Vec<String> {
    let monitors: BTreeSet<i64> = sources
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| {
            s["name"].as_str().is_some_and(|n| n.ends_with(".monitor"))
                || s["monitor_of_sink"].as_i64().is_some()
                || s["monitor_of_sink_name"].as_str().is_some_and(|n| n != "n/a")
        })
        .filter_map(|s| s["index"].as_i64())
        .collect();
    let mut apps = Vec::new();
    for o in outputs.as_array().into_iter().flatten() {
        if o["corked"].as_bool() == Some(true) {
            continue;
        }
        if o["source"].as_i64().is_some_and(|s| monitors.contains(&s)) {
            continue;
        }
        let p = &o["properties"];
        let media = p["media.name"].as_str().unwrap_or_default();
        let binary = p["application.process.binary"].as_str().unwrap_or_default();
        // Plasma's volume applet keeps a level meter on the microphone.
        if media.contains("Peak detect") || binary == "plasmashell" || p["application.id"].as_str() == Some("org.kde.plasma.pa") {
            continue;
        }
        let name = p["application.name"].as_str().filter(|n| !n.is_empty()).or(Some(binary).filter(|b| !b.is_empty())).unwrap_or("An app");
        apps.push(tidy(name));
    }
    dedupe(apps)
}

pub fn microphone_apps() -> Vec<String> {
    match (pactl_json("source-outputs"), pactl_json("sources")) {
        (Some(o), Some(s)) => parse_recording_apps(&o, &s),
        _ => Vec::new(),
    }
}

/// Apps with a running PipeWire video capture stream, from `pw-dump`.
pub fn parse_pipewire_video(dump: &Value) -> Vec<String> {
    let apps = dump
        .as_array()
        .into_iter()
        .flatten()
        .filter(|n| n["info"]["props"]["media.class"].as_str() == Some("Stream/Input/Video") && n["info"]["state"].as_str() == Some("running"))
        .map(|n| {
            let p = &n["info"]["props"];
            tidy(p["application.name"].as_str().or(p["node.name"].as_str()).filter(|s| !s.is_empty()).unwrap_or("An app"))
        })
        .collect();
    dedupe(apps)
}

fn pipewire_video_apps() -> Vec<String> {
    let Ok(o) = Command::new("pw-dump").output() else { return Vec::new() };
    serde_json::from_slice::<Value>(&o.stdout).map(|v| parse_pipewire_video(&v)).unwrap_or_default()
}

/// Programs (other than PipeWire and this one) that have a camera device open.
fn v4l2_holders() -> Vec<String> {
    // No camera device at all: nothing to scan for.
    let has_camera = fs::read_dir("/dev").map(|d| d.flatten().any(|e| e.file_name().to_string_lossy().starts_with("video"))).unwrap_or(false);
    if !has_camera {
        return Vec::new();
    }
    let me = std::process::id().to_string();
    let Ok(procs) = fs::read_dir("/proc") else { return Vec::new() };
    let mut out = Vec::new();
    for p in procs.flatten() {
        let name = p.file_name();
        let pid = name.to_string_lossy();
        if !pid.bytes().all(|b| b.is_ascii_digit()) || *pid == *me {
            continue;
        }
        let Ok(fds) = fs::read_dir(p.path().join("fd")) else { continue };
        let uses_camera = fds.flatten().any(|fd| fs::read_link(fd.path()).map(|t| t.to_string_lossy().starts_with("/dev/video")).unwrap_or(false));
        if !uses_camera {
            continue;
        }
        // `comm` is cut at 15 characters; the executable's name isn't.
        let exe = fs::read_link(p.path().join("exe")).ok().and_then(|e| e.file_name().map(|n| n.to_string_lossy().into_owned()));
        let comm = fs::read_to_string(p.path().join("comm")).unwrap_or_default();
        let name = exe.filter(|e| !e.is_empty() && !e.starts_with("ld-")).unwrap_or_else(|| comm.trim().to_string());
        // PipeWire opens cameras on behalf of apps (they're named from its
        // streams), and these system pieces aren't apps.
        if name.is_empty() || name == "wireplumber" || name.starts_with("pipewire") || name.starts_with("xdg-desktop-portal") || name == "systemd" {
            continue;
        }
        out.push(tidy(&name));
    }
    dedupe(out)
}

pub fn camera_apps() -> Vec<String> {
    let mut all = v4l2_holders();
    all.extend(pipewire_video_apps());
    dedupe(all)
}

/// GeoClue reports whether any app currently has a location client running.
pub async fn location_in_use() -> bool {
    async fn ask() -> zbus::Result<bool> {
        let conn = zbus::Connection::system().await?;
        // Don't start GeoClue just to ask; if it isn't running nothing is using it.
        let dbus = zbus::fdo::DBusProxy::new(&conn).await?;
        let name = zbus::names::BusName::try_from("org.freedesktop.GeoClue2")?;
        if !dbus.name_has_owner(name).await? {
            return Ok(false);
        }
        let proxy = zbus::Proxy::new(&conn, "org.freedesktop.GeoClue2", "/org/freedesktop/GeoClue2/Manager", "org.freedesktop.GeoClue2.Manager").await?;
        proxy.get_property::<bool>("InUse").await
    }
    matches!(tokio::time::timeout(Duration::from_secs(3), ask()).await, Ok(Ok(true)))
}

/// Everything in use right now (blocks briefly; call off the UI thread).
pub fn scan_now() -> Snapshot {
    Snapshot {
        microphone: microphone_apps(),
        camera: camera_apps(),
        location: worker::block_on(location_in_use()),
    }
}

/// Whether the tray indicator program is running.
pub fn daemon_running() -> bool {
    Command::new("pgrep").args(["-f", "zohara-settings --privacy-indicator"]).output().map(|o| o.status.success()).unwrap_or(false)
}

pub fn start_daemon() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = Command::new(exe).arg("--privacy-indicator").spawn();
    }
}

// ── Activity log ───────────────────────────────────────────────────────────

fn log_path() -> PathBuf {
    diag::state_dir().join("privacy-access.log")
}

/// One line per event: `time<TAB>device<TAB>app<TAB>started|stopped`.
fn record(device: &str, app: &str, what: &str) {
    let _ = fs::create_dir_all(diag::state_dir());
    let path = log_path();
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{}\t{device}\t{app}\t{what}", diag::timestamp());
    }
    // Keep it short.
    if let Ok(text) = fs::read_to_string(&path) {
        let lines: Vec<&str> = text.lines().collect();
        if lines.len() > MAX_LOG_LINES * 2 {
            let _ = fs::write(&path, lines[lines.len() - MAX_LOG_LINES..].join("\n") + "\n");
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub time: String,
    pub device: String,
    pub app: String,
    pub started: bool,
}

pub fn parse_log(text: &str) -> Vec<Entry> {
    text.lines()
        .filter_map(|l| {
            let mut f = l.split('\t');
            Some(Entry { time: f.next()?.into(), device: f.next()?.into(), app: f.next()?.into(), started: f.next()? == "started" })
        })
        .collect()
}

/// Newest first.
pub fn recent_activity(limit: usize) -> Vec<Entry> {
    let mut v = parse_log(&fs::read_to_string(log_path()).unwrap_or_default());
    v.reverse();
    v.truncate(limit);
    v
}

pub fn clear_activity() {
    let _ = fs::remove_file(log_path());
}

/// (started, stopped) between two sets of app names.
pub fn changes(before: &[String], after: &[String]) -> (Vec<String>, Vec<String>) {
    let started = after.iter().filter(|a| !before.contains(a)).cloned().collect();
    let stopped = before.iter().filter(|b| !after.contains(b)).cloned().collect();
    (started, stopped)
}

// ── StatusNotifier item ────────────────────────────────────────────────────

struct Item {
    id: &'static str,
    title: &'static str,
    icon: &'static str,
    active: bool,
    tip: String,
}

fn tooltip_text(what: &str, apps: &[String]) -> String {
    if apps.is_empty() {
        format!("Something is using your {what}")
    } else {
        format!("Using your {what}: {}", apps.join(", "))
    }
}

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl Item {
    #[zbus(property)]
    fn category(&self) -> String {
        "Hardware".into()
    }
    #[zbus(property)]
    fn id(&self) -> String {
        self.id.into()
    }
    #[zbus(property)]
    fn title(&self) -> String {
        self.title.into()
    }
    #[zbus(property)]
    fn status(&self) -> String {
        let s = if self.active { "Active" } else { "Passive" };
        s.into()
    }
    #[zbus(property)]
    fn window_id(&self) -> u32 {
        0
    }
    #[zbus(property)]
    fn icon_name(&self) -> String {
        self.icon.into()
    }
    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn menu(&self) -> ObjectPath<'static> {
        ObjectPath::from_static_str_unchecked("/NO_DBUSMENU")
    }
    #[zbus(property)]
    fn tool_tip(&self) -> (String, Vec<(i32, i32, Vec<u8>)>, String, String) {
        (self.icon.into(), Vec::new(), self.title.into(), self.tip.clone())
    }

    fn activate(&self, _x: i32, _y: i32) {
        if let Ok(exe) = std::env::current_exe() {
            let _ = Command::new(exe).args(["--page", "Privacy & security"]).spawn();
        }
    }
    fn secondary_activate(&self, _x: i32, _y: i32) {}
    fn context_menu(&self, _x: i32, _y: i32) {}
    fn scroll(&self, _delta: i32, _orientation: &str) {}

    #[zbus(signal)]
    async fn new_status(emitter: &SignalEmitter<'_>, status: String) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn new_tool_tip(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn new_icon(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

struct Device {
    path: &'static str,
    key: &'static str,
    noun: &'static str,
}

const DEVICES: [Device; 3] = [
    Device { path: "/StatusNotifierItem/microphone", key: "microphone", noun: "microphone" },
    Device { path: "/StatusNotifierItem/camera", key: "camera", noun: "camera" },
    Device { path: "/StatusNotifierItem/location", key: "location", noun: "location" },
];

async fn register(conn: &zbus::Connection) -> bool {
    let mut ok = true;
    for d in &DEVICES {
        // A leading "/" tells Plasma's watcher to use our connection and this path.
        let r = conn
            .call_method(Some("org.kde.StatusNotifierWatcher"), "/StatusNotifierWatcher", Some("org.kde.StatusNotifierWatcher"), "RegisterStatusNotifierItem", &d.path)
            .await;
        ok &= r.is_ok();
    }
    ok
}

/// True if the tray's watcher still lists all three of our items.
async fn registered(conn: &zbus::Connection) -> bool {
    let Ok(p) = zbus::Proxy::new(conn, "org.kde.StatusNotifierWatcher", "/StatusNotifierWatcher", "org.kde.StatusNotifierWatcher").await else {
        return false;
    };
    let Ok(items) = p.get_property::<Vec<String>>("RegisteredStatusNotifierItems").await else { return false };
    DEVICES.iter().all(|d| items.iter().any(|i| i.ends_with(d.path)))
}

async fn set_item(server: &zbus::ObjectServer, d: &Device, apps: Option<&[String]>) -> zbus::Result<()> {
    let iface = server.interface::<_, Item>(d.path).await?;
    let (changed, status) = {
        let mut item = iface.get_mut().await;
        let active = apps.is_some();
        let tip = apps.map(|a| tooltip_text(d.noun, a)).unwrap_or_default();
        let changed = item.active != active || item.tip != tip;
        item.active = active;
        item.tip = tip;
        (changed, if active { "Active" } else { "Passive" })
    };
    if changed {
        Item::new_status(iface.signal_emitter(), status.to_string()).await?;
        Item::new_tool_tip(iface.signal_emitter()).await?;
        Item::new_icon(iface.signal_emitter()).await?;
    }
    Ok(())
}

/// `zohara-settings --privacy-indicator`. Returns the process exit code.
pub fn run() -> i32 {
    worker::block_on(async {
        let mut builder = match zbus::connection::Builder::session().and_then(|b| b.name(BUS_NAME)) {
            Ok(b) => b,
            Err(e) => {
                log::error!("privacy indicator: {e}");
                return 1;
            }
        };
        let items = [("Microphone in use", "audio-input-microphone", "zohara-privacy-microphone"), ("Camera in use", "camera-web", "zohara-privacy-camera"), ("Location in use", "find-location", "zohara-privacy-location")];
        for (d, (title, icon, id)) in DEVICES.iter().zip(items) {
            builder = match builder.serve_at(d.path, Item { id, title, icon, active: false, tip: String::new() }) {
                Ok(b) => b,
                Err(e) => {
                    log::error!("privacy indicator: {e}");
                    return 1;
                }
            };
        }
        let conn = match builder.build().await {
            Ok(c) => c,
            Err(zbus::Error::NameTaken) => return 0, // already running
            Err(e) => {
                log::error!("privacy indicator: couldn't start: {e}");
                return 1;
            }
        };
        log::info!("privacy indicator running");

        let mut before = Snapshot::default();
        let mut tick: u32 = 0;
        loop {
            // Plasma may not be ready yet at sign-in, and can restart later.
            if tick % 10 == 0 && !registered(&conn).await {
                register(&conn).await;
            }
            tick = tick.wrapping_add(1);

            let cfg = load_config();
            let now = if cfg.enabled {
                let (mic, cam) = tokio::task::spawn_blocking(|| (microphone_apps(), camera_apps())).await.unwrap_or_default();
                Snapshot { microphone: mic, camera: cam, location: location_in_use().await }
            } else {
                Snapshot::default()
            };

            let server = conn.object_server();
            let loc_apps: Vec<String> = Vec::new();
            let sets: [(&Device, &[String], bool); 3] = [
                (&DEVICES[0], now.microphone.as_slice(), cfg.microphone && !now.microphone.is_empty()),
                (&DEVICES[1], now.camera.as_slice(), cfg.camera && !now.camera.is_empty()),
                (&DEVICES[2], loc_apps.as_slice(), cfg.location && now.location),
            ];
            for (d, apps, active) in sets {
                if let Err(e) = set_item(server, d, active.then_some(apps)).await {
                    log::warn!("privacy indicator: updating {}: {e}", d.key);
                }
            }

            for (device, b, a) in [("microphone", &before.microphone, &now.microphone), ("camera", &before.camera, &now.camera)] {
                let (started, stopped) = changes(b, a);
                for app in started {
                    record(device, &app, "started");
                }
                for app in stopped {
                    record(device, &app, "stopped");
                }
            }
            if now.location != before.location {
                record("location", "An app", if now.location { "started" } else { "stopped" });
            }
            before = now;
            tokio::time::sleep(POLL).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn config_defaults_and_overrides() {
        assert_eq!(parse_config(""), Config::default());
        assert!(!Config::default().microphone, "Plasma already shows a microphone icon");
        let c = parse_config("[Other]\nCamera=false\n[Indicators]\nMicrophone=false\nEnabled=true\n");
        assert!(c.camera && !c.microphone && c.enabled);
    }

    #[test]
    fn finds_real_recorders_only() {
        let sources = json!([
            {"index": 1, "name": "alsa_output.pci.monitor", "monitor_of_sink": 3},
            {"index": 2, "name": "alsa_input.pci", "monitor_of_sink": null},
        ]);
        let outputs = json!([
            {"source": 2, "corked": false, "properties": {"application.name": "firefox"}},
            {"source": 2, "corked": true,  "properties": {"application.name": "paused"}},
            {"source": 1, "corked": false, "properties": {"application.name": "recorder-of-speakers"}},
            {"source": 2, "corked": false, "properties": {"application.process.binary": "plasmashell", "media.name": "Peak detect"}},
            {"source": 2, "corked": false, "properties": {"application.process.binary": "obs"}},
        ]);
        assert_eq!(parse_recording_apps(&outputs, &sources), ["Firefox", "Obs"]);
    }

    #[test]
    fn finds_running_camera_streams() {
        let dump = json!([
            {"info": {"state": "running", "props": {"media.class": "Stream/Input/Video", "application.name": "Chromium"}}},
            {"info": {"state": "idle", "props": {"media.class": "Stream/Input/Video", "application.name": "Idle"}}},
            {"info": {"state": "running", "props": {"media.class": "Stream/Input/Audio", "application.name": "Mic"}}},
        ]);
        assert_eq!(parse_pipewire_video(&dump), ["Chromium"]);
    }

    #[test]
    fn diffs_and_log() {
        let (s, e) = changes(&["a".into(), "b".into()], &["b".into(), "c".into()]);
        assert_eq!((s, e), (vec!["c".to_string()], vec!["a".to_string()]));
        let log = parse_log("2026-09-25 10:00:00\tcamera\tZoom\tstarted\nbroken line\n2026-09-25 10:05:00\tcamera\tZoom\tstopped\n");
        assert_eq!(log.len(), 2);
        assert!(log[0].started && !log[1].started);
    }

    #[test]
    fn tooltips() {
        assert_eq!(tooltip_text("camera", &["Zoom".into(), "OBS".into()]), "Using your camera: Zoom, OBS");
        assert_eq!(tooltip_text("location", &[]), "Something is using your location");
    }
}
