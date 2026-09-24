//! Apps for websites: turn a URL into a desktop app.
//!
//! Each web app gets its own Chromium-family browser profile (so logins,
//! cookies and permissions are separate), a launcher entry with the site's
//! icon, a window mode, optional start at sign-in (optionally minimized, via
//! a KWin rule that matches only the sign-in launch), and per-site
//! permissions written into that profile's Preferences.

use crate::backend::kconfig;
use crate::backend::worker::in_background;
use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;

const BROWSERS: &[&str] = &["brave-origin", "brave", "chromium", "google-chrome-stable", "microsoft-edge-stable", "vivaldi-stable"];
const PERMS: [(&str, &str); 4] = [
    ("notifications", "Notifications"),
    ("media_stream_camera", "Camera"),
    ("media_stream_mic", "Microphone"),
    ("geolocation", "Location"),
];
const PERM_CHOICES: [&str; 3] = ["Ask", "Allow", "Block"];
const MODES: [&str; 3] = ["Window", "Maximized", "Full screen"];

#[derive(Clone, Serialize, Deserialize, Default)]
struct WebApp {
    id: String,
    name: String,
    url: String,
    icon: String,
    /// index into MODES
    mode: u32,
    startup: bool,
    start_minimized: bool,
    /// per PERMS entry, index into PERM_CHOICES
    perms: Vec<u32>,
}

fn data_home() -> PathBuf {
    std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share"))
}

fn config_home() -> PathBuf {
    std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config"))
}

fn apps_dir() -> PathBuf {
    data_home().join("zohara-webapps")
}

fn app_dir(id: &str) -> PathBuf {
    apps_dir().join(id)
}

fn desktop_file(id: &str) -> PathBuf {
    data_home().join("applications").join(format!("zohara-webapp-{id}.desktop"))
}

fn autostart_file(id: &str) -> PathBuf {
    config_home().join("autostart").join(format!("zohara-webapp-{id}.desktop"))
}

fn browser() -> Option<&'static str> {
    BROWSERS.iter().copied().find(|b| {
        std::env::var("PATH")
            .unwrap_or_default()
            .split(':')
            .any(|d| std::path::Path::new(d).join(b).is_file())
    })
}

fn load_all() -> Vec<WebApp> {
    let mut v: Vec<WebApp> = std::fs::read_dir(apps_dir())
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| std::fs::read_to_string(e.path().join("app.json")).ok())
                .filter_map(|t| serde_json::from_str(&t).ok())
                .collect()
        })
        .unwrap_or_default();
    v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    v
}

fn slug(name: &str) -> String {
    let s: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let base = if s.is_empty() { "webapp".to_string() } else { s };
    let mut id = base.clone();
    let mut n = 2;
    while app_dir(&id).exists() {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

fn normalize_url(u: &str) -> String {
    let u = u.trim();
    if u.starts_with("http://") || u.starts_with("https://") {
        u.to_string()
    } else {
        format!("https://{u}")
    }
}

/// Desktop-entry Exec quoting: wrap in quotes and escape the reserved characters.
fn exec_quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' | '`' | '$' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            '%' => out.push_str("%%"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

fn exec_line(app: &WebApp, bin: &str, background: bool) -> String {
    let profile = app_dir(&app.id).join("profile");
    let class = if background { format!("zohara-webapp-{}-bg", app.id) } else { format!("zohara-webapp-{}", app.id) };
    let mut parts = vec![
        bin.to_string(),
        format!("--app={}", exec_quote(&app.url)),
        format!("--user-data-dir={}", exec_quote(&profile.to_string_lossy())),
        format!("--class={class}"),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
    ];
    match app.mode {
        1 => parts.push("--start-maximized".into()),
        2 => parts.push("--start-fullscreen".into()),
        _ => {}
    }
    parts.join(" ")
}

fn desktop_entry(app: &WebApp, bin: &str, background: bool) -> String {
    let icon = if app.icon.is_empty() { "applications-internet".to_string() } else { app.icon.clone() };
    let clean = |s: &str| s.replace(['\n', '\r'], " ");
    format!(
        "[Desktop Entry]\nType=Application\nName={}\nComment=Web app for {}\nExec={}\nIcon={}\nTerminal=false\nCategories=Network;\nStartupWMClass=zohara-webapp-{}\nX-Zohara-WebApp={}\n",
        clean(&app.name),
        clean(&app.url),
        exec_line(app, bin, background),
        clean(&icon),
        app.id,
        app.id
    )
}

/// Chromium content-setting values: 1 allow, 2 block, 3 ask.
fn write_permissions(app: &WebApp) -> std::io::Result<()> {
    let dir = app_dir(&app.id).join("profile").join("Default");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("Preferences");
    let mut prefs: serde_json::Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    for (i, (key, _)) in PERMS.iter().enumerate() {
        let v = match app.perms.get(i).copied().unwrap_or(0) {
            1 => 1,
            2 => 2,
            _ => 3,
        };
        prefs["profile"]["default_content_setting_values"][*key] = serde_json::json!(v);
    }
    std::fs::write(&path, serde_json::to_string(&prefs)?)
}

/// KWin rule that minimizes only the sign-in launch (it has its own window class).
fn set_background_rule(app: &WebApp, on: bool) {
    let group = format!("zohara-webapp-{}", app.id);
    let mut rules: Vec<String> = kconfig::read("kwinrulesrc", &["General"], "rules")
        .map(|r| r.split(',').filter(|s| !s.is_empty()).map(str::to_string).collect())
        .unwrap_or_default();
    rules.retain(|r| r != &group);
    if on {
        let g: &[&str] = &[&group];
        kconfig::write("kwinrulesrc", g, "Description", &format!("Start {} in the background", app.name));
        kconfig::write("kwinrulesrc", g, "wmclass", &format!("zohara-webapp-{}-bg", app.id));
        kconfig::write("kwinrulesrc", g, "wmclassmatch", "1");
        kconfig::write("kwinrulesrc", g, "minimize", "true");
        kconfig::write("kwinrulesrc", g, "minimizerule", "3");
        rules.push(group);
    }
    kconfig::write("kwinrulesrc", &["General"], "rules", &rules.join(","));
    kconfig::write("kwinrulesrc", &["General"], "count", &rules.len().to_string());
    kconfig::kwin_reconfigure();
}

fn save(app: &WebApp) -> Result<(), String> {
    let bin = browser().ok_or("No supported browser is installed (Brave, Chromium, Chrome, Edge or Vivaldi).")?;
    let dir = app_dir(&app.id);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("app.json"), serde_json::to_string_pretty(app).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    write_permissions(app).map_err(|e| e.to_string())?;
    let df = desktop_file(&app.id);
    std::fs::create_dir_all(df.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&df, desktop_entry(app, bin, false)).map_err(|e| e.to_string())?;
    let af = autostart_file(&app.id);
    if app.startup {
        std::fs::create_dir_all(af.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&af, desktop_entry(app, bin, app.start_minimized)).map_err(|e| e.to_string())?;
    } else {
        let _ = std::fs::remove_file(&af);
    }
    set_background_rule(app, app.startup && app.start_minimized);
    let _ = Command::new("update-desktop-database").arg(df.parent().unwrap()).status();
    Ok(())
}

fn remove(app: &WebApp) {
    set_background_rule(app, false);
    let _ = std::fs::remove_file(desktop_file(&app.id));
    let _ = std::fs::remove_file(autostart_file(&app.id));
    let _ = std::fs::remove_dir_all(app_dir(&app.id));
}

fn launch(app: &WebApp) {
    let _ = Command::new("gtk-launch").arg(format!("zohara-webapp-{}", app.id)).spawn();
}

fn is_running(app: &WebApp) -> bool {
    let profile = app_dir(&app.id).join("profile");
    Command::new("pgrep")
        .args(["-f", &format!("user-data-dir={}", profile.to_string_lossy())])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// ── Fetching the site's name and icon ──────────────────────────────────────

fn fetch(url: &str) -> Option<String> {
    let o = Command::new("curl")
        .args(["-sL", "--max-time", "10", "-A", "Mozilla/5.0 (X11; Linux x86_64) ZoharaSettings", url])
        .output()
        .ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).to_string())
}

fn html_title(html: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let start = lower.find("<title")?;
    let open_end = lower[start..].find('>')? + start + 1;
    let end = lower[open_end..].find("</title>")? + open_end;
    let t = html[open_end..end].trim();
    let t = t.replace("&amp;", "&").replace("&#39;", "'").replace("&quot;", "\"");
    (!t.is_empty()).then_some(t)
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_lowercase();
    let i = lower.find(&format!("{name}="))? + name.len() + 1;
    let rest = &tag[i..];
    let (q, body) = match rest.chars().next()? {
        c @ ('"' | '\'') => (Some(c), &rest[1..]),
        _ => (None, rest),
    };
    let end = match q {
        Some(c) => body.find(c)?,
        None => body.find(|c: char| c.is_whitespace() || c == '>').unwrap_or(body.len()),
    };
    Some(body[..end].to_string())
}

fn resolve(base: &str, href: &str) -> String {
    if href.starts_with("http://") || href.starts_with("https://") {
        return href.to_string();
    }
    let scheme_end = base.find("://").map(|i| i + 3).unwrap_or(0);
    let origin_end = base[scheme_end..].find('/').map(|i| i + scheme_end).unwrap_or(base.len());
    let origin = &base[..origin_end];
    if let Some(rest) = href.strip_prefix("//") {
        format!("{}{rest}", &base[..scheme_end])
    } else if href.starts_with('/') {
        format!("{origin}{href}")
    } else {
        let dir_end = base.rfind('/').filter(|i| *i >= origin_end).unwrap_or(origin_end);
        format!("{}/{href}", &base[..dir_end])
    }
}

/// Best icon candidates: apple-touch-icon (large PNG), declared icons, then /favicon.ico.
fn icon_candidates(url: &str, html: &str) -> Vec<String> {
    let mut touch = Vec::new();
    let mut icons = Vec::new();
    let lower = html.to_lowercase();
    let mut pos = 0;
    while let Some(i) = lower[pos..].find("<link") {
        let start = pos + i;
        let end = lower[start..].find('>').map(|e| start + e).unwrap_or(lower.len());
        let tag = &html[start..end];
        let rel = attr(tag, "rel").unwrap_or_default().to_lowercase();
        if let Some(href) = attr(tag, "href") {
            if rel.contains("apple-touch-icon") {
                touch.push(resolve(url, &href));
            } else if rel.split_whitespace().any(|r| r == "icon") {
                icons.push(resolve(url, &href));
            }
        }
        pos = end;
    }
    let mut out = touch;
    // Prefer PNG/SVG over ICO among declared icons.
    icons.sort_by_key(|u| if u.ends_with(".ico") { 1 } else { 0 });
    out.extend(icons);
    out.push(resolve(url, "/favicon.ico"));
    out
}

/// Returns (title, path of a downloaded icon).
fn fetch_details(url: String, id_hint: String) -> (Option<String>, Option<String>) {
    let html = fetch(&url).unwrap_or_default();
    let title = html_title(&html);
    let dir = data_home().join("zohara-webapps").join(".icons");
    let _ = std::fs::create_dir_all(&dir);
    for cand in icon_candidates(&url, &html) {
        let ext = cand.rsplit('.').next().filter(|e| ["png", "svg", "ico", "jpg", "jpeg", "webp"].contains(e)).unwrap_or("png");
        let path = dir.join(format!("{id_hint}.{ext}"));
        let ok = Command::new("curl")
            .args(["-sfL", "--max-time", "10", "-o"])
            .arg(&path)
            .arg(&cand)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok && std::fs::metadata(&path).map(|m| m.len() > 100).unwrap_or(false) {
            return (title, Some(path.to_string_lossy().to_string()));
        }
        let _ = std::fs::remove_file(&path);
    }
    (title, None)
}

// ── UI ─────────────────────────────────────────────────────────────────────

fn message(w: &impl IsA<gtk4::Widget>, title: &str, body: &str) {
    let d = adw::AlertDialog::new(Some(title), Some(body));
    d.add_response("ok", "OK");
    d.present(Some(w));
}

fn icon_image(path: &str, size: i32) -> gtk4::Image {
    let img = if !path.is_empty() && std::path::Path::new(path).exists() {
        gtk4::Image::from_file(path)
    } else {
        gtk4::Image::from_icon_name("applications-internet")
    };
    img.set_pixel_size(size);
    img
}

fn editor(parent: &gtk4::Window, existing: Option<WebApp>, on_saved: std::rc::Rc<dyn Fn()>) {
    let is_new = existing.is_none();
    let app = std::rc::Rc::new(std::cell::RefCell::new(existing.unwrap_or_else(|| WebApp { perms: vec![0; PERMS.len()], ..Default::default() })));

    let win = adw::Window::builder()
        .modal(true)
        .transient_for(parent)
        .default_width(560)
        .default_height(680)
        .title(if is_new { "New web app" } else { "Edit web app" })
        .build();
    let header = adw::HeaderBar::new();
    let save_btn = gtk4::Button::with_label(if is_new { "Create" } else { "Save" });
    save_btn.add_css_class("suggested-action");
    header.pack_end(&save_btn);
    let cancel = gtk4::Button::with_label("Cancel");
    header.pack_start(&cancel);
    {
        let win = win.clone();
        cancel.connect_clicked(move |_| win.close());
    }

    let page = adw::PreferencesPage::new();

    let site = adw::PreferencesGroup::new();
    let url = adw::EntryRow::new();
    url.set_title("Website address");
    url.set_text(&app.borrow().url);
    url.set_input_purpose(gtk4::InputPurpose::Url);
    url.set_show_apply_button(true);
    let name = adw::EntryRow::new();
    name.set_title("Name");
    name.set_text(&app.borrow().name);
    let icon_row = adw::ActionRow::new();
    icon_row.set_title("Icon");
    let icon_img = std::rc::Rc::new(std::cell::RefCell::new(icon_image(&app.borrow().icon, 32)));
    icon_row.add_prefix(&*icon_img.borrow());
    let status = adw::ActionRow::new();
    status.set_visible(false);
    let choose = gtk4::Button::with_label("Choose…");
    choose.set_valign(gtk4::Align::Center);
    icon_row.add_suffix(&choose);
    site.add(&url);
    site.add(&name);
    site.add(&icon_row);
    site.add(&status);
    page.add(&site);

    let set_icon = {
        let (app, icon_row, icon_img) = (app.clone(), icon_row.clone(), icon_img.clone());
        move |path: String| {
            icon_row.remove(&*icon_img.borrow());
            let img = icon_image(&path, 32);
            icon_row.add_prefix(&img);
            *icon_img.borrow_mut() = img;
            app.borrow_mut().icon = path;
        }
    };

    // Fetch the site's title and icon when the address is confirmed.
    {
        let (name, status, set_icon) = (name.clone(), status.clone(), set_icon.clone());
        url.connect_apply(move |e| {
            let u = normalize_url(&e.text());
            e.set_text(&u);
            status.set_visible(true);
            status.set_title("Getting the site's name and icon…");
            let hint = slug(&u.split("://").nth(1).unwrap_or(&u).split('/').next().unwrap_or("site").to_string());
            let (name, status, set_icon) = (name.clone(), status.clone(), set_icon.clone());
            in_background(
                move || fetch_details(u, hint),
                move |(title, icon)| {
                    if let Some(t) = title {
                        if name.text().is_empty() {
                            name.set_text(&t);
                        }
                    }
                    if let Some(i) = icon {
                        set_icon(i);
                    }
                    status.set_visible(false);
                },
            );
        });
    }
    {
        let (win, set_icon) = (win.clone(), set_icon.clone());
        choose.connect_clicked(move |_| {
            let dialog = gtk4::FileDialog::new();
            dialog.set_title("Choose an icon");
            let filter = gtk4::FileFilter::new();
            filter.add_pixbuf_formats();
            filter.set_name(Some("Images"));
            let filters = gtk4::gio::ListStore::new::<gtk4::FileFilter>();
            filters.append(&filter);
            dialog.set_filters(Some(&filters));
            let set_icon = set_icon.clone();
            dialog.open(Some(&win), None::<&gtk4::gio::Cancellable>, move |res| {
                if let Some(p) = res.ok().and_then(|f| f.path()) {
                    set_icon(p.to_string_lossy().to_string());
                }
            });
        });
    }

    let behaviour = adw::PreferencesGroup::new();
    behaviour.set_title("Behaviour");
    let mode = adw::ComboRow::new();
    mode.set_title("Open as");
    mode.set_model(Some(&gtk4::StringList::new(&MODES)));
    mode.set_selected(app.borrow().mode);
    let startup = adw::SwitchRow::new();
    startup.set_title("Open when you sign in");
    startup.set_active(app.borrow().startup);
    let minimized = adw::SwitchRow::new();
    minimized.set_title("Start in the background");
    minimized.set_subtitle("Opens minimized at sign-in, so notifications keep working without a window in your way");
    minimized.set_active(app.borrow().start_minimized);
    minimized.set_sensitive(app.borrow().startup);
    {
        let minimized = minimized.clone();
        startup.connect_active_notify(move |s| minimized.set_sensitive(s.is_active()));
    }
    behaviour.add(&mode);
    behaviour.add(&startup);
    behaviour.add(&minimized);
    page.add(&behaviour);

    let perms = adw::PreferencesGroup::new();
    perms.set_title("Permissions");
    perms.set_description(Some("What this site may use. Close the app before changing these."));
    let perm_rows: Vec<adw::ComboRow> = PERMS
        .iter()
        .enumerate()
        .map(|(i, (_, label))| {
            let r = adw::ComboRow::new();
            r.set_title(label);
            r.set_model(Some(&gtk4::StringList::new(&PERM_CHOICES)));
            r.set_selected(app.borrow().perms.get(i).copied().unwrap_or(0));
            perms.add(&r);
            r
        })
        .collect();
    page.add(&perms);

    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&page));
    win.set_content(Some(&view));

    {
        let win2 = win.clone();
        save_btn.connect_clicked(move |b| {
            let url_text = normalize_url(&url.text());
            if url.text().trim().is_empty() || !url_text.contains('.') {
                message(&win2, "Enter a website address", "For example: web.whatsapp.com");
                return;
            }
            let name_text = name.text().trim().to_string();
            let mut a = app.borrow().clone();
            if is_new {
                a.id = slug(if name_text.is_empty() { &url_text } else { &name_text });
            } else if is_running(&a) {
                message(&win2, "Close the app first", &format!("{} is open. Close it, then save your changes.", a.name));
                return;
            }
            a.url = url_text.clone();
            a.name = if name_text.is_empty() { url_text } else { name_text };
            a.mode = mode.selected();
            a.startup = startup.is_active();
            a.start_minimized = minimized.is_active();
            a.perms = perm_rows.iter().map(|r| r.selected()).collect();
            b.set_sensitive(false);
            let (win3, on_saved, b2) = (win2.clone(), on_saved.clone(), b.clone());
            in_background(
                move || save(&a),
                move |res| match res {
                    Ok(()) => {
                        on_saved();
                        win3.close();
                    }
                    Err(e) => {
                        b2.set_sensitive(true);
                        message(&win3, "Web app not saved", &e);
                    }
                },
            );
        });
    }
    win.present();
}

pub fn open(parent: &gtk4::Widget) {
    let win = adw::Window::builder().default_width(640).default_height(560).title("Apps for websites").build();
    if let Some(p) = parent.root().and_downcast::<gtk4::Window>() {
        win.set_transient_for(Some(&p));
        win.set_modal(true);
    }
    let header = adw::HeaderBar::new();
    let add = gtk4::Button::from_icon_name("list-add-symbolic");
    add.set_tooltip_text(Some("New web app"));
    header.pack_start(&add);

    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::new();
    group.set_description(Some(
        "Web apps open in their own window with their own icon, sign-ins and permissions, separate from your browser.",
    ));
    page.add(&group);

    let rows: std::rc::Rc<std::cell::RefCell<Vec<gtk4::Widget>>> = Default::default();
    let refill: std::rc::Rc<std::cell::RefCell<Option<std::rc::Rc<dyn Fn()>>>> = Default::default();
    let fill: std::rc::Rc<dyn Fn()> = {
        let (group, rows, win, refill) = (group.clone(), rows.clone(), win.clone(), refill.clone());
        std::rc::Rc::new(move || {
            for r in rows.borrow_mut().drain(..) {
                group.remove(&r);
            }
            let again = refill.borrow().clone().expect("set");
            let apps = load_all();
            if apps.is_empty() {
                let r = adw::ActionRow::new();
                r.set_title("No web apps yet");
                r.set_subtitle("Select + to turn a website into an app");
                group.add(&r);
                rows.borrow_mut().push(r.upcast());
            }
            for a in apps {
                let r = adw::ActionRow::new();
                r.set_title(&glib::markup_escape_text(&a.name));
                r.set_subtitle(&glib::markup_escape_text(&a.url));
                r.add_prefix(&icon_image(&a.icon, 32));

                let open_btn = gtk4::Button::with_label("Open");
                open_btn.set_valign(gtk4::Align::Center);
                let a2 = a.clone();
                open_btn.connect_clicked(move |_| launch(&a2));
                r.add_suffix(&open_btn);

                let edit = gtk4::Button::from_icon_name("document-edit-symbolic");
                edit.add_css_class("flat");
                edit.set_valign(gtk4::Align::Center);
                edit.set_tooltip_text(Some("Edit"));
                let (a2, win2, again2) = (a.clone(), win.clone(), again.clone());
                edit.connect_clicked(move |_| editor(win2.upcast_ref(), Some(a2.clone()), again2.clone()));
                r.add_suffix(&edit);

                let del = gtk4::Button::from_icon_name("user-trash-symbolic");
                del.add_css_class("flat");
                del.set_valign(gtk4::Align::Center);
                del.set_tooltip_text(Some("Remove"));
                let (a2, win2, again2) = (a.clone(), win.clone(), again.clone());
                del.connect_clicked(move |_| {
                    let d = adw::AlertDialog::new(
                        Some(&format!("Remove {}?", a2.name)),
                        Some("Its sign-ins and data for this site are deleted too."),
                    );
                    d.add_responses(&[("cancel", "Cancel"), ("remove", "Remove")]);
                    d.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
                    let (a3, again3) = (a2.clone(), again2.clone());
                    d.connect_response(None, move |_, r| {
                        if r == "remove" {
                            let a4 = a3.clone();
                            let again4 = again3.clone();
                            in_background(move || remove(&a4), move |_| again4());
                        }
                    });
                    d.present(Some(&win2));
                });
                r.add_suffix(&del);
                group.add(&r);
                rows.borrow_mut().push(r.upcast());
            }
        })
    };
    *refill.borrow_mut() = Some(fill.clone());
    fill();

    if browser().is_none() {
        add.set_sensitive(false);
        group.set_description(Some("Web apps need Brave, Chromium, Chrome, Edge or Vivaldi. Install one from Zohara Store."));
    }
    {
        let (win2, fill) = (win.clone(), fill.clone());
        add.connect_clicked(move |_| editor(win2.upcast_ref(), None, fill.clone()));
    }

    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&page));
    win.set_content(Some(&view));
    win.present();
}
