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
    /// A package built from the AUR (no repository carries it).
    Aur(String),
    Flatpak(String),
    /// An AppImage installed through Zohara Store (its id).
    AppImage(String),
    /// An app inside a distrobox container (the Ubuntu one Zohara sets up for .deb files), exported to the menu; the container's name.
    Container(String),
    /// A program unpacked from an archive (Apps from archives): its short name. Removed in "Apps from archives" above.
    Archive(String),
    Other,
}

#[derive(Clone)]
struct App {
    name: String,
    icon: Option<gtk4::gio::Icon>,
    source: Source,
    version: String,
    /// The launcher's file name (`firefox.desktop`) and where it is.
    desktop_id: String,
    desktop_path: String,
    /// What the launcher says the program is for.
    comment: String,
    /// A part of the desktop rather than an app (settings module, Xfce/Cinnamon piece).
    system: bool,
    /// The desktop needs it: never offered for removal.
    protected: bool,
}

// gio::Icon isn't Send, so gather plain data on the worker and build icons on the GTK thread.
#[derive(Clone)]
struct RawApp {
    id: String,
    name: String,
    icon: String,
    desktop_path: String,
    flatpak: Option<String>,
    /// An AppImage from the Store: its id.
    appimage: Option<String>,
    /// Its package is built from the AUR.
    aur: bool,
    /// It starts inside this distrobox container.
    container: Option<String>,
    /// Installed from an archive by Zohara (`zohara-archive-<slug>.desktop`).
    archive: bool,
    /// The launcher's own description (`Comment=`).
    comment: Option<String>,
    categories: String,
    only_show_in: Option<String>,
}

/// Parts of the desktop that are not apps a person chose: settings modules and the pieces of other desktops (Xfce, Cinnamon).
/// Hidden from the list unless "Show system components" is on.
fn is_system_component(categories: &str, only_show_in: Option<&str>, pkg: Option<&str>) -> bool {
    let cats: Vec<&str> = categories.split(';').map(str::trim).collect();
    if cats.iter().any(|c| matches!(*c, "X-XFCE-SettingsDialog" | "X-XFCE-SystemSettings" | "X-GNOME-Settings-Panel" | "X-Cinnamon-Settings")) {
        return true;
    }
    // Made for another desktop only (OnlyShowIn=XFCE; or X-Cinnamon;): not something to pick from a Plasma app list.
    if let Some(o) = only_show_in {
        let o = o.trim();
        if !o.is_empty() && !o.split(';').any(|d| d.trim().eq_ignore_ascii_case("kde")) {
            return true;
        }
    }
    pkg.map(|p| ["libxfce4", "xfwm4", "xfdesktop", "xfconf", "muffin", "cinnamon-", "cjs"].iter().any(|pre| p.starts_with(pre)) || p == "cinnamon")
        .unwrap_or(false)
}

/// Needed by the desktop itself: listed, but never offered for removal.
fn is_protected(pkg: &str) -> bool {
    PROTECTED.contains(&pkg) || pkg.starts_with("plasma") || pkg.starts_with("kf6")
}

/// The container a launcher starts its program in: `Exec=/usr/bin/distrobox-enter -n zohara-deb-runner -- app` -> `zohara-deb-runner`.
/// `None` for an ordinary launcher.
fn distrobox_container(exec: &str) -> Option<String> {
    if !exec.contains("distrobox-enter") && !exec.contains("distrobox enter") {
        return None;
    }
    let mut it = exec.split_whitespace();
    while let Some(w) = it.next() {
        if w == "-n" || w == "--name" {
            return it.next().map(|n| n.trim_matches('"').trim_matches('\'').to_string()).filter(|n| !n.is_empty());
        }
    }
    Some("container".to_string())
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
    // The person's own launchers (~/.local/share/applications) are not in XDG_DATA_DIRS. Only the ones that start a distrobox
    // container are listed from there (the Ubuntu apps installed from a .deb); shortcuts, Android and web-app entries have their own pages.
    let user_dir = std::env::var("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|_| home().join(".local/share")).join("applications");
    let mut dirs: Vec<(PathBuf, bool)> = dirs.into_iter().map(|d| (d, false)).collect();
    dirs.insert(0, (user_dir, true));
    for (dir, containers_only) in dirs {
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
            let container = get("Exec=").and_then(|e| distrobox_container(&e));
            let archive = id.starts_with("zohara-archive-");
            if containers_only && container.is_none() && !archive {
                seen.remove(&id);
                continue;
            }
            let flatpak = get("X-Flatpak=");
            out.push(RawApp {
                id: id.clone(),
                name,
                icon: get("Icon=").unwrap_or_default(),
                desktop_path: std::fs::canonicalize(&path).unwrap_or(path).to_string_lossy().to_string(),
                flatpak,
                appimage: None,
                aur: false,
                container,
                archive,
                comment: get("Comment=").filter(|c| !c.is_empty()),
                categories: get("Categories=").unwrap_or_default(),
                only_show_in: get("OnlyShowIn="),
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
    let mut raw = raw_apps();
    // AppImages installed through the Store keep their launchers in the person's own folder, which the system lists don't cover.
    for a in crate::backend::community_apps::appimages() {
        raw.push(RawApp {
            id: a.launcher.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
            name: a.name,
            icon: a.icon,
            desktop_path: a.launcher.to_string_lossy().to_string(),
            flatpak: None,
            appimage: Some(a.slug),
            aur: false,
            container: None,
            archive: false,
            comment: None,
            categories: String::new(),
            only_show_in: None,
        });
    }
    let paths: Vec<String> = raw.iter().filter(|a| a.flatpak.is_none() && a.appimage.is_none() && a.container.is_none() && !a.archive).map(|a| a.desktop_path.clone()).collect();
    let own = owners(&paths);
    let fv = flatpak_versions();
    let foreign = crate::backend::community_apps::foreign_packages();
    let mut v: Loaded = raw
        .into_iter()
        .map(|mut a| {
            let o = own.get(&a.desktop_path).cloned();
            a.aur = o.as_ref().map(|(pkg, _)| crate::backend::community_apps::is_aur(pkg, &foreign)).unwrap_or(false);
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
    let (source, version) = match (&raw.appimage, &raw.flatpak, owner) {
        _ if raw.container.is_some() => (Source::Container(raw.container.clone().unwrap_or_default()), String::new()),
        _ if raw.archive => (Source::Archive(raw.id.trim_start_matches("zohara-archive-").trim_end_matches(".desktop").to_string()), String::new()),
        (Some(slug), _, _) => (Source::AppImage(slug.clone()), String::new()),
        (None, Some(id), _) => (Source::Flatpak(id.clone()), fver.unwrap_or_default()),
        (None, None, Some((pkg, ver))) if raw.aur => (Source::Aur(pkg), ver),
        (None, None, Some((pkg, ver))) => (Source::Pacman(pkg), ver),
        _ => (Source::Other, String::new()),
    };
    let pkg = match &source {
        Source::Pacman(p) | Source::Aur(p) => Some(p.as_str()),
        _ => None,
    };
    let system = is_system_component(&raw.categories, raw.only_show_in.as_deref(), pkg);
    let protected = pkg.map(is_protected).unwrap_or(false);
    App { name: raw.name, icon, source, version, desktop_id: raw.id, desktop_path: raw.desktop_path, comment: raw.comment.unwrap_or_default(), system, protected }
}

/// The graphics button of an app on a laptop with two chips: open it once on the stronger one, or always.
fn gpu_button(app: &App, page: &gtk4::Box) -> gtk4::MenuButton {
    use crate::backend::gpu_pref as pref;
    let btn = gtk4::MenuButton::new();
    btn.set_icon_name("video-display-symbolic");
    btn.set_valign(gtk4::Align::Center);
    btn.add_css_class("flat");
    btn.set_tooltip_text(Some("Which graphics chip this app uses"));

    let body = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    body.set_margin_top(10);
    body.set_margin_bottom(10);
    body.set_margin_start(10);
    body.set_margin_end(10);
    let heading = gtk4::Label::builder().label("Dedicated graphics card").xalign(0.0).css_classes(vec!["heading".to_string()]).build();
    body.append(&heading);
    let now = gtk4::Button::with_label("Open now with it");
    now.add_css_class("flat");
    let always = gtk4::CheckButton::with_label("Always use it for this app");
    let path = std::path::PathBuf::from(&app.desktop_path);
    // A Flatpak app takes the choice as its own settings, an ordinary one through its launcher.
    let flatpak_id = match &app.source {
        Source::Flatpak(id) => Some(id.clone()),
        _ => None,
    };
    always.set_active(match &flatpak_id {
        Some(id) => pref::flatpak_is_set(id),
        None => pref::is_set(&app.desktop_id, &path),
    });
    body.append(&now);
    body.append(&always);
    if flatpak_id.is_some() && !pref::flatpak_gpu_ready() {
        let note = gtk4::Label::builder()
            .label("This app can only use it once the NVIDIA part for Flatpak is installed. Update your Flatpak apps and runtimes (in the Store, or run “flatpak update”).")
            .wrap(true)
            .max_width_chars(34)
            .xalign(0.0)
            .css_classes(vec!["dim-label".to_string(), "caption".to_string()])
            .build();
        body.append(&note);
    }
    let pop = gtk4::Popover::new();
    pop.set_child(Some(&body));
    btn.set_popover(Some(&pop));

    {
        let (pop, path, page, fid) = (pop.clone(), path.clone(), page.clone(), flatpak_id.clone());
        now.connect_clicked(move |_| {
            pop.popdown();
            let started = match &fid {
                Some(id) => pref::flatpak_launch_now(id),
                None => pref::launch_now(&path),
            };
            if let Err(e) = started {
                message(&page, "Couldn't start the app", &e);
            }
        });
    }
    {
        let (id, page, fid) = (app.desktop_id.clone(), page.clone(), flatpak_id.clone());
        let undoing = std::rc::Rc::new(std::cell::Cell::new(false));
        always.connect_toggled(move |c| {
            if undoing.get() {
                return;
            }
            let on = c.is_active();
            let saved = match &fid {
                Some(app_id) => pref::flatpak_set(app_id, on),
                None => pref::set(&id, &path, on),
            };
            if let Err(e) = saved {
                undoing.set(true);
                c.set_active(!on);
                undoing.set(false);
                message(&page, "Couldn't change it", &e);
            }
        });
    }
    btn
}

/// What `pacman -Rs` would do, read without administrator rights.
#[derive(Debug, PartialEq, Eq)]
enum Preview {
    /// These packages would be removed (the app and what only it needed).
    Removes(Vec<String>),
    /// These installed packages need it, so pacman would refuse.
    Blocked(Vec<String>),
}

/// Reads `pacman -Rsp --print-format %n` output (`stdout`) and its error text (`stderr`).
fn parse_preview(stdout: &str, stderr: &str) -> Preview {
    let mut needed_by: Vec<String> = Vec::new();
    for l in stderr.lines() {
        if let Some((_, who)) = l.split_once("' required by ") {
            let who = who.trim().to_string();
            if !who.is_empty() && !needed_by.contains(&who) {
                needed_by.push(who);
            }
        }
    }
    if !needed_by.is_empty() {
        return Preview::Blocked(needed_by);
    }
    Preview::Removes(stdout.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("::") && !l.starts_with("error")).map(String::from).collect())
}

fn removal_preview(pkg: &str) -> Option<Preview> {
    let out = Command::new("pacman").args(["-Rsp", "--print-format", "%n", pkg]).output().ok()?;
    Some(parse_preview(&String::from_utf8_lossy(&out.stdout), &String::from_utf8_lossy(&out.stderr)))
}

fn uninstall(app: &App, page: &gtk4::Box, row: &adw::ActionRow) {
    // For a package, look first at what would go with it and who needs it: the confirmation says so.
    let pkg = match &app.source {
        Source::Pacman(p) | Source::Aur(p) => Some(p.clone()),
        _ => None,
    };
    let Some(pkg) = pkg else { return show_uninstall(app, page, row, None) };
    let (app2, page2, row2) = (app.clone(), page.clone(), row.clone());
    row.set_sensitive(false);
    in_background(move || removal_preview(&pkg), move |pv| {
        row2.set_sensitive(true);
        match pv {
            Some(Preview::Blocked(by)) => message(
                &page2,
                &format!("{} can't be uninstalled", app2.name),
                &format!("These installed programs need it: {}. Remove them first, or leave it installed.", by.join(", ")),
            ),
            Some(Preview::Removes(list)) => {
                let others: Vec<&String> = list.iter().filter(|n| !matches!(&app2.source, Source::Pacman(p) | Source::Aur(p) if p == *n)).collect();
                let extra = if others.is_empty() {
                    None
                } else {
                    let shown: Vec<&str> = others.iter().take(8).map(|s| s.as_str()).collect();
                    let more = if others.len() > 8 { format!(" and {} more", others.len() - 8) } else { String::new() };
                    Some(format!("\n\nThese go with it, because nothing else needs them: {}{}.", shown.join(", "), more))
                };
                show_uninstall(&app2, &page2, &row2, extra)
            }
            None => show_uninstall(&app2, &page2, &row2, None),
        }
    });
}

fn show_uninstall(app: &App, page: &gtk4::Box, row: &adw::ActionRow, extra: Option<String>) {
    let (title, mut detail, cmd): (String, String, Vec<String>) = match &app.source {
        Source::Pacman(pkg) => (
            format!("Uninstall {}?", app.name),
            format!("This removes the “{pkg}” package and anything that was only installed for it."),
            vec!["pkexec".into(), "pacman".into(), "-Rns".into(), "--noconfirm".into(), pkg.clone()],
        ),
        Source::Aur(pkg) => (
            format!("Uninstall {}?", app.name),
            format!("This removes the “{pkg}” package, which was built from the AUR, and anything that was only installed for it."),
            vec!["pkexec".into(), "pacman".into(), "-Rns".into(), "--noconfirm".into(), pkg.clone()],
        ),
        Source::Flatpak(id) => (
            format!("Uninstall {}?", app.name),
            "This removes the app. Its settings and data stay in your home folder.".into(),
            vec!["flatpak".into(), "uninstall".into(), "-y".into(), "--noninteractive".into(), id.clone()],
        ),
        // An AppImage is one file: removing it needs no command and no password.
        Source::AppImage(slug) => (
            format!("Remove {}?", app.name),
            "This deletes the AppImage file and its menu entry. Its settings and data stay in your home folder.".into(),
            vec!["appimage".into(), slug.clone()],
        ),
        // Removing it means removing it inside its container; that is not done from here.
        Source::Container(_) | Source::Archive(_) | Source::Other => return,
    };
    if let Some(e) = extra {
        detail.push_str(&e);
    }
    let d = adw::AlertDialog::new(Some(&title), Some(&detail));
    d.set_body_use_markup(false);
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
                if cmd[0] == "appimage" {
                    return match crate::backend::community_apps::remove_appimage(&cmd[1]) {
                        Ok(()) => (true, String::new()),
                        Err(e) => (false, e),
                    };
                }
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

/// One row of the list as the filter and the search see it.
struct Item {
    /// Lower-case name, description and package, for the search box.
    text: String,
    /// n native (pacman, AUR), f flatpak, a AppImage or archive, c container, o anything else.
    kind: char,
    system: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Chip {
    All,
    Native,
    Flatpak,
    Archive,
    Container,
    System,
}

/// Does the row show with this search text (lower case), chip and "Show system components" switch?
fn item_shown(it: &Item, q: &str, chip: Chip, show_system: bool) -> bool {
    if !q.is_empty() && !it.text.contains(q) {
        return false;
    }
    match chip {
        Chip::System => it.system,
        _ if it.system && !show_system => false,
        Chip::All => true,
        Chip::Native => it.kind == 'n',
        Chip::Flatpak => it.kind == 'f',
        Chip::Archive => it.kind == 'a',
        Chip::Container => it.kind == 'c',
    }
}

/// (filter letter, badge text) of a source.
fn source_kind(s: &Source) -> (char, &'static str) {
    match s {
        Source::Pacman(_) => ('n', "Native"),
        Source::Aur(_) => ('n', "AUR"),
        Source::Flatpak(_) => ('f', "Flatpak"),
        Source::AppImage(_) => ('a', "AppImage"),
        Source::Archive(_) => ('a', "Archive"),
        Source::Container(_) => ('c', "Container"),
        Source::Other => ('o', "Manual"),
    }
}

fn badge(text: &str, tip: &str, extra_class: &str) -> gtk4::Label {
    let l = gtk4::Label::new(Some(text));
    l.add_css_class("win11-badge");
    if !extra_class.is_empty() {
        l.add_css_class(extra_class);
    }
    l.set_valign(gtk4::Align::Center);
    l.set_tooltip_text(Some(tip));
    l
}

fn installed_group(page: &gtk4::Box) -> adw::PreferencesGroup {
    use std::cell::RefCell;
    use std::rc::Rc;
    let g = adw::PreferencesGroup::new();
    g.set_title("Installed apps");
    g.set_description(Some("Finding your apps…"));
    // Scoped on purpose: this box only searches the apps below, not the settings (that is the box at the top of the window).
    let search = gtk4::SearchEntry::builder().placeholder_text("Search installed apps").tooltip_text("Searches only the apps in this list").build();
    search.set_valign(gtk4::Align::Center);
    g.set_header_suffix(Some(&search));

    // Where an app came from, and whether parts of the desktop are shown.
    let bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    bar.set_margin_bottom(8);
    let chips_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    let mut chips: Vec<(Chip, gtk4::ToggleButton)> = Vec::new();
    for (chip, label, tip) in [
        (Chip::All, "All", "Every app"),
        (Chip::Native, "Native", "Installed with the system's package manager"),
        (Chip::Flatpak, "Flatpak", "Installed from Flathub"),
        (Chip::Archive, "Archive", "AppImages and programs unpacked from an archive"),
        (Chip::Container, "Container", "Ubuntu apps that run in the container"),
        (Chip::System, "System", "Settings modules and pieces of other desktops"),
    ] {
        let b = gtk4::ToggleButton::with_label(label);
        b.set_tooltip_text(Some(tip));
        if let Some((_, first)) = chips.first() {
            b.set_group(Some(first));
        }
        chips_box.append(&b);
        chips.push((chip, b));
    }
    chips[0].1.set_active(true);
    let spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    let sys_label = gtk4::Label::new(Some("Show system components"));
    let sys_switch = gtk4::Switch::new();
    sys_switch.set_valign(gtk4::Align::Center);
    sys_switch.set_tooltip_text(Some("Settings modules and the pieces of Xfce or Cinnamon are hidden by default"));
    bar.append(&chips_box);
    bar.append(&spacer);
    bar.append(&sys_label);
    bar.append(&sys_switch);
    g.add(&bar);

    let list = gtk4::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk4::SelectionMode::None);
    let loading = adw::ActionRow::new();
    loading.set_title("Finding your apps…");
    list.append(&loading);
    g.add(&list);

    let items: Rc<RefCell<Vec<Item>>> = Rc::new(RefCell::new(Vec::new()));
    let state: Rc<RefCell<(Chip, bool)>> = Rc::new(RefCell::new((Chip::All, false)));

    // Refilters the list and says how many apps show.
    let refresh: Rc<dyn Fn()> = {
        let (list, items, state, search, g) = (list.clone(), items.clone(), state.clone(), search.clone(), g.clone());
        Rc::new(move || {
            list.invalidate_filter();
            let items = items.borrow();
            if items.is_empty() {
                return;
            }
            let (chip, show) = *state.borrow();
            let q = search.text().to_lowercase();
            let n = items.iter().filter(|it| item_shown(it, &q, chip, show)).count();
            let hidden = items.iter().filter(|it| it.system).count();
            let note = if !show && chip != Chip::System && hidden > 0 { format!(" · {hidden} system components hidden") } else { String::new() };
            g.set_description(Some(&format!("Showing {n} of {} apps{note}", items.len())));
        })
    };
    for (chip, b) in &chips {
        let (chip, state, refresh) = (*chip, state.clone(), refresh.clone());
        b.connect_toggled(move |b| {
            if b.is_active() {
                state.borrow_mut().0 = chip;
                refresh();
            }
        });
    }
    {
        let (state, refresh) = (state.clone(), refresh.clone());
        sys_switch.connect_active_notify(move |s| {
            state.borrow_mut().1 = s.is_active();
            refresh();
        });
    }
    {
        let refresh = refresh.clone();
        search.connect_search_changed(move |_| refresh());
    }
    {
        let (items, state, search) = (items.clone(), state.clone(), search.clone());
        list.set_filter_func(move |row| {
            let (chip, show) = *state.borrow();
            let q = search.text().to_lowercase();
            match row.widget_name().parse::<usize>().ok().and_then(|i| items.borrow().get(i).map(|it| item_shown(it, &q, chip, show))) {
                Some(v) => v,
                None => true,
            }
        });
    }

    let (list2, page2) = (list.clone(), page.clone());
    let dual_gpu = crate::backend::gpu_pref::has_two_gpus();
    in_background(load_apps, move |loaded| {
        list2.remove(&loading);
        let apps: Vec<App> = loaded.into_iter().map(to_app).collect();
        let mut built: Vec<Item> = Vec::new();
        for app in apps {
            let row = adw::ActionRow::new();
            row.set_title(&glib::markup_escape_text(&app.name));
            let (kind, kind_label) = source_kind(&app.source);
            // What it is first (the launcher's own description), then where it came from and its package.
            let detail = match &app.source {
                Source::Pacman(p) => format!("{p} {}", app.version),
                Source::Aur(p) => format!("AUR · {p} {}", app.version),
                Source::Flatpak(_) => format!("Flatpak {}", app.version),
                Source::AppImage(_) => "AppImage".to_string(),
                Source::Archive(_) => "Unpacked from an archive".to_string(),
                Source::Container(c) => format!("Ubuntu container · {c}"),
                Source::Other => "Installed manually".into(),
            };
            let subtitle = if app.comment.is_empty() { detail.trim().to_string() } else { format!("{}\n{}", app.comment, detail.trim()) };
            row.set_subtitle(&glib::markup_escape_text(&subtitle));
            let pkg_text = match &app.source {
                Source::Pacman(p) | Source::Aur(p) => p.clone(),
                _ => String::new(),
            };
            built.push(Item { text: format!("{} {} {}", app.name, app.comment, pkg_text).to_lowercase(), kind, system: app.system });
            row.set_widget_name(&(built.len() - 1).to_string());
            let img = match &app.icon {
                Some(i) => gtk4::Image::from_gicon(i),
                None => gtk4::Image::from_icon_name("application-x-executable"),
            };
            img.set_pixel_size(32);
            row.add_prefix(&img);
            // Where it came from, and the two things worth a warning.
            row.add_suffix(&badge(kind_label, "Where this app came from", ""));
            if app.system {
                row.add_suffix(&badge("System", "A part of the desktop, not an app you chose", "win11-badge-system"));
            }
            if app.protected {
                row.add_suffix(&badge("Needed", "The desktop needs this, so it can't be uninstalled here", "win11-badge-system"));
            }
            // A laptop with two graphics chips: choose which one this app uses.
            if dual_gpu {
                let gb = gpu_button(&app, &page2);
                gb.set_tooltip_text(Some("Choose which graphics card this app uses"));
                row.add_suffix(&gb);
            }
            let removable = match &app.source {
                Source::Pacman(p) | Source::Aur(p) => !is_protected(p),
                Source::Flatpak(_) | Source::AppImage(_) => true,
                Source::Container(_) | Source::Archive(_) | Source::Other => false,
            };
            if removable {
                let btn = gtk4::Button::with_label("Uninstall");
                btn.set_valign(gtk4::Align::Center);
                btn.set_tooltip_text(Some(&format!("Uninstall {}", app.name)));
                let (app2, page3, row2) = (app.clone(), page2.clone(), row.clone());
                btn.connect_clicked(move |_| uninstall(&app2, &page3, &row2));
                row.add_suffix(&btn);
            }
            // Clicking the app opens its details: storage, notifications, permissions, uninstall.
            let target = super::app_details::Target {
                name: app.name.clone(),
                icon: app.icon.clone(),
                version: app.version.clone(),
                kind: match &app.source {
                    Source::Flatpak(id) => super::app_details::Kind::Flatpak(id.clone()),
                    Source::Pacman(p) => super::app_details::Kind::Pacman(p.clone()),
                    Source::Aur(p) => super::app_details::Kind::Aur(p.clone()),
                    Source::AppImage(s) => super::app_details::Kind::AppImage(s.clone()),
                    Source::Container(_) | Source::Archive(_) | Source::Other => super::app_details::Kind::Other,
                },
                desktop_id: app.desktop_id.clone(),
                desktop_path: app.desktop_path.clone(),
            };
            let remove_cb: Option<std::rc::Rc<dyn Fn()>> = if removable {
                let (app3, page4, row3) = (app.clone(), page2.clone(), row.clone());
                Some(std::rc::Rc::new(move || uninstall(&app3, &page4, &row3)))
            } else {
                None
            };
            row.set_activatable(true);
            let chevron = gtk4::Image::from_icon_name("go-next-symbolic");
            chevron.set_tooltip_text(Some("Open details: storage, permissions, notifications"));
            row.add_suffix(&chevron);
            row.connect_activated(move |r| super::app_details::open(r, target.clone(), remove_cb.clone()));
            list2.append(&row);
        }
        *items.borrow_mut() = built;
        refresh();
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
    exec: String,
    /// Added by the person or by an app in their own folder (no system entry of the same name): can be deleted.
    own: bool,
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
        .filter_map(|(file, (e, user))| {
            let own = user && !Path::new("/etc/xdg/autostart").join(&file).exists();
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
                    && e.get("X-KDE-autostart-enabled").map(String::as_str) != Some("false")
                    && !is_masked(&file),
                exec: e.get("Exec").cloned().unwrap_or_default(),
                own,
                file,
            })
        })
        .collect();
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

/// systemd's escaping of a name inside a unit name (what `systemd-escape` prints).
fn unit_escape(name: &str) -> String {
    let mut out = String::new();
    for (i, b) in name.bytes().enumerate() {
        match b {
            b'/' => out.push('-'),
            b'.' if i == 0 => out.push_str("\\x2e"),
            b if b.is_ascii_alphanumeric() || b == b':' || b == b'_' || b == b'.' => out.push(b as char),
            b => out.push_str(&format!("\\x{b:02x}")),
        }
    }
    out
}

/// The user unit the session makes for an autostart file (systemd-xdg-autostart-generator).
fn autostart_unit(file: &str) -> String {
    format!("app-{}@autostart.service", unit_escape(file.strip_suffix(".desktop").unwrap_or(file)))
}

fn mask_path(file: &str) -> PathBuf {
    let cfg = std::env::var("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|_| home().join(".config"));
    cfg.join("systemd/user").join(autostart_unit(file))
}

/// An app can write its own autostart file back when it starts (JetBrains Toolbox and many other apps taken from
/// archives do), which drops our `Hidden=true`. A masked unit cannot be started whatever the file says.
fn is_masked(file: &str) -> bool {
    std::fs::read_link(mask_path(file)).map(|t| t == Path::new("/dev/null")).unwrap_or(false)
}

fn set_masked(file: &str, on: bool) -> std::io::Result<()> {
    let path = mask_path(file);
    if !on {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    if is_masked(file) {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let _ = std::fs::remove_file(&path);
    std::os::unix::fs::symlink("/dev/null", &path)
}

/// Disabling writes a user copy with Hidden=true (the XDG way to turn off a
/// system autostart entry); enabling drops the flag, or removes our override
/// entirely when it was only hiding the system entry.
fn set_autostart(file: &str, on: bool) -> std::io::Result<()> {
    set_masked(file, !on)?;
    set_autostart_file(file, on)
}

fn set_autostart_file(file: &str, on: bool) -> std::io::Result<()> {
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

/// A desktop entry for a custom startup command.
pub fn custom_entry(name: &str, command: &str) -> String {
    let clean = |t: &str| t.replace(['\n', '\r'], " ").trim().to_string();
    format!("[Desktop Entry]\nType=Application\nName={}\nExec={}\nTerminal=false\nX-KDE-autostart-enabled=true\n", clean(name), clean(command))
}

/// File name for a new startup entry: letters, digits and dashes, not clashing with an existing one.
fn new_entry_file(name: &str) -> String {
    let base: String = name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect::<String>().split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    let base = if base.is_empty() { "startup-app".to_string() } else { base };
    let mut file = format!("{base}.desktop");
    let mut n = 2;
    while autostart_user_dir().join(&file).exists() || Path::new("/etc/xdg/autostart").join(&file).exists() {
        file = format!("{base}-{n}.desktop");
        n += 1;
    }
    file
}

fn add_autostart_file(file: &str, text: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(autostart_user_dir())?;
    std::fs::write(autostart_user_dir().join(file), text)
}

/// The program an `Exec=` line starts: first word, quotes removed, `env VAR=x` prefixes skipped, field codes ignored.
fn exec_program(exec: &str) -> Option<String> {
    let mut words: Vec<String> = Vec::new();
    let (mut cur, mut quote, mut any) = (String::new(), None::<char>, false);
    for c in exec.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                any = true;
            }
            (None, c) if c.is_whitespace() => {
                if any || !cur.is_empty() {
                    words.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            (None, c) => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        words.push(cur);
    }
    let mut it = words.into_iter().peekable();
    while let Some(w) = it.peek() {
        if w == "env" || (w.contains('=') && !w.starts_with('/') && !w.starts_with('%')) {
            it.next();
        } else {
            break;
        }
    }
    it.next().filter(|w| !w.is_empty() && !w.starts_with('%'))
}

/// Process ids whose program is `program` (an absolute path, or a bare command name). Our own process is never listed.
fn running_pids(program: &str) -> Vec<u32> {
    let me = std::process::id();
    let want_name = Path::new(program).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let absolute = program.starts_with('/');
    let Ok(dir) = std::fs::read_dir("/proc") else { return Vec::new() };
    let mut out = Vec::new();
    for e in dir.flatten() {
        let Some(pid) = e.file_name().to_string_lossy().parse::<u32>().ok().filter(|p| *p != me) else { continue };
        let exe = std::fs::read_link(format!("/proc/{pid}/exe")).ok();
        let argv0 = std::fs::read(format!("/proc/{pid}/cmdline")).ok().and_then(|c| c.split(|b| *b == 0).next().map(|a| String::from_utf8_lossy(a).to_string()));
        let hit = if absolute {
            exe.as_deref() == Some(Path::new(program)) || argv0.as_deref() == Some(program)
        } else {
            !want_name.is_empty() && argv0.as_deref().map(|a| Path::new(a).file_name().map(|n| n.to_string_lossy() == want_name).unwrap_or(false)).unwrap_or(false)
        };
        if hit {
            out.push(pid);
        }
    }
    out
}

fn stop_now(exec: &str) {
    if let Some(program) = exec_program(exec) {
        for pid in running_pids(&program) {
            let _ = Command::new("kill").arg(pid.to_string()).status();
        }
    }
}

/// Starts an entry now, the same way the session would.
fn start_now(file: &str) {
    let user = autostart_user_dir().join(file);
    let path = if user.exists() { user } else { Path::new("/etc/xdg/autostart").join(file) };
    let _ = Command::new("gio").arg("launch").arg(path).spawn();
}

/// "Add a startup app": pick an installed app, or type a command.
fn add_dialog(page: &gtk4::Box, refresh: std::rc::Rc<dyn Fn()>) {
    let win = adw::Window::builder().modal(true).default_width(520).default_height(620).title("Add a startup app").build();
    if let Some(p) = page.root().and_downcast::<gtk4::Window>() {
        win.set_transient_for(Some(&p));
    }
    let header = adw::HeaderBar::new();
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 14);
    content.set_margin_top(12);
    content.set_margin_bottom(16);
    content.set_margin_start(16);
    content.set_margin_end(16);

    let custom = adw::PreferencesGroup::new();
    custom.set_title("Run a command");
    let name = adw::EntryRow::new();
    name.set_title("Name");
    let command = adw::EntryRow::new();
    command.set_title("Command (for example: firefox)");
    let add_cmd = gtk4::Button::with_label("Add");
    add_cmd.add_css_class("suggested-action");
    add_cmd.set_valign(gtk4::Align::Center);
    command.add_suffix(&add_cmd);
    custom.add(&name);
    custom.add(&command);
    content.append(&custom);
    {
        let (win, refresh, name, command, page) = (win.clone(), refresh.clone(), name.clone(), command.clone(), page.clone());
        add_cmd.connect_clicked(move |_| {
            let (n, c) = (name.text().trim().to_string(), command.text().trim().to_string());
            if c.is_empty() {
                message(&win, "Enter a command", "For example: firefox");
                return;
            }
            let n = if n.is_empty() { c.split_whitespace().next().unwrap_or("App").to_string() } else { n };
            match add_autostart_file(&new_entry_file(&n), &custom_entry(&n, &c)) {
                Ok(()) => {
                    refresh();
                    win.close();
                }
                Err(e) => message(&page, "Not added", &e.to_string()),
            }
        });
    }

    let apps = adw::PreferencesGroup::new();
    apps.set_title("Or choose an installed app");
    let search = gtk4::SearchEntry::new();
    search.set_placeholder_text(Some("Search apps"));
    apps.add(&search);
    let list = gtk4::ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk4::SelectionMode::None);
    for a in raw_apps() {
        let row = adw::ActionRow::new();
        row.set_title(&glib::markup_escape_text(&a.name));
        let img = if a.icon.starts_with('/') { gtk4::Image::from_file(&a.icon) } else { gtk4::Image::from_icon_name(if a.icon.is_empty() { "application-x-executable" } else { &a.icon }) };
        img.set_pixel_size(24);
        row.add_prefix(&img);
        row.set_activatable(true);
        let (win, refresh, page, path, label) = (win.clone(), refresh.clone(), page.clone(), a.desktop_path.clone(), a.name.clone());
        row.connect_activated(move |_| match std::fs::read_to_string(&path).and_then(|t| add_autostart_file(&new_entry_file(&label), &t)) {
            Ok(()) => {
                refresh();
                win.close();
            }
            Err(e) => message(&page, "Not added", &e.to_string()),
        });
        list.append(&row);
    }
    {
        let s = search.clone();
        list.set_filter_func(move |row| {
            let q = s.text().to_lowercase();
            q.is_empty() || row.downcast_ref::<adw::ActionRow>().map(|r| r.title().to_lowercase().contains(&q)).unwrap_or(true)
        });
        let l = list.clone();
        search.connect_search_changed(move |_| l.invalidate_filter());
    }
    let scroll = gtk4::ScrolledWindow::builder().vexpand(true).min_content_height(260).child(&list).build();
    content.append(&apps);
    content.append(&scroll);

    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&content));
    win.set_content(Some(&view));
    win.present();
}

/// The Startup apps window (opened from the Apps page).
fn open_startup(parent: &gtk4::Widget) {
    let win = adw::Window::builder().default_width(640).default_height(620).title("Startup apps").build();
    if let Some(p) = parent.root().and_downcast::<gtk4::Window>() {
        win.set_transient_for(Some(&p));
        win.set_modal(true);
    }
    let header = adw::HeaderBar::new();
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 14);
    content.set_margin_top(12);
    content.set_margin_bottom(16);
    content.set_margin_start(16);
    content.set_margin_end(16);
    content.append(&startup_group(&content));
    let scroll = gtk4::ScrolledWindow::builder().hscrollbar_policy(gtk4::PolicyType::Never).vexpand(true).child(&content).build();
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&scroll));
    win.set_content(Some(&view));
    win.present();
}

fn startup_group(page: &gtk4::Box) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("Startup");
    g.set_description(Some("Apps that start automatically when you sign in. Turn one off to stop it, press play to start it now, or add your own."));
    let add = gtk4::Button::with_label("Add");
    add.add_css_class("suggested-action");
    add.set_valign(gtk4::Align::Center);
    g.set_header_suffix(Some(&add));

    let rows: std::rc::Rc<std::cell::RefCell<Vec<gtk4::Widget>>> = Default::default();
    let refill: std::rc::Rc<std::cell::RefCell<Option<std::rc::Rc<dyn Fn()>>>> = Default::default();
    let fill: std::rc::Rc<dyn Fn()> = {
        let (g, rows, page, refill) = (g.clone(), rows.clone(), page.clone(), refill.clone());
        std::rc::Rc::new(move || {
            for r in rows.borrow_mut().drain(..) {
                g.remove(&r);
            }
            let again = refill.borrow().clone().expect("set");
            let entries = load_autostart();
            if entries.is_empty() {
                let r = adw::ActionRow::new();
                r.set_title("No startup apps");
                g.add(&r);
                rows.borrow_mut().push(r.upcast());
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

                let play = gtk4::Button::from_icon_name("media-playback-start-symbolic");
                play.add_css_class("flat");
                play.set_valign(gtk4::Align::Center);
                play.set_tooltip_text(Some("Start it now"));
                let f = a.file.clone();
                play.connect_clicked(move |_| start_now(&f));
                row.add_suffix(&play);

                if exec_program(&a.exec).map(|p| !running_pids(&p).is_empty()).unwrap_or(false) {
                    let stop = gtk4::Button::from_icon_name("media-playback-stop-symbolic");
                    stop.add_css_class("flat");
                    stop.set_valign(gtk4::Align::Center);
                    stop.set_tooltip_text(Some("It is running now: stop it"));
                    let (exec, again2) = (a.exec.clone(), again.clone());
                    stop.connect_clicked(move |_| {
                        stop_now(&exec);
                        let again3 = again2.clone();
                        glib::timeout_add_local_once(std::time::Duration::from_millis(1200), move || again3());
                    });
                    row.add_suffix(&stop);
                }

                if a.own {
                    let del = gtk4::Button::from_icon_name("user-trash-symbolic");
                    del.add_css_class("flat");
                    del.set_valign(gtk4::Align::Center);
                    del.set_tooltip_text(Some("Remove from startup"));
                    let (f, page2, again2, name) = (a.file.clone(), page.clone(), again.clone(), a.name.clone());
                    del.connect_clicked(move |b| {
                        let d = adw::AlertDialog::new(Some(&format!("Remove {name} from startup?")), Some("It stops starting when you sign in. The app itself stays installed."));
                        d.add_responses(&[("cancel", "Cancel"), ("remove", "Remove")]);
                        d.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
                        let (f, page3, again3) = (f.clone(), page2.clone(), again2.clone());
                        d.connect_response(None, move |_, r| {
                            if r == "remove" {
                                match std::fs::remove_file(autostart_user_dir().join(&f)) {
                                    Ok(()) => again3(),
                                    Err(e) => message(&page3, "Not removed", &e.to_string()),
                                }
                            }
                        });
                        d.present(Some(b));
                    });
                    row.add_suffix(&del);
                }

                let (file, page2) = (a.file.clone(), page.clone());
                row.connect_active_notify(move |r| {
                    if let Err(e) = set_autostart(&file, r.is_active()) {
                        message(&page2, "Startup setting not changed", &e.to_string());
                    }
                });
                g.add(&row);
                rows.borrow_mut().push(row.upcast());
            }
        })
    };
    *refill.borrow_mut() = Some(fill.clone());
    fill();
    {
        let (page, fill) = (page.clone(), fill.clone());
        add.connect_clicked(move |_| add_dialog(&page, fill.clone()));
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
    // From /sys: instant. `lspci` took over two seconds on a hybrid laptop (it wakes the NVIDIA chip) and froze the page.
    let g = crate::backend::gpu::detect();
    let mut v = Vec::new();
    if g.intel {
        v.push("intel");
    }
    if g.amd {
        v.push("amd");
    }
    if g.nvidia {
        v.push("nvidia");
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
    row.set_activatable(false);
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
        (true, _, _) => "Videos play smoothly and use less battery. It is switched on by the system; there is nothing to set.",
        (false, Some(_), _) => "A driver for your graphics card isn't installed.",
        (false, None, true) => "No graphics card detected.",
        (false, None, false) => "Not available for your graphics card.",
    });
    // A status, not a switch: a label in the place a switch would be reads as "On" without looking clickable.
    let status = gtk4::Label::new(Some(if ok { "On" } else { "Off" }));
    status.add_css_class("win11-badge");
    if ok {
        status.add_css_class("win11-badge-system");
    }
    status.set_valign(gtk4::Align::Center);
    status.set_tooltip_text(Some("Status only: this is decided by your graphics driver"));
    row.add_suffix(&status);
    if let Some(pkg) = missing {
        let btn = gtk4::Button::with_label("Install driver");
        btn.add_css_class("suggested-action");
        btn.set_valign(gtk4::Align::Center);
        let (page2, row2, status2) = (page.clone(), row.clone(), status.clone());
        btn.connect_clicked(move |b| {
            b.set_sensitive(false);
            let (page3, row3, b2, status3) = (page2.clone(), row2.clone(), b.clone(), status2.clone());
            in_background(
                move || Command::new("pkexec").args(["pacman", "-S", "--needed", "--noconfirm", pkg]).status().map(|s| s.success()).unwrap_or(false),
                move |done| {
                    if done {
                        row3.set_subtitle("Restart apps that are playing video.");
                        status3.set_text("On");
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
    let maps = nav_row("Offline maps", "Download maps of the places you choose to use without internet", "mark-location-symbolic");
    {
        let root = root.clone();
        maps.connect_activated(move |_| super::offline_maps::open(root.upcast_ref()));
    }
    more.add(&maps);
    let startup = nav_row("Startup apps", "Apps that start when you sign in: turn off, start, remove or add", "system-run-symbolic");
    {
        let root = root.clone();
        startup.connect_activated(move |_| open_startup(root.upcast_ref()));
    }
    more.add(&startup);
    root.append(&more);

    root.append(&super::archive_apps::build());
    root.append(&installed_group(&root));
    // An information card, not something most people need: folded away below the list.
    let mac = gtk4::Expander::new(Some("Mac and iPhone apps: can this app run here?"));
    mac.set_child(Some(&super::mac_apps::build()));
    root.append(&mac);

    root.append(&sources_group(&root));
    root.append(&video_group(&root));

    scroll.set_child(Some(&root));
    scroll.upcast()
}

#[cfg(test)]
mod startup_tests {
    use super::*;

    /// These tests point XDG_CONFIG_HOME at their own folder, so they must not run at the same time.
    static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// The whole add / list / switch off / switch on / remove cycle against a throwaway folder.
    #[test]
    fn startup_entries_can_be_added_switched_and_removed() {
        let _lock = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("zs-startup-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("XDG_CONFIG_HOME", &dir);

        // Add a custom command, and a second one with the same name (must not overwrite the first).
        let f1 = new_entry_file("My Tool");
        add_autostart_file(&f1, &custom_entry("My Tool", "mytool --quiet")).unwrap();
        let f2 = new_entry_file("My Tool");
        add_autostart_file(&f2, &custom_entry("My Tool", "other")).unwrap();
        assert_eq!(f1, "my-tool.desktop");
        assert_eq!(f2, "my-tool-2.desktop");

        let find = |file: &str| load_autostart().into_iter().find(|a| a.file == file);
        let a = find(&f1).expect("listed");
        assert!(a.enabled && a.own);
        assert_eq!(a.name, "My Tool");

        // Switch off, then on again.
        set_autostart(&f1, false).unwrap();
        assert!(!find(&f1).unwrap().enabled);
        set_autostart(&f1, true).unwrap();
        assert!(find(&f1).unwrap().enabled);

        // Remove one: the other stays.
        std::fs::remove_file(autostart_user_dir().join(&f1)).unwrap();
        assert!(find(&f1).is_none());
        assert!(find(&f2).is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unit_names_match_what_the_session_makes() {
        assert_eq!(autostart_unit("jetbrains-toolbox.desktop"), "app-jetbrains\\x2dtoolbox@autostart.service");
        assert_eq!(autostart_unit("org.kde.plasma-fallback-session-restore.desktop"), "app-org.kde.plasma\\x2dfallback\\x2dsession\\x2drestore@autostart.service");
    }

    #[test]
    fn exec_lines_give_their_program() {
        assert_eq!(exec_program("/home/x/.local/share/zohara/archive-apps/tb/bin/tb --minimize").as_deref(), Some("/home/x/.local/share/zohara/archive-apps/tb/bin/tb"));
        assert_eq!(exec_program("\"/opt/My App/run\" %U").as_deref(), Some("/opt/My App/run"));
        assert_eq!(exec_program("env GTK_THEME=x firefox --private").as_deref(), Some("firefox"));
        assert_eq!(exec_program("").as_deref(), None);
    }

    #[test]
    fn a_masked_startup_app_stays_off_even_if_the_app_rewrites_its_file() {
        let _lock = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("zs-startup-mask-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("XDG_CONFIG_HOME", &dir);
        let f = new_entry_file("Archive App");
        add_autostart_file(&f, &custom_entry("Archive App", "/opt/app/run")).unwrap();
        let find = || load_autostart().into_iter().find(|a| a.file == f).unwrap();
        set_autostart(&f, false).unwrap();
        assert!(is_masked(&f) && !find().enabled);
        // The app writes its entry back without Hidden=true.
        add_autostart_file(&f, &custom_entry("Archive App", "/opt/app/run")).unwrap();
        assert!(!find().enabled, "still off");
        set_autostart(&f, true).unwrap();
        assert!(!is_masked(&f) && find().enabled);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn custom_entry_is_a_valid_one_line_entry() {
        let t = custom_entry("My app", "firefox --private\nrm -rf /");
        assert!(t.starts_with("[Desktop Entry]\nType=Application\nName=My app\nExec=firefox --private rm -rf /\n"));
        assert_eq!(t.matches("Exec=").count(), 1);
    }
}

#[cfg(test)]
mod container_tests {
    use super::*;

    #[test]
    fn a_distrobox_launcher_names_its_container() {
        assert_eq!(
            distrobox_container("/usr/bin/distrobox-enter  -n zohara-deb-runner  --  /opt/ChatGPT/chatgpt %U").as_deref(),
            Some("zohara-deb-runner")
        );
        assert_eq!(distrobox_container("/usr/bin/distrobox-enter -- foo").as_deref(), Some("container"));
        assert_eq!(distrobox_container("firefox %u"), None);
        assert_eq!(distrobox_container("env -n x steam"), None, "only distrobox launchers count");
    }
}

#[cfg(test)]
mod list_tests {
    use super::*;

    #[test]
    fn settings_modules_and_other_desktops_are_system_components() {
        assert!(is_system_component("Settings;DesktopSettings;X-XFCE-SettingsDialog;", None, Some("xfce4-settings")));
        assert!(!is_system_component("Settings;System;", None, Some("zohara-settings")), "a Settings-category tool of this desktop is an app");
        assert!(is_system_component("Utility;", Some("XFCE;"), None));
        assert!(is_system_component("", Some("X-Cinnamon;"), Some("cinnamon")));
        assert!(is_system_component("Utility;", None, Some("libxfce4ui")));
        assert!(!is_system_component("Utility;TerminalEmulator;", None, Some("xfce4-terminal")), "a terminal is an app");
        assert!(!is_system_component("Network;", Some("KDE;GNOME;"), Some("firefox")));
        assert!(!is_system_component("Network;WebBrowser;", None, None));
    }

    #[test]
    fn protected_packages_are_never_offered_for_removal() {
        assert!(is_protected("plasma-desktop") && is_protected("kf6-whatever") && is_protected("zohara-settings"));
        assert!(!is_protected("vlc"));
    }

    #[test]
    fn filters_and_search_combine() {
        let app = Item { text: "vlc media player vlc".into(), kind: 'n', system: false };
        let sys = Item { text: "workspaces xfwm4".into(), kind: 'n', system: true };
        let flat = Item { text: "zoom".into(), kind: 'f', system: false };
        assert!(item_shown(&app, "", Chip::All, false) && item_shown(&flat, "", Chip::All, false));
        assert!(!item_shown(&sys, "", Chip::All, false), "system components are hidden by default");
        assert!(item_shown(&sys, "", Chip::All, true));
        assert!(item_shown(&sys, "", Chip::System, false) && !item_shown(&app, "", Chip::System, true));
        assert!(item_shown(&app, "", Chip::Native, false) && !item_shown(&flat, "", Chip::Native, false));
        assert!(item_shown(&flat, "", Chip::Flatpak, false) && !item_shown(&app, "", Chip::Flatpak, false));
        assert!(item_shown(&app, "media", Chip::All, false) && !item_shown(&app, "zoom", Chip::All, false));
    }

    #[test]
    fn a_removal_preview_lists_what_goes_or_who_needs_it() {
        assert_eq!(parse_preview("vlc\nlibplacebo\n", ""), Preview::Removes(vec!["vlc".into(), "libplacebo".into()]));
        let err = "error: failed to prepare transaction (could not satisfy dependencies)\n:: removing zenity breaks dependency 'zenity' required by materialx\n:: removing zenity breaks dependency 'zenity' required by muffin\n";
        assert_eq!(parse_preview("", err), Preview::Blocked(vec!["materialx".into(), "muffin".into()]));
    }
}
