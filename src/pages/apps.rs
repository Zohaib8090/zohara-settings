//! Apps: installed applications (pacman and Flatpak) with uninstall, startup
//! apps from the XDG autostart folders, app sources (Flathub), hardware
//! video decoding, and entry points to Default apps, Apps for websites and
//! Offline maps.

use crate::backend::worker::in_background;
use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Packages the desktop can't run without; never offered for removal here.
const PROTECTED: &[&str] = &[
    "zohara-settings", "zohara-store", "plasma-desktop", "plasma-workspace", "kwin", "systemsettings",
    "dolphin", "konsole", "linux", "linux-zen", "systemd", "pacman", "sddm", "networkmanager", "pipewire",
    "bluedevil", "kscreen", "powerdevil", "xdg-desktop-portal-kde",
];

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
}

fn message(w: &impl IsA<gtk4::Widget>, title: &str, body: &str) {
    let d = adw::AlertDialog::new(Some(title), Some(body));
    d.add_response("ok", "OK");
    d.present(Some(w));
}

fn nav_row(title: &str, subtitle: &str, icon: &str) -> adw::ActionRow {
    let r = adw::ActionRow::new();
    r.set_title(title);
    r.set_subtitle(subtitle);
    r.add_prefix(&gtk4::Image::from_icon_name(icon));
    r.add_suffix(&gtk4::Image::from_icon_name("go-next-symbolic"));
    r.set_activatable(true);
    r
}

// ── Installed apps ─────────────────────────────────────────────────────────

#[derive(Clone)]
enum Source {
    Pacman(String),
    Flatpak(String),
    Other,
}

#[derive(Clone)]
struct App {
    name: String,
    icon: Option<gtk4::gio::Icon>,
    source: Source,
    version: String,
}

// gio::Icon isn't Send, so gather plain data on the worker and build icons on the GTK thread.
#[derive(Clone)]
struct RawApp {
    name: String,
    icon: String,
    desktop_path: String,
    flatpak: Option<String>,
}

fn raw_apps() -> Vec<RawApp> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    let mut dirs: Vec<PathBuf> = std::env::var("XDG_DATA_DIRS")
        .unwrap_or_else(|_| "/usr/local/share:/usr/share".into())
        .split(':')
        .map(|d| PathBuf::from(d).join("applications"))
        .collect();
    dirs.push(PathBuf::from("/var/lib/flatpak/exports/share/applications"));
    dirs.push(home().join(".local/share/flatpak/exports/share/applications"));
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("desktop") {
                continue;
            }
            let id = path.file_name().unwrap().to_string_lossy().to_string();
            if !seen.insert(id.clone()) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            let section: Vec<&str> = text
                .lines()
                .skip_while(|l| l.trim() != "[Desktop Entry]")
                .skip(1)
                .take_while(|l| !l.starts_with('['))
                .collect();
            let get = |k: &str| section.iter().find_map(|l| l.strip_prefix(k).map(|v| v.trim().to_string()));
            if get("NoDisplay=").as_deref() == Some("true")
                || get("Hidden=").as_deref() == Some("true")
                || get("Type=").as_deref() != Some("Application")
            {
                continue;
            }
            let Some(name) = get("Name=") else { continue };
            let flatpak = get("X-Flatpak=");
            out.push(RawApp {
                name,
                icon: get("Icon=").unwrap_or_default(),
                desktop_path: std::fs::canonicalize(&path).unwrap_or(path).to_string_lossy().to_string(),
                flatpak,
            });
        }
    }
    out
}

/// desktop file -> (owning package, version), in one pacman call.
fn owners(paths: &[String]) -> HashMap<String, (String, String)> {
    let mut map = HashMap::new();
    if paths.is_empty() {
        return map;
    }
    let out = Command::new("pacman").arg("-Qo").args(paths).output().map(|o| o.stdout).unwrap_or_default();
    // "/usr/share/applications/x.desktop is owned by pkg 1.2-1"
    for line in String::from_utf8_lossy(&out).lines() {
        if let Some((file, rest)) = line.split_once(" is owned by ") {
            let mut it = rest.split_whitespace();
            if let (Some(pkg), Some(ver)) = (it.next(), it.next()) {
                map.insert(file.to_string(), (pkg.to_string(), ver.to_string()));
            }
        }
    }
    map
}

fn flatpak_versions() -> HashMap<String, String> {
    Command::new("flatpak")
        .args(["list", "--app", "--columns=application,version"])
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter_map(|l| {
                    let mut p = l.split('\t');
                    Some((p.next()?.to_string(), p.next().unwrap_or("").to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

type Loaded = Vec<(RawApp, Option<(String, String)>, Option<String>)>;

fn load_apps() -> Loaded {
    let raw = raw_apps();
    let paths: Vec<String> = raw.iter().filter(|a| a.flatpak.is_none()).map(|a| a.desktop_path.clone()).collect();
    let own = owners(&paths);
    let fv = flatpak_versions();
    let mut v: Loaded = raw
        .into_iter()
        .map(|a| {
            let o = own.get(&a.desktop_path).cloned();
            let f = a.flatpak.as_ref().and_then(|id| fv.get(id).cloned());
            (a, o, f)
        })
        .collect();
    v.sort_by(|a, b| a.0.name.to_lowercase().cmp(&b.0.name.to_lowercase()));
    v
}

fn to_app((raw, owner, fver): (RawApp, Option<(String, String)>, Option<String>)) -> App {
    let icon: Option<gtk4::gio::Icon> = if raw.icon.starts_with('/') {
        Some(gtk4::gio::FileIcon::new(&gtk4::gio::File::for_path(&raw.icon)).upcast())
    } else if !raw.icon.is_empty() {
        Some(gtk4::gio::ThemedIcon::new(&raw.icon).upcast())
    } else {
        None
    };
    let (source, version) = match (&raw.flatpak, owner) {
        (Some(id), _) => (Source::Flatpak(id.clone()), fver.unwrap_or_default()),
        (None, Some((pkg, ver))) => (Source::Pacman(pkg), ver),
        _ => (Source::Other, String::new()),
    };
    App { name: raw.name, icon, source, version }
}

fn uninstall(app: &App, page: &gtk4::Box, row: &adw::ActionRow) {
    let (title, detail, cmd): (String, String, Vec<String>) = match &app.source {
        Source::Pacman(pkg) => (
            format!("Uninstall {}?", app.name),
            format!("This removes the “{pkg}” package and anything that was only installed for it."),
            vec!["pkexec".into(), "pacman".into(), "-Rns".into(), "--noconfirm".into(), pkg.clone()],
        ),
        Source::Flatpak(id) => (
            format!("Uninstall {}?", app.name),
            "This removes the app. Its settings and data stay in your home folder.".into(),
            vec!["flatpak".into(), "uninstall".into(), "-y".into(), "--noninteractive".into(), id.clone()],
        ),
        Source::Other => return,
    };
    let d = adw::AlertDialog::new(Some(&title), Some(&detail));
    d.add_responses(&[("cancel", "Cancel"), ("remove", "Uninstall")]);
    d.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
    d.set_default_response(Some("cancel"));
    d.set_close_response("cancel");
    let (page2, row2) = (page.clone(), row.clone());
    d.connect_response(None, move |_, r| {
        if r != "remove" {
            return;
        }
        let cmd = cmd.clone();
        let (page3, row3) = (page2.clone(), row2.clone());
        row3.set_subtitle("Uninstalling…");
        row3.set_sensitive(false);
        in_background(
            move || {
                Command::new(&cmd[0])
                    .args(&cmd[1..])
                    .output()
                    .map(|o| (o.status.success(), String::from_utf8_lossy(&o.stderr).trim().to_string()))
                    .unwrap_or((false, String::new()))
            },
            move |(ok, err)| {
                if ok {
                    if let Some(parent) = row3.parent() {
                        if let Some(list) = parent.downcast_ref::<gtk4::ListBox>() {
                            list.remove(&row3);
                        }
                    }
                } else {
                    row3.set_sensitive(true);
                    row3.set_subtitle("Not uninstalled");
                    let body = if err.contains("required by") || err.contains("breaks dependency") {
                        "Other installed software depends on it.".to_string()
                    } else if err.is_empty() {
                        "Administrator approval was not given.".to_string()
                    } else {
                        err
                    };
                    message(&page3, "Couldn't uninstall", &body);
                }
            },
        );
    });
    d.present(Some(page));
}

fn installed_group(page: &gtk4::Box) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("Installed apps");
    let search = gtk4::SearchEntry::builder().placeholder_text("Search apps").build();
    search.set_valign(gtk4::Align::Center);
    g.set_header_suffix(Some(&search));

    let list = gtk4::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk4::SelectionMode::None);
    let loading = adw::ActionRow::new();
    loading.set_title("Finding your apps…");
    list.append(&loading);
    g.add(&list);

    let (list2, page2, search2) = (list.clone(), page.clone(), search.clone());
    in_background(load_apps, move |loaded| {
        list2.remove(&loading);
        let apps: Vec<App> = loaded.into_iter().map(to_app).collect();
        for app in apps {
            let row = adw::ActionRow::new();
            row.set_title(&glib::markup_escape_text(&app.name));
            let where_from = match &app.source {
                Source::Pacman(p) => format!("{p} {}", app.version),
                Source::Flatpak(_) => format!("Flatpak {}", app.version),
                Source::Other => "Installed manually".into(),
            };
            row.set_subtitle(&glib::markup_escape_text(where_from.trim()));
            let img = match &app.icon {
                Some(i) => gtk4::Image::from_gicon(i),
                None => gtk4::Image::from_icon_name("application-x-executable"),
            };
            img.set_pixel_size(32);
            row.add_prefix(&img);
            let removable = match &app.source {
                Source::Pacman(p) => !PROTECTED.contains(&p.as_str()) && !p.starts_with("plasma") && !p.starts_with("kf6"),
                Source::Flatpak(_) => true,
                Source::Other => false,
            };
            if removable {
                let btn = gtk4::Button::with_label("Uninstall");
                btn.set_valign(gtk4::Align::Center);
                let (app2, page3, row2) = (app.clone(), page2.clone(), row.clone());
                btn.connect_clicked(move |_| uninstall(&app2, &page3, &row2));
                row.add_suffix(&btn);
            }
            list2.append(&row);
        }
        let s = search2.clone();
        list2.set_filter_func(move |row| {
            let q = s.text().to_lowercase();
            q.is_empty()
                || row
                    .downcast_ref::<adw::ActionRow>()
                    .map(|r| r.title().to_lowercase().contains(&q) || r.subtitle().map(|s| s.to_lowercase().contains(&q)).unwrap_or(false))
                    .unwrap_or(true)
        });
        let l = list2.clone();
        search2.connect_search_changed(move |_| l.invalidate_filter());
    });
    g
}

// ── Startup apps ───────────────────────────────────────────────────────────

fn autostart_user_dir() -> PathBuf {
    std::env::var("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|_| home().join(".config")).join("autostart")
}

struct Autostart {
    file: String,
    name: String,
    comment: String,
    icon: String,
    enabled: bool,
}

fn parse_entry(text: &str) -> HashMap<String, String> {
    text.lines()
        .skip_while(|l| l.trim() != "[Desktop Entry]")
        .skip(1)
        .take_while(|l| !l.starts_with('['))
        .filter_map(|l| l.split_once('=').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
        .collect()
}

fn load_autostart() -> Vec<Autostart> {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "KDE".into());
    let mut by_file: HashMap<String, (HashMap<String, String>, bool)> = HashMap::new();
    // System entries first; a user entry with the same file name overrides it.
    for (dir, user) in [(PathBuf::from("/etc/xdg/autostart"), false), (autostart_user_dir(), true)] {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("desktop") {
                continue;
            }
            if let Ok(t) = std::fs::read_to_string(&p) {
                by_file.insert(p.file_name().unwrap().to_string_lossy().to_string(), (parse_entry(&t), user));
            }
        }
    }
    let mut out: Vec<Autostart> = by_file
        .into_iter()
        .filter_map(|(file, (e, _))| {
            // Skip entries meant for other desktops and hidden plumbing.
            if let Some(only) = e.get("OnlyShowIn") {
                if !only.split(';').any(|d| !d.is_empty() && desktop.contains(d)) {
                    return None;
                }
            }
            if let Some(not) = e.get("NotShowIn") {
                if not.split(';').any(|d| !d.is_empty() && desktop.contains(d)) {
                    return None;
                }
            }
            if e.get("NoDisplay").map(String::as_str) == Some("true") {
                return None;
            }
            Some(Autostart {
                name: e.get("Name").cloned().unwrap_or_else(|| file.trim_end_matches(".desktop").to_string()),
                comment: e.get("Comment").cloned().unwrap_or_default(),
                icon: e.get("Icon").cloned().unwrap_or_default(),
                enabled: e.get("Hidden").map(String::as_str) != Some("true")
                    && e.get("X-KDE-autostart-enabled").map(String::as_str) != Some("false"),
                file,
            })
        })
        .collect();
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

/// Disabling writes a user copy with Hidden=true (the XDG way to turn off a
/// system autostart entry); enabling drops the flag, or removes our override
/// entirely when it was only hiding the system entry.
fn set_autostart(file: &str, on: bool) -> std::io::Result<()> {
    let user = autostart_user_dir().join(file);
    let system = Path::new("/etc/xdg/autostart").join(file);
    let strip = |t: &str| -> Vec<String> {
        t.lines()
            .filter(|l| !l.starts_with("Hidden=") && !l.starts_with("X-KDE-autostart-enabled="))
            .map(str::to_string)
            .collect()
    };
    if on {
        if let (Ok(u), Ok(s)) = (std::fs::read_to_string(&user), std::fs::read_to_string(&system)) {
            if strip(&u) == strip(&s) {
                return std::fs::remove_file(&user);
            }
        }
    }
    let base = std::fs::read_to_string(&user).or_else(|_| std::fs::read_to_string(&system))?;
    let mut lines = strip(&base);
    if !on {
        let at = lines.iter().position(|l| l.trim() == "[Desktop Entry]").map(|i| i + 1).unwrap_or(0);
        lines.insert(at, "Hidden=true".into());
    }
    std::fs::create_dir_all(autostart_user_dir())?;
    std::fs::write(&user, lines.join("
") + "
")
}

fn startup_group(page: &gtk4::Box) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("Startup");
    g.set_description(Some("Apps that start automatically when you sign in"));
    let entries = load_autostart();
    if entries.is_empty() {
        let r = adw::ActionRow::new();
        r.set_title("No startup apps");
        g.add(&r);
    }
    for a in entries {
        let row = adw::SwitchRow::new();
        row.set_title(&glib::markup_escape_text(&a.name));
        if !a.comment.is_empty() {
            row.set_subtitle(&glib::markup_escape_text(&a.comment));
        }
        let img = if a.icon.is_empty() {
            gtk4::Image::from_icon_name("system-run-symbolic")
        } else if a.icon.starts_with('/') {
            gtk4::Image::from_file(&a.icon)
        } else {
            gtk4::Image::from_icon_name(&a.icon)
        };
        img.set_pixel_size(24);
        row.add_prefix(&img);
        row.set_active(a.enabled);
        let (file, page2) = (a.file.clone(), page.clone());
        row.connect_active_notify(move |r| {
            if let Err(e) = set_autostart(&file, r.is_active()) {
                message(&page2, "Startup setting not changed", &e.to_string());
            }
        });
        g.add(&row);
    }
    g
}

// ── App sources ────────────────────────────────────────────────────────────

fn flathub_enabled() -> Option<bool> {
    let out = Command::new("flatpak").args(["remotes", "--columns=name,options"]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let line = text.lines().find(|l| l.split_whitespace().next() == Some("flathub"))?;
    Some(!line.contains("disabled"))
}

fn sources_group(page: &gtk4::Box) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("App sources");
    let flathub = adw::SwitchRow::new();
    flathub.set_title("Flathub");
    flathub.add_prefix(&gtk4::Image::from_icon_name("system-software-install-symbolic"));
    match flathub_enabled() {
        Some(on) => {
            flathub.set_active(on);
            flathub.set_subtitle("Thousands of apps packaged as Flatpaks, available in Zohara Store");
        }
        None => {
            flathub.set_active(false);
            flathub.set_subtitle("Not set up yet. Turn on to add it");
        }
    }
    let page2 = page.clone();
    let reverting = std::rc::Rc::new(std::cell::Cell::new(false));
    flathub.connect_active_notify(move |r| {
        if reverting.get() {
            return;
        }
        let on = r.is_active();
        let (r2, page3, reverting) = (r.clone(), page2.clone(), reverting.clone());
        in_background(
            move || {
                let exists = flathub_enabled().is_some();
                let args: Vec<&str> = match (on, exists) {
                    (true, false) => vec!["remote-add", "--if-not-exists", "flathub", "https://dl.flathub.org/repo/flathub.flatpakrepo"],
                    (true, true) => vec!["remote-modify", "--enable", "flathub"],
                    (false, _) => vec!["remote-modify", "--disable", "flathub"],
                };
                Command::new("flatpak").args(&args).status().map(|s| s.success()).unwrap_or(false)
            },
            move |ok| {
                if !ok {
                    reverting.set(true);
                    r2.set_active(!on);
                    reverting.set(false);
                    message(&page3, "Flathub not changed", "Administrator approval was not given, or there's no internet connection.");
                }
            },
        );
    });
    g.add(&flathub);
    g
}

// ── Video playback ─────────────────────────────────────────────────────────

fn gpu_vendors() -> Vec<&'static str> {
    let out = Command::new("lspci").arg("-nn").output().map(|o| o.stdout).unwrap_or_default();
    let text = String::from_utf8_lossy(&out).to_lowercase();
    let mut v = Vec::new();
    for line in text.lines().filter(|l| l.contains("vga") || l.contains("3d controller") || l.contains("display controller")) {
        if line.contains("intel") && !v.contains(&"intel") {
            v.push("intel");
        } else if (line.contains("amd") || line.contains("ati ")) && !v.contains(&"amd") {
            v.push("amd");
        } else if line.contains("nvidia") && !v.contains(&"nvidia") {
            v.push("nvidia");
        }
    }
    v
}

fn va_driver(name: &str) -> bool {
    Path::new(&format!("/usr/lib/dri/{name}_drv_video.so")).exists()
}

fn video_group(page: &gtk4::Box) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("Video playback");
    let row = adw::ActionRow::new();
    row.set_title("Hardware video decoding");
    row.add_prefix(&gtk4::Image::from_icon_name("video-x-generic-symbolic"));
    let vendors = gpu_vendors();
    // (supported, missing package that would enable it)
    let (ok, missing): (bool, Option<&str>) = if vendors.contains(&"intel") {
        if va_driver("iHD") || va_driver("i965") { (true, None) } else { (false, Some("intel-media-driver")) }
    } else if vendors.contains(&"amd") {
        (va_driver("radeonsi"), None)
    } else if vendors.contains(&"nvidia") {
        (va_driver("nvidia") || va_driver("nouveau"), None)
    } else {
        (false, None)
    };
    row.set_subtitle(match (ok, missing, vendors.is_empty()) {
        (true, _, _) => "On · Videos play smoothly and use less battery",
        (false, Some(_), _) => "Off · A driver for your graphics card isn't installed",
        (false, None, true) => "No graphics card detected",
        (false, None, false) => "Not available for your graphics card",
    });
    if let Some(pkg) = missing {
        let btn = gtk4::Button::with_label("Install driver");
        btn.add_css_class("suggested-action");
        btn.set_valign(gtk4::Align::Center);
        let (page2, row2) = (page.clone(), row.clone());
        btn.connect_clicked(move |b| {
            b.set_sensitive(false);
            let (page3, row3, b2) = (page2.clone(), row2.clone(), b.clone());
            in_background(
                move || Command::new("pkexec").args(["pacman", "-S", "--needed", "--noconfirm", pkg]).status().map(|s| s.success()).unwrap_or(false),
                move |done| {
                    if done {
                        row3.set_subtitle("On · Restart apps that are playing video");
                        b2.set_visible(false);
                    } else {
                        b2.set_sensitive(true);
                        message(&page3, "Driver not installed", "Administrator approval was not given, or there's no internet connection.");
                    }
                },
            );
        });
        row.add_suffix(&btn);
    }
    g.add(&row);
    g
}

// ── Page ───────────────────────────────────────────────────────────────────

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
            .label("Apps")
            .halign(gtk4::Align::Start)
            .css_classes(vec!["win11-page-title".to_string()])
            .build(),
    );

    let more = adw::PreferencesGroup::new();
    let defaults = nav_row("Default apps", "Choose which apps open links, files and email", "preferences-desktop-default-applications-symbolic");
    {
        let root = root.clone();
        defaults.connect_activated(move |_| super::goto(&root, "Default apps"));
    }
    more.add(&defaults);
    let web = nav_row("Apps for websites", "Turn any website into an app with its own window and permissions", "applications-internet-symbolic");
    {
        let root = root.clone();
        web.connect_activated(move |_| super::web_apps::open(root.upcast_ref()));
    }
    more.add(&web);
    let maps = nav_row("Offline maps", "Download maps of the places you choose to use without internet", "find-location-symbolic");
    {
        let root = root.clone();
        maps.connect_activated(move |_| super::offline_maps::open(root.upcast_ref()));
    }
    more.add(&maps);
    root.append(&more);

    root.append(&installed_group(&root));
    root.append(&startup_group(&root));
    root.append(&sources_group(&root));
    root.append(&video_group(&root));

    scroll.set_child(Some(&root));
    scroll.upcast()
}
