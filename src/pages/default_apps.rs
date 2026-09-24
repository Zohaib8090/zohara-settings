//! Default apps: which app opens links, email, folders, pictures, video,
//! music, text, PDFs and maps, plus the terminal and any other file type.
//!
//! Uses GIO's MIME database (`mimeapps.list`, read by Plasma and GTK apps
//! alike) and also Plasma's own `kdeglobals` keys for the browser and
//! terminal, which KDE apps consult directly.

use crate::backend::kconfig;
use adw::prelude::*;
use gtk4::gio;
use gtk4::gio::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;

struct Category {
    title: &'static str,
    icon: &'static str,
    /// The first type decides the current default; all are set together.
    types: &'static [&'static str],
}

const CATEGORIES: &[Category] = &[
    Category { title: "Web browser", icon: "web-browser-symbolic", types: &["x-scheme-handler/https", "x-scheme-handler/http", "text/html", "application/xhtml+xml"] },
    Category { title: "Email", icon: "mail-unread-symbolic", types: &["x-scheme-handler/mailto"] },
    Category { title: "Files", icon: "system-file-manager-symbolic", types: &["inode/directory"] },
    Category { title: "Photos", icon: "image-x-generic-symbolic", types: &["image/jpeg", "image/png", "image/gif", "image/webp", "image/heif", "image/avif", "image/bmp", "image/tiff"] },
    Category { title: "Video", icon: "video-x-generic-symbolic", types: &["video/mp4", "video/x-matroska", "video/webm", "video/quicktime", "video/x-msvideo", "video/mpeg"] },
    Category { title: "Music", icon: "audio-x-generic-symbolic", types: &["audio/mpeg", "audio/flac", "audio/ogg", "audio/x-wav", "audio/mp4", "audio/aac", "audio/x-opus+ogg"] },
    Category { title: "Text files", icon: "text-x-generic-symbolic", types: &["text/plain", "text/markdown", "application/x-shellscript"] },
    Category { title: "PDF documents", icon: "x-office-document-symbolic", types: &["application/pdf"] },
    Category { title: "Archives", icon: "package-x-generic-symbolic", types: &["application/zip", "application/x-tar", "application/x-7z-compressed", "application/vnd.rar", "application/x-compressed-tar"] },
    Category { title: "Maps", icon: "find-location-symbolic", types: &["x-scheme-handler/geo"] },
];

fn same(a: &gio::AppInfo, b: &gio::AppInfo) -> bool {
    a.id().is_some() && a.id() == b.id()
}

/// Apps that can open every type in the list's first entry, deduplicated.
fn candidates(mime: &str) -> Vec<gio::AppInfo> {
    let mut out: Vec<gio::AppInfo> = Vec::new();
    for a in gio::AppInfo::all_for_type(mime) {
        if a.should_show() && !out.iter().any(|b| same(&a, b)) {
            out.push(a);
        }
    }
    out.sort_by_key(|a| a.display_name().to_lowercase());
    out
}

fn set_default(app: &gio::AppInfo, types: &[&str]) -> Result<(), String> {
    for t in types {
        app.set_as_default_for_type(t).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn message(w: &impl IsA<gtk4::Widget>, title: &str, body: &str) {
    let d = adw::AlertDialog::new(Some(title), Some(body));
    d.add_response("ok", "OK");
    d.present(Some(w));
}

/// Combo row listing `apps`, preselected to `current`, calling `on_pick` on change.
fn app_combo(title: &str, icon: &str, apps: Vec<gio::AppInfo>, current: Option<gio::AppInfo>, on_pick: impl Fn(&gio::AppInfo) + 'static) -> adw::ComboRow {
    let row = adw::ComboRow::new();
    row.set_title(title);
    row.add_prefix(&gtk4::Image::from_icon_name(icon));
    if apps.is_empty() {
        row.set_subtitle("No installed app can do this. Find one in Zohara Store");
        row.set_sensitive(false);
        return row;
    }
    let names: Vec<String> = apps.iter().map(|a| a.display_name().to_string()).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    row.set_model(Some(&gtk4::StringList::new(&refs)));
    // Show each app's icon next to its name in the dropdown.
    let factory = gtk4::SignalListItemFactory::new();
    {
        let apps = apps.clone();
        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk4::ListItem>().expect("list item");
            let b = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
            b.append(&gtk4::Image::new());
            b.append(&gtk4::Label::new(None));
            item.set_child(Some(&b));
        });
        factory.connect_bind(move |_, item| {
            let item = item.downcast_ref::<gtk4::ListItem>().expect("list item");
            let (Some(b), Some(app)) = (item.child().and_downcast::<gtk4::Box>(), apps.get(item.position() as usize)) else { return };
            if let Some(img) = b.first_child().and_downcast::<gtk4::Image>() {
                match app.icon() {
                    Some(i) => img.set_from_gicon(&i),
                    None => img.set_icon_name(Some("application-x-executable")),
                }
                img.set_pixel_size(20);
            }
            if let Some(lbl) = b.last_child().and_downcast::<gtk4::Label>() {
                lbl.set_label(&app.display_name());
            }
        });
    }
    row.set_factory(Some(&factory));
    match current.as_ref().and_then(|c| apps.iter().position(|a| same(a, c))) {
        Some(i) => row.set_selected(i as u32),
        None => {
            row.set_selected(gtk4::INVALID_LIST_POSITION);
            row.set_subtitle("Not set");
        }
    }
    row.connect_selected_notify(move |r| {
        if let Some(app) = apps.get(r.selected() as usize) {
            r.set_subtitle("");
            on_pick(app);
        }
    });
    row
}

fn category_row(c: &'static Category, page: &gtk4::Box) -> adw::ComboRow {
    let current = gio::AppInfo::default_for_type(c.types[0], false);
    let page = page.clone();
    app_combo(c.title, c.icon, candidates(c.types[0]), current, move |app| {
        if let Err(e) = set_default(app, c.types) {
            message(&page, "Default not changed", &e);
            return;
        }
        log::info!("default for {} set to {:?}", c.title, app.id());
        if c.title == "Web browser" {
            if let Some(id) = app.id() {
                let id = id.to_string();
                kconfig::spawn(move || kconfig::write("kdeglobals", &["General"], "BrowserApplication", &id));
            }
        }
    })
}

fn terminal_row() -> adw::ComboRow {
    let terms: Vec<gio::AppInfo> = gio::AppInfo::all()
        .into_iter()
        .filter(|a| a.should_show())
        .filter(|a| {
            a.downcast_ref::<gio::DesktopAppInfo>()
                .and_then(|d| d.categories())
                .map(|c| c.split(';').any(|x| x == "TerminalEmulator"))
                .unwrap_or(false)
        })
        .collect();
    let current_id = kconfig::read("kdeglobals", &["General"], "TerminalService").unwrap_or_else(|| "org.kde.konsole.desktop".into());
    let current = terms.iter().find(|a| a.id().map(|i| i == current_id).unwrap_or(false)).cloned();
    app_combo("Terminal", "utilities-terminal-symbolic", terms, current, |app| {
        let (Some(id), exe) = (app.id(), app.executable()) else { return };
        let (id, exe) = (id.to_string(), exe.to_string_lossy().to_string());
        kconfig::spawn(move || {
            kconfig::write("kdeglobals", &["General"], "TerminalService", &id);
            kconfig::write("kdeglobals", &["General"], "TerminalApplication", &exe);
        });
    })
}

/// "Choose by file type": type an extension or MIME type, pick the default for it.
fn by_type_group(page: &gtk4::Box) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("Other file types");
    g.set_description(Some("Type a file extension (like .odt or .csv) to choose the app that opens it"));
    let entry = adw::EntryRow::new();
    entry.set_title("File extension or type");
    entry.set_show_apply_button(true);
    g.add(&entry);
    let result: std::rc::Rc<std::cell::RefCell<Option<adw::ComboRow>>> = Default::default();
    let (g2, page2) = (g.clone(), page.clone());
    entry.connect_apply(move |e| {
        if let Some(old) = result.borrow_mut().take() {
            g2.remove(&old);
        }
        let text = e.text().trim().to_string();
        if text.is_empty() {
            return;
        }
        let mime = if text.contains('/') {
            text.clone()
        } else {
            let name = format!("file.{}", text.trim_start_matches('.'));
            let (guess, uncertain) = gio::content_type_guess(Some(name.as_str()), &[]);
            if uncertain && guess.as_str() == "application/octet-stream" {
                message(&page2, "Unknown file type", &format!("No file type is registered for “{text}”."));
                return;
            }
            guess.to_string()
        };
        let desc = gio::content_type_get_description(&mime).to_string();
        let title = format!("{desc} ({mime})");
        let current = gio::AppInfo::default_for_type(&mime, false);
        let (page3, mime2) = (page2.clone(), mime.clone());
        let row = app_combo(&title, "text-x-generic-symbolic", candidates(&mime), current, move |app| {
            if let Err(e) = set_default(app, &[mime2.as_str()]) {
                message(&page3, "Default not changed", &e);
            }
        });
        g2.add(&row);
        *result.borrow_mut() = Some(row);
    });
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
            .label("Default apps")
            .halign(gtk4::Align::Start)
            .css_classes(vec!["win11-page-title".to_string()])
            .build(),
    );

    let g = adw::PreferencesGroup::new();
    g.set_title("Open with");
    for c in CATEGORIES {
        g.add(&category_row(c, &root));
    }
    g.add(&terminal_row());
    root.append(&g);
    root.append(&by_type_group(&root));

    scroll.set_child(Some(&root));
    scroll.upcast()
}
