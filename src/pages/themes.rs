//! Personalization > Themes: global look, colours, icons and cursors, plus installing a theme from a file.
//! Everything shown here is read from the system (Plasma's own `plasma-apply-*` tools and the icon folders), and
//! every change is applied with those same tools, so it matches what Plasma's own settings would do.

use super::look_reset;
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

/// `plasma-apply-cursortheme --list-themes` prints "Breeze Dark [breeze_cursors]": the name to show, then the id the
/// tool wants in brackets.
pub fn parse_cursors(out: &str) -> Vec<Choice> {
    parse_star_list(out)
        .into_iter()
        .map(|mut c| {
            if let (Some(open), true) = (c.id.rfind(" ["), c.id.ends_with(']')) {
                let id = c.id[open + 2..c.id.len() - 1].to_string();
                c.label = c.id[..open].trim().to_string();
                c.id = id;
            }
            c
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
        cursors: parse_cursors(&run_stdout("plasma-apply-cursortheme", &["--list-themes"])),
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
            // KIconLoader groups: Desktop, Toolbar, MainToolbar, Small, Panel, Dialog. The panel (taskbar) listens to
            // its own group, so tell all of them or the taskbar keeps the old icons until the next login.
            for group in 0..=5 {
                let _ = Command::new("dbus-send")
                    .args(["--session", "--type=signal", "/KIconLoader", "org.kde.KIconLoader.iconChanged", &format!("int32:{group}")])
                    .status();
            }
            Ok(())
        }
        _ => Err("unknown kind".into()),
    }
}

/// The look Zohara OS ships with (from the ISO's `/etc/skel`: kdeglobals and Kvantum config).
pub const DEFAULT_LOOK: &str = "org.kde.breezedark.desktop";
pub const DEFAULT_COLORS: &str = "BreezeDark";
pub const DEFAULT_ICONS: &str = "Fluent-dark";
pub const DEFAULT_CURSOR: &str = "breeze_cursors";
pub const DEFAULT_WIDGET_STYLE: &str = "kvantum-dark";
pub const DEFAULT_KVANTUM: &str = "MateriaDark";

/// Puts back everything a theme can change, and says what it could not do. Returns the list of problems.
fn reset_to_default(plan: look_reset::PanelPlan, restore_button: bool) -> Vec<String> {
    let mut problems = Vec::new();
    let mut note = |what: &str, r: Result<(), String>| {
        if let Err(e) = r {
            problems.push(format!("{what}: {e}"));
        }
    };
    // The global theme first: it sets many things at once, and the steps below then put the exact defaults back.
    note("Global theme", apply("look", DEFAULT_LOOK));
    note("Colours", apply("colors", DEFAULT_COLORS));
    if installed_icon_themes("").iter().any(|c| c.id == DEFAULT_ICONS) {
        note("Icons", apply("icons", DEFAULT_ICONS));
    } else {
        // The icon set shipped with the OS was removed: fall back to Breeze rather than leave a broken one.
        note("Icons", apply("icons", "breeze-dark"));
    }
    note("Pointer", apply("cursors", DEFAULT_CURSOR));
    // Widget style (Kvantum) and the window decoration and Plasma style a theme may have replaced.
    kconfig::write("kdeglobals", &["KDE"], "widgetStyle", DEFAULT_WIDGET_STYLE);
    let home = std::env::var("HOME").unwrap_or_default();
    let kv = format!("{home}/.config/Kvantum");
    let _ = std::fs::create_dir_all(&kv);
    let _ = std::fs::write(format!("{kv}/kvantum.kvconfig"), format!("[General]\ntheme={DEFAULT_KVANTUM}\n"));
    kconfig::delete("plasmarc", &["Theme"], "name");
    kconfig::delete("kwinrc", &["org.kde.kdecoration2"], "theme");
    kconfig::delete("kwinrc", &["org.kde.kdecoration2"], "library");
    kconfig::kwin_reconfigure();
    // The taskbar: extra panels a theme added are removed (the one with the start menu stays), then the bar goes back to
    // the bottom with the icons at the left, and the keyboard button comes back if it was there.
    problems.extend(look_reset::apply_panel_plan(&plan));
    if !super::personalization::run_taskbar_script(0, Some("bottom")) {
        problems.push("Taskbar: Plasma did not answer".to_string());
    }
    if restore_button {
        crate::backend::touch_keyboard::set_button(true);
    }
    problems
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

        let reset = adw::PreferencesGroup::new();
        reset.set_title("Something looks wrong?");
        let rrow = adw::ActionRow::new();
        rrow.set_title("Reset to the Zohara default look");
        rrow.set_subtitle("Puts back the theme, colours, icons, pointer, window style and taskbar that Zohara OS comes with. Your files and apps are not touched.");
        let rbtn = gtk4::Button::with_label("Reset…");
        rbtn.set_valign(gtk4::Align::Center);
        rbtn.add_css_class("destructive-action");
        let (content_x, status_x) = (content.clone(), status.clone());
        rbtn.connect_clicked(move |b| {
            let (content_y, status_y) = (content_x.clone(), status_x.clone());
            look_reset::open_review(b, move |choice| {
                status_y.set_text("Resetting…");
                let (content_z, status_z) = (content_y.clone(), status_y.clone());
                in_background(
                    move || {
                        let had_button = crate::backend::touch_keyboard::button_present();
                        reset_to_default(choice.plan, had_button)
                    },
                    move |problems| {
                        if problems.is_empty() {
                            status_z.set_text("Done: the default look is back. Some open apps change when they are reopened.");
                        } else {
                            status_z.set_text(&format!("Mostly done. Couldn't do: {}", problems.join("; ")));
                        }
                        fill(content_z);
                    },
                );
            });
        });
        rrow.add_suffix(&rbtn);
        reset.add(&rrow);
        content.append(&reset);

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
    fn cursor_names_and_ids_are_split() {
        let v = parse_cursors("You have the following themes:\n * Breeze Dark [breeze_cursors] (current theme for cursors)\n * Fluent [Fluent-cursors]\n");
        assert_eq!(v[0], Choice { id: "breeze_cursors".into(), label: "Breeze Dark".into(), current: true });
        assert_eq!(v[1].id, "Fluent-cursors");
        assert_eq!(v[1].label, "Fluent");
    }

    #[test]
    fn the_default_look_matches_what_the_iso_ships() {
        // Keep in step with zohara-profile/airootfs/etc/skel/.config/kdeglobals and Kvantum/kvantum.kvconfig.
        assert_eq!(DEFAULT_LOOK, "org.kde.breezedark.desktop");
        assert_eq!(DEFAULT_ICONS, "Fluent-dark");
        assert_eq!(DEFAULT_WIDGET_STYLE, "kvantum-dark");
        assert_eq!(DEFAULT_KVANTUM, "MateriaDark");
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
