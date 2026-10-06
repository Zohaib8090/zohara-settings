//! Personalization > Themes: global look, colours, icons and cursors, plus installing a theme from a file.
//! Everything shown here is read from the system (Plasma's own `plasma-apply-*` tools and the icon folders), and
//! every change is applied with those same tools, so it matches what Plasma's own settings would do.

use crate::backend::{kconfig, worker::in_background};
use gtk4::prelude::*;
use libadwaita as adw;
use adw::prelude::*;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;

/// One choice in a list: what is applied, what is shown, and whether it is in use right now.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub id: String,
    pub label: String,
    pub current: bool,
}

#[derive(Default, Clone)]
struct Themes {
    look: Vec<Choice>,
    colors: Vec<Choice>,
    icons: Vec<Choice>,
    cursors: Vec<Choice>,
}

/// What an archive holds, judged from its file names.
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Kind {
    Icons,
    Colors,
    LookAndFeel,
    Unknown,
}

pub fn classify_archive(entries: &[String]) -> Kind {
    let ends = |s: &str| entries.iter().any(|e| e.trim_end_matches('/').ends_with(s));
    if ends("index.theme") {
        Kind::Icons
    } else if ends(".colors") {
        Kind::Colors
    } else if ends("metadata.json") || ends("metadata.desktop") {
        Kind::LookAndFeel
    } else {
        Kind::Unknown
    }
}

/// Output like `You have the following ...:\n * Name (current color scheme)\n * Other` -> choices.
/// Lines that do not start with a star are headings and are ignored.
pub fn parse_star_list(out: &str) -> Vec<Choice> {
    out.lines()
        .filter_map(|l| {
            let l = l.trim_start();
            let rest = l.strip_prefix("* ").or_else(|| l.strip_prefix("*"))?.trim();
            if rest.is_empty() {
                return None;
            }
            let (name, tail) = match rest.find(" (") {
                Some(i) => (&rest[..i], &rest[i..]),
                None => (rest, ""),
            };
            let name = name.trim();
            Some(Choice { id: name.to_string(), label: name.to_string(), current: tail.contains("current") })
        })
        .collect()
}

/// `plasma-apply-lookandfeel --list` prints one id per line (and a heading in some versions).
pub fn parse_lookandfeel(out: &str, current: &str) -> Vec<Choice> {
    out.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.contains(' ') && !l.ends_with(':'))
        .map(|id| Choice { id: id.to_string(), label: pretty_look(id), current: id == current })
        .collect()
}

/// "org.kde.breezedark.desktop" -> "Breezedark".
fn pretty_look(id: &str) -> String {
    let last = id.rsplit('.').find(|p| *p != "desktop").unwrap_or(id);
    let mut c = last.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_else(|| id.to_string())
}

/// Reads `Name=` from an icon theme's index.theme, and tells whether it is an icon theme (has `Directories=`)
/// as opposed to a cursor-only theme.
pub fn parse_index_theme(text: &str) -> (Option<String>, bool, bool) {
    let mut name = None;
    let mut dirs = false;
    let mut hidden = false;
    let mut in_icon_theme = false;
    for l in text.lines() {
        let l = l.trim();
        if l.starts_with('[') {
            in_icon_theme = l == "[Icon Theme]";
            continue;
        }
        if !in_icon_theme {
            continue;
        }
        if let Some(v) = l.strip_prefix("Name=") {
            if name.is_none() {
                name = Some(v.trim().to_string());
            }
        } else if l.starts_with("Directories=") {
            dirs = true;
        } else if l.eq_ignore_ascii_case("Hidden=true") {
            hidden = true;
        }
    }
    (name, dirs, hidden)
}

fn icon_dirs() -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    vec![PathBuf::from("/usr/share/icons"), PathBuf::from(format!("{home}/.local/share/icons")), PathBuf::from(format!("{home}/.icons"))]
}

fn installed_icon_themes(current: &str) -> Vec<Choice> {
    let mut out: Vec<Choice> = Vec::new();
    for dir in icon_dirs() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let id = e.file_name().to_string_lossy().to_string();
            let Ok(text) = std::fs::read_to_string(e.path().join("index.theme")) else { continue };
            let (name, dirs, hidden) = parse_index_theme(&text);
            if !dirs || hidden || id == "default" || out.iter().any(|c| c.id == id) {
                continue;
            }
            out.push(Choice { current: id == current, label: name.unwrap_or_else(|| id.clone()), id });
        }
    }
    out.sort_by(|a, b| a.label.to_lowercase().cmp(&b.label.to_lowercase()));
    out
}

fn run_stdout(cmd: &str, args: &[&str]) -> String {
    Command::new(cmd).args(args).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default()
}

fn load() -> Themes {
    let cur_look = kconfig::read("kdeglobals", &["KDE"], "LookAndFeelPackage").unwrap_or_default();
    let cur_icons = kconfig::read("kdeglobals", &["Icons"], "Theme").unwrap_or_default();
    Themes {
        look: parse_lookandfeel(&run_stdout("plasma-apply-lookandfeel", &["--list"]), &cur_look),
        colors: parse_star_list(&run_stdout("plasma-apply-colorscheme", &["--list-schemes"])),
        icons: installed_icon_themes(&cur_icons),
        cursors: parse_star_list(&run_stdout("plasma-apply-cursortheme", &["--list-themes"])),
    }
}

/// Applies one choice. Returns an error text when the tool is missing or refuses.
fn apply(kind: &str, id: &str) -> Result<(), String> {
    let run = |cmd: &str, args: &[&str]| -> Result<(), String> {
        let o = Command::new(cmd).args(args).output().map_err(|_| format!("{cmd} is not installed"))?;
        if o.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&o.stderr).trim().to_string())
        }
    };
    match kind {
        "look" => run("plasma-apply-lookandfeel", &["-a", id]),
        "colors" => run("plasma-apply-colorscheme", &[id]),
        "cursors" => run("plasma-apply-cursortheme", &[id]),
        "icons" => {
            // The same two steps Plasma's own icon page does: store the name, tell running apps.
            kconfig::write("kdeglobals", &["Icons"], "Theme", id);
            let _ = Command::new("dbus-send")
                .args(["--session", "--type=signal", "/KIconLoader", "org.kde.KIconLoader.iconChanged", "int32:0"])
                .status();
            Ok(())
        }
        _ => Err("unknown kind".into()),
    }
}

/// Installs a theme file. Returns what was installed, in words.
fn install_file(path: &Path) -> Result<String, String> {
    let home = std::env::var("HOME").map_err(|_| "no home folder".to_string())?;
    let p = path.to_string_lossy().to_string();
    let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
    if p.ends_with(".colors") {
        let dir = format!("{home}/.local/share/color-schemes");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        std::fs::copy(path, format!("{dir}/{name}")).map_err(|e| e.to_string())?;
        return Ok(format!("Colour scheme {name} installed"));
    }
    let list = Command::new("bsdtar").args(["-tf", &p]).output().map_err(|_| "Can't open archives here (bsdtar is missing)".to_string())?;
    if !list.status.success() {
        return Err("That file isn't a theme archive I can open".into());
    }
    let entries: Vec<String> = String::from_utf8_lossy(&list.stdout).lines().map(str::to_string).collect();
    match classify_archive(&entries) {
        Kind::Icons => {
            let dir = format!("{home}/.local/share/icons");
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let o = Command::new("bsdtar").args(["-xf", &p, "-C", &dir]).output().map_err(|e| e.to_string())?;
            if o.status.success() { Ok("Icon or cursor theme installed".into()) } else { Err(String::from_utf8_lossy(&o.stderr).trim().to_string()) }
        }
        Kind::Colors => {
            let dir = format!("{home}/.local/share/color-schemes");
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let o = Command::new("bsdtar").args(["-xf", &p, "-C", &dir, "--strip-components=0", "--include", "*.colors"]).output().map_err(|e| e.to_string())?;
            if o.status.success() { Ok("Colour scheme installed".into()) } else { Err(String::from_utf8_lossy(&o.stderr).trim().to_string()) }
        }
        Kind::LookAndFeel => {
            let o = Command::new("kpackagetool6").args(["--type", "Plasma/LookAndFeel", "--install", &p]).output().map_err(|_| "kpackagetool6 is missing".to_string())?;
            if o.status.success() {
                Ok("Global theme installed".into())
            } else {
                // Already installed: upgrade it instead.
                let u = Command::new("kpackagetool6").args(["--type", "Plasma/LookAndFeel", "--upgrade", &p]).output().map_err(|e| e.to_string())?;
                if u.status.success() { Ok("Global theme installed".into()) } else { Err(String::from_utf8_lossy(&o.stderr).trim().to_string()) }
            }
        }
        Kind::Unknown => Err("I couldn't tell what kind of theme this is. Icon, cursor, colour and global themes are supported".into()),
    }
}

fn choice_row(title: &str, subtitle: &str, kind: &'static str, items: &[Choice], status: &gtk4::Label, auto: bool) -> adw::ComboRow {
    let row = adw::ComboRow::new();
    row.set_title(title);
    row.set_subtitle(subtitle);
    let labels: Vec<&str> = items.iter().map(|c| c.label.as_str()).collect();
    row.set_model(Some(&gtk4::StringList::new(&labels)));
    row.set_enable_search(true);
    if let Some(i) = items.iter().position(|c| c.current) {
        row.set_selected(i as u32);
    }
    if auto {
        let ready = Rc::new(std::cell::Cell::new(false));
        let (items, status, r2) = (items.to_vec(), status.clone(), ready.clone());
        row.connect_selected_notify(move |r| {
            if !r2.get() {
                return;
            }
            let Some(c) = items.get(r.selected() as usize).cloned() else { return };
            status.set_text(&format!("Applying {}…", c.label));
            let status = status.clone();
            in_background(move || (apply(kind, &c.id), c.label), move |(res, label)| {
                status.set_text(&match res {
                    Ok(()) => format!("Applied: {label}"),
                    Err(e) => format!("Couldn't apply {label}: {e}"),
                });
            });
        });
        ready.set(true);
    }
    row
}

/// Opens the Themes window over `parent_row`'s window.
pub fn open(parent_row: &adw::ActionRow) {
    let content = super::personalization::dialog_content_box();
    let spinner = gtk4::Spinner::new();
    spinner.start();
    spinner.set_margin_top(24);
    content.append(&spinner);
    super::personalization::open_settings_window_sized(parent_row, "Themes", &content, 560, 680);
    fill(content);
}

fn fill(content: gtk4::Box) {
    in_background(load, move |t| {
        while let Some(c) = content.first_child() {
            content.remove(&c);
        }
        let status = gtk4::Label::new(Some("Pick one and it is applied right away."));
        status.set_halign(gtk4::Align::Start);
        status.set_wrap(true);
        status.add_css_class("dim-label");

        let g = adw::PreferencesGroup::new();
        g.set_title("Appearance");
        let mut any = false;
        if !t.look.is_empty() {
            any = true;
            let row = choice_row("Global theme", "Colours, icons, cursor and window style together", "look", &t.look, &status, false);
            let apply_btn = gtk4::Button::with_label("Apply");
            apply_btn.set_valign(gtk4::Align::Center);
            apply_btn.add_css_class("suggested-action");
            let (row2, items, status2) = (row.clone(), t.look.clone(), status.clone());
            apply_btn.connect_clicked(move |_| {
                let Some(c) = items.get(row2.selected() as usize).cloned() else { return };
                status2.set_text(&format!("Applying {}…", c.label));
                let status3 = status2.clone();
                in_background(move || (apply("look", &c.id), c.label), move |(res, label)| {
                    status3.set_text(&match res { Ok(()) => format!("Applied: {label}"), Err(e) => format!("Couldn't apply {label}: {e}") });
                });
            });
            row.add_suffix(&apply_btn);
            g.add(&row);
        }
        if !t.colors.is_empty() {
            any = true;
            g.add(&choice_row("Colour scheme", "The colours of windows and buttons", "colors", &t.colors, &status, true));
        }
        if !t.icons.is_empty() {
            any = true;
            g.add(&choice_row("Icons", "The icon set used by apps and the desktop", "icons", &t.icons, &status, true));
        }
        if !t.cursors.is_empty() {
            any = true;
            g.add(&choice_row("Mouse pointer", "The look of the cursor", "cursors", &t.cursors, &status, true));
        }
        if any {
            content.append(&g);
        } else {
            let msg = gtk4::Label::new(Some("No themes were found. Plasma's theme tools (plasma-apply-lookandfeel and friends) are missing, or no themes are installed."));
            msg.set_wrap(true);
            msg.set_halign(gtk4::Align::Start);
            content.append(&msg);
        }
        content.append(&status);

        let inst = adw::PreferencesGroup::new();
        inst.set_title("Add more themes");
        inst.set_description(Some("Icon, cursor, colour and global themes are supported."));
        let from_file = adw::ActionRow::new();
        from_file.set_title("Install a theme from a file");
        from_file.set_subtitle("A .tar.gz, .zip or .colors file you downloaded");
        let choose = gtk4::Button::with_label("Choose file…");
        choose.set_valign(gtk4::Align::Center);
        let (content_c, status_c) = (content.clone(), status.clone());
        choose.connect_clicked(move |btn| {
            let parent = btn.root().and_downcast::<gtk4::Window>();
            let dialog = gtk4::FileDialog::new();
            dialog.set_title("Choose a theme file");
            let (content_d, status_d) = (content_c.clone(), status_c.clone());
            dialog.open(parent.as_ref(), None::<&gtk4::gio::Cancellable>, move |res| {
                let Ok(file) = res else { return };
                let Some(path) = file.path() else { return };
                status_d.set_text("Installing…");
                let (content_e, status_e) = (content_d.clone(), status_d.clone());
                in_background(move || install_file(&path), move |res| match res {
                    Ok(msg) => {
                        status_e.set_text(&format!("{msg}. Pick it from the lists above."));
                        // Reload the lists so the new theme shows up.
                        fill(content_e);
                    }
                    Err(e) => status_e.set_text(&format!("Not installed: {e}")),
                });
            });
        });
        from_file.add_suffix(&choose);
        inst.add(&from_file);

        let online = adw::ActionRow::new();
        online.set_title("Get more themes online");
        online.set_subtitle("Browse and download themes from the KDE Store");
        let open_btn = gtk4::Button::with_label("Open");
        open_btn.set_valign(gtk4::Align::Center);
        open_btn.connect_clicked(|_| {
            // Plasma's own page has a "Get New..." button; fall back to the website.
            if Command::new("kcmshell6").arg("kcm_lookandfeel").spawn().is_err() {
                let _ = Command::new("xdg-open").arg("https://store.kde.org/browse?cat=100").spawn();
            }
        });
        online.add_suffix(&open_btn);
        inst.add(&online);

        let reload = adw::ActionRow::new();
        reload.set_title("Reload the lists");
        reload.set_subtitle("Use this after installing a theme some other way");
        let rb = gtk4::Button::from_icon_name("view-refresh-symbolic");
        rb.set_valign(gtk4::Align::Center);
        rb.set_tooltip_text(Some("Reload the lists"));
        let content_r = content.clone();
        rb.connect_clicked(move |_| fill(content_r.clone()));
        reload.add_suffix(&rb);
        inst.add(&reload);
        content.append(&inst);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn star_list_marks_the_current_one() {
        let out = "You have the following color schemes:\n * BreezeDark (current color scheme)\n * BreezeLight\n * Honeycomb\n";
        let v = parse_star_list(out);
        assert_eq!(v.len(), 3);
        assert_eq!(v[0], Choice { id: "BreezeDark".into(), label: "BreezeDark".into(), current: true });
        assert!(!v[1].current);
    }

    #[test]
    fn star_list_ignores_headings_and_blank_lines() {
        assert!(parse_star_list("You have the following themes:\n\n").is_empty());
    }

    #[test]
    fn lookandfeel_ids_get_readable_names() {
        let v = parse_lookandfeel("org.kde.breezedark.desktop\norg.kde.breeze.desktop\n", "org.kde.breeze.desktop");
        assert_eq!(v[0].label, "Breezedark");
        assert!(v[1].current && !v[0].current);
    }

    #[test]
    fn archives_are_told_apart() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(classify_archive(&s(&["Tela/", "Tela/index.theme", "Tela/16/x.svg"])), Kind::Icons);
        assert_eq!(classify_archive(&s(&["Nice.colors"])), Kind::Colors);
        assert_eq!(classify_archive(&s(&["x/metadata.json", "x/contents/defaults"])), Kind::LookAndFeel);
        assert_eq!(classify_archive(&s(&["readme.txt"])), Kind::Unknown);
    }

    #[test]
    fn cursor_only_themes_are_not_icon_themes() {
        let cursor = "[Icon Theme]\nName=Bibata\nInherits=hicolor\n";
        let icons = "[Icon Theme]\nName=Tela\nDirectories=16,22\n";
        assert_eq!(parse_index_theme(cursor), (Some("Bibata".into()), false, false));
        assert_eq!(parse_index_theme(icons), (Some("Tela".into()), true, false));
    }
}
