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
                    && e.get("X-KDE-autostart-enabled").map(String::as_str) != Some("false"),
                own,
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
    let startup = nav_row("Startup apps", "Apps that start when you sign in: turn off, start, remove or add", "system-run-symbolic");
    {
        let root = root.clone();
        startup.connect_activated(move |_| open_startup(root.upcast_ref()));
    }
    more.add(&startup);
    root.append(&more);

    root.append(&installed_group(&root));

    root.append(&sources_group(&root));
    root.append(&video_group(&root));

    scroll.set_child(Some(&root));
    scroll.upcast()
}

#[cfg(test)]
mod startup_tests {
    use super::*;

    /// The whole add / list / switch off / switch on / remove cycle against a throwaway folder.
    #[test]
    fn startup_entries_can_be_added_switched_and_removed() {
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
    fn custom_entry_is_a_valid_one_line_entry() {
        let t = custom_entry("My app", "firefox --private\nrm -rf /");
        assert!(t.starts_with("[Desktop Entry]\nType=Application\nName=My app\nExec=firefox --private rm -rf /\n"));
        assert_eq!(t.matches("Exec=").count(), 1);
    }
}
