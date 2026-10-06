//! Gaming > My games: games that were not installed from a store. A game is a desktop entry in
//! `~/.local/share/applications` with `Categories=Game;` (so it shows under Games in the Start menu) and the marker
//! `X-Zohara-Game=true` (so this page can list and remove only what it added). Two ways to add one: pick a program file
//! (a native program, an AppImage or a Windows .exe), or mark an app that is already installed as a game.

use crate::backend::worker::in_background;
use adw::prelude::*;
use gtk4::gio;
use gtk4::prelude::*;
use libadwaita as adw;
use std::path::{Path, PathBuf};
use std::process::Command;

const MARK: &str = "X-Zohara-Game=true";

fn apps_dir() -> PathBuf {
    let data = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share"));
    data.join("applications")
}

/// "Half-Life 2: Remastered!" -> "half-life-2-remastered".
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

/// A path as one word in an `Exec=` line: quoted, with the characters the spec escapes.
pub fn exec_quote(p: &str) -> String {
    let mut s = String::from("\"");
    for c in p.chars() {
        match c {
            '"' | '`' | '$' | '\\' => {
                s.push('\\');
                s.push(c);
            }
            '%' => s.push_str("%%"),
            _ => s.push(c),
        }
    }
    s.push('"');
    s
}

/// What starts a program file: a Windows .exe needs Wine, anything else runs as it is.
pub fn launch_command(path: &str, gamemode: bool) -> String {
    let base = if path.to_lowercase().ends_with(".exe") { format!("wine {}", exec_quote(path)) } else { exec_quote(path) };
    if gamemode { format!("gamemoderun {base}") } else { base }
}

/// The desktop entry for a program file.
pub fn desktop_entry(name: &str, path: &str, gamemode: bool) -> String {
    let dir = Path::new(path).parent().map(|d| d.to_string_lossy().to_string()).unwrap_or_default();
    let mut s = String::from("[Desktop Entry]\nType=Application\n");
    s.push_str(&format!("Name={}\n", name.replace('\n', " ")));
    s.push_str(&format!("Exec={}\n", launch_command(path, gamemode)));
    if !dir.is_empty() {
        s.push_str(&format!("Path={dir}\n"));
    }
    s.push_str("Icon=applications-games\nCategories=Game;\nTerminal=false\n");
    s.push_str(MARK);
    s.push('\n');
    s
}

/// An installed app's own desktop entry with `Game` added to its categories and the marker added. Only the
/// `[Desktop Entry]` section is touched; its actions keep working.
pub fn mark_as_game(original: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let (mut in_main, mut done_cat, mut done_mark) = (false, false, false);
    let finish_main = |out: &mut Vec<String>, done_cat: &mut bool, done_mark: &mut bool| {
        if !*done_cat {
            out.push("Categories=Game;".into());
            *done_cat = true;
        }
        if !*done_mark {
            out.push(MARK.into());
            *done_mark = true;
        }
    };
    for l in original.lines() {
        let t = l.trim();
        if t.starts_with('[') {
            if in_main {
                finish_main(&mut out, &mut done_cat, &mut done_mark);
            }
            in_main = t == "[Desktop Entry]";
        }
        if in_main && t.starts_with("Categories=") {
            let cur = t.trim_start_matches("Categories=");
            let has = cur.split(';').any(|c| c == "Game");
            let joined = if has { cur.to_string() } else { format!("Game;{}", cur.trim_start_matches(';')) };
            out.push(format!("Categories={joined}"));
            done_cat = true;
        } else if in_main && t == MARK {
            out.push(l.to_string());
            done_mark = true;
        } else {
            out.push(l.to_string());
        }
    }
    if in_main {
        finish_main(&mut out, &mut done_cat, &mut done_mark);
    }
    out.join("\n") + "\n"
}

pub fn is_marked(text: &str) -> bool {
    text.lines().any(|l| l.trim() == MARK)
}

fn key_of(text: &str, key: &str) -> Option<String> {
    let mut in_main = false;
    for l in text.lines() {
        let t = l.trim();
        if t.starts_with('[') {
            in_main = t == "[Desktop Entry]";
        } else if in_main {
            if let Some(v) = t.strip_prefix(&format!("{key}=")) {
                return Some(v.to_string());
            }
        }
    }
    None
}

#[derive(Debug, Clone, PartialEq)]
pub struct Game {
    pub file: PathBuf,
    pub name: String,
    pub icon: String,
}

/// The games this page added.
pub fn my_games() -> Vec<Game> {
    let mut v: Vec<Game> = Vec::new();
    let Ok(rd) = std::fs::read_dir(apps_dir()) else { return v };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().map_or(true, |x| x != "desktop") {
            continue;
        }
        let Ok(t) = std::fs::read_to_string(&p) else { continue };
        if !is_marked(&t) {
            continue;
        }
        let name = key_of(&t, "Name").unwrap_or_else(|| p.file_stem().unwrap_or_default().to_string_lossy().to_string());
        v.push(Game { file: p, name, icon: key_of(&t, "Icon").unwrap_or_else(|| "applications-games".into()) });
    }
    v.sort_by_key(|g| g.name.to_lowercase());
    v
}

fn refresh_menu() {
    let _ = Command::new("update-desktop-database").arg(apps_dir()).status();
    let _ = Command::new("kbuildsycoca6").status();
}

fn add_program(name: &str, path: &str, gamemode: bool) -> Result<(), String> {
    let p = Path::new(path);
    if !p.is_file() {
        return Err("That file doesn't exist".into());
    }
    if name.trim().is_empty() {
        return Err("Give the game a name".into());
    }
    // An AppImage or a program needs the executable bit; Wine runs a .exe without it.
    if !path.to_lowercase().ends_with(".exe") {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = std::fs::metadata(p).map_err(|e| e.to_string())?.permissions();
        if perm.mode() & 0o111 == 0 {
            perm.set_mode(perm.mode() | 0o755);
            std::fs::set_permissions(p, perm).map_err(|e| format!("Can't make it runnable: {e}"))?;
        }
    }
    std::fs::create_dir_all(apps_dir()).map_err(|e| e.to_string())?;
    let id = slug(name);
    if id.is_empty() {
        return Err("Use letters or numbers in the name".into());
    }
    let file = apps_dir().join(format!("zohara-game-{id}.desktop"));
    std::fs::write(&file, desktop_entry(name.trim(), path, gamemode)).map_err(|e| e.to_string())?;
    refresh_menu();
    Ok(())
}

fn mark_installed(info: &gio::DesktopAppInfo) -> Result<(), String> {
    let src = info.filename().ok_or("This app has no desktop file")?;
    let id = src.file_name().ok_or("bad name")?.to_string_lossy().to_string();
    let text = std::fs::read_to_string(&src).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(apps_dir()).map_err(|e| e.to_string())?;
    std::fs::write(apps_dir().join(&id), mark_as_game(&text)).map_err(|e| e.to_string())?;
    refresh_menu();
    Ok(())
}

/// The group shown on the Gaming page. It rebuilds itself after every change.
pub fn build() -> gtk4::Box {
    let holder = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    fill(&holder);
    holder
}

fn fill(holder: &gtk4::Box) {
    while let Some(c) = holder.first_child() {
        holder.remove(&c);
    }
    let g = adw::PreferencesGroup::new();
    g.set_title("My games");
    g.set_description(Some("Games that didn't come from a store. Added games show up under Games in the Start menu."));

    let from_apps = gtk4::Button::with_label("Add from my apps");
    from_apps.add_css_class("suggested-action");
    from_apps.set_valign(gtk4::Align::Center);
    from_apps.set_tooltip_text(Some("Pick an app you already installed and mark it as a game"));
    let h = holder.clone();
    from_apps.connect_clicked(move |b| pick_installed_window(b, &h, None));
    let add = gtk4::Button::with_label("Add a game file");
    add.set_valign(gtk4::Align::Center);
    add.set_tooltip_text(Some("Choose a game's program file, AppImage or Windows .exe"));
    let h = holder.clone();
    add.connect_clicked(move |b| add_window(b, &h));
    let buttons = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    buttons.append(&from_apps);
    buttons.append(&add);
    g.set_header_suffix(Some(&buttons));

    let games = my_games();
    if games.is_empty() {
        let r = adw::ActionRow::new();
        r.set_title("No games added yet");
        r.set_subtitle("Press Add from my apps to pick an app you installed, or Add a game file for a program you downloaded.");
        g.add(&r);
    }
    for game in games {
        let r = adw::ActionRow::new();
        r.set_title(&glib::markup_escape_text(&game.name));
        r.add_prefix(&gtk4::Image::from_icon_name(&game.icon));
        let play = gtk4::Button::with_label("Play");
        play.add_css_class("suggested-action");
        play.set_valign(gtk4::Align::Center);
        let f = game.file.clone();
        play.connect_clicked(move |_| {
            if let Some(info) = gio::DesktopAppInfo::from_filename(&f) {
                let _ = info.launch(&[], None::<&gio::AppLaunchContext>);
            }
        });
        let rm = gtk4::Button::from_icon_name("user-trash-symbolic");
        rm.set_valign(gtk4::Align::Center);
        rm.set_tooltip_text(Some("Remove from My games"));
        rm.update_property(&[gtk4::accessible::Property::Label("Remove from My games")]);
        let (f, h, name) = (game.file.clone(), holder.clone(), game.name.clone());
        rm.connect_clicked(move |b| {
            let d = adw::AlertDialog::new(Some(&format!("Remove {name}?")), Some("This only removes it from My games and the Start menu's Games list. The game itself stays installed."));
            d.add_response("cancel", "Cancel");
            d.add_response("remove", "Remove");
            d.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
            let (f, h) = (f.clone(), h.clone());
            d.connect_response(None, move |_, resp| {
                if resp == "remove" {
                    let _ = std::fs::remove_file(&f);
                    refresh_menu();
                    fill(&h);
                }
            });
            d.present(Some(b));
        });
        r.add_suffix(&play);
        r.add_suffix(&rm);
        g.add(&r);
    }
    holder.append(&g);
}

fn add_window(from: &impl IsA<gtk4::Widget>, holder: &gtk4::Box) {
    let Some(parent) = from.root().and_downcast::<gtk4::Window>() else { return };
    let win = gtk4::Window::builder().title("Add a game").transient_for(&parent).modal(true).default_width(520).build();
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 14);
    content.set_margin_top(18);
    content.set_margin_bottom(18);
    content.set_margin_start(18);
    content.set_margin_end(18);

    // 1. A program file.
    let file_group = adw::PreferencesGroup::new();
    file_group.set_title("A game's program file");
    file_group.set_description(Some("A program, an AppImage, or a Windows .exe (needs Wine)."));
    let name_row = adw::EntryRow::new();
    name_row.set_title("Name");
    let file_row = adw::ActionRow::new();
    file_row.set_title("Program file");
    file_row.set_subtitle("Nothing chosen yet");
    let choose = gtk4::Button::with_label("Choose…");
    choose.set_valign(gtk4::Align::Center);
    file_row.add_suffix(&choose);
    let gm = adw::SwitchRow::new();
    gm.set_title("Use Game Mode");
    gm.set_subtitle("Give the game priority while it runs");
    let has_gm = Command::new("sh").args(["-c", "command -v gamemoderun"]).output().map(|o| o.status.success()).unwrap_or(false);
    gm.set_active(has_gm);
    gm.set_sensitive(has_gm);
    if !has_gm {
        gm.set_subtitle("Game Mode isn't installed");
    }
    file_group.add(&name_row);
    file_group.add(&file_row);
    file_group.add(&gm);
    content.append(&file_group);

    let status = gtk4::Label::new(None);
    status.set_wrap(true);
    status.set_halign(gtk4::Align::Start);
    status.add_css_class("dim-label");

    let chosen: std::rc::Rc<std::cell::RefCell<Option<PathBuf>>> = Default::default();
    {
        let (file_row, name_row, chosen) = (file_row.clone(), name_row.clone(), chosen.clone());
        choose.connect_clicked(move |b| {
            let parent = b.root().and_downcast::<gtk4::Window>();
            let dialog = gtk4::FileDialog::new();
            dialog.set_title("Choose the game's program file");
            let (file_row, name_row, chosen) = (file_row.clone(), name_row.clone(), chosen.clone());
            dialog.open(parent.as_ref(), None::<&gio::Cancellable>, move |res| {
                let Some(path) = res.ok().and_then(|f| f.path()) else { return };
                file_row.set_subtitle(&path.to_string_lossy());
                if name_row.text().is_empty() {
                    let stem = path.file_stem().unwrap_or_default().to_string_lossy().replace(['_', '-'], " ");
                    name_row.set_text(&stem);
                }
                *chosen.borrow_mut() = Some(path);
            });
        });
    }

    let add_btn = gtk4::Button::with_label("Add game");
    add_btn.add_css_class("suggested-action");
    add_btn.set_halign(gtk4::Align::End);
    {
        let (win, holder, status, name_row, gm, chosen) = (win.clone(), holder.clone(), status.clone(), name_row.clone(), gm.clone(), chosen.clone());
        add_btn.connect_clicked(move |_| {
            let Some(path) = chosen.borrow().clone() else {
                status.set_text("Choose the game's program file first.");
                return;
            };
            let (name, gamemode, p) = (name_row.text().to_string(), gm.is_active() && gm.is_sensitive(), path.to_string_lossy().to_string());
            let (win, holder, status) = (win.clone(), holder.clone(), status.clone());
            in_background(move || add_program(&name, &p, gamemode), move |res| match res {
                Ok(()) => {
                    fill(&holder);
                    win.close();
                }
                Err(e) => status.set_text(&e),
            });
        });
    }
    content.append(&add_btn);

    // 2. An app that is already installed.
    let inst = adw::PreferencesGroup::new();
    inst.set_title("Or an app you already installed");
    let pick = adw::ActionRow::new();
    pick.set_title("Mark an installed app as a game");
    pick.set_subtitle("Pick it from the list of your apps");
    let pb = gtk4::Button::with_label("Pick…");
    pb.set_valign(gtk4::Align::Center);
    {
        let (holder, win) = (holder.clone(), win.clone());
        pb.connect_clicked(move |b| pick_installed_window(b, &holder, Some(&win)));
    }
    pick.add_suffix(&pb);
    inst.add(&pick);
    content.append(&inst);
    content.append(&status);

    win.set_child(Some(&content));
    win.present();
}

fn pick_installed_window(from: &impl IsA<gtk4::Widget>, holder: &gtk4::Box, add_win: Option<&gtk4::Window>) {
    let Some(parent) = from.root().and_downcast::<gtk4::Window>() else { return };
    let win = gtk4::Window::builder().title("Pick an app to mark as a game").transient_for(&parent).modal(true).default_width(480).default_height(560).build();
    let col = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    col.set_margin_top(12);
    col.set_margin_bottom(12);
    col.set_margin_start(12);
    col.set_margin_end(12);
    let search = gtk4::SearchEntry::new();
    search.set_placeholder_text(Some("Search your apps"));
    col.append(&search);
    let scroll = gtk4::ScrolledWindow::builder().vexpand(true).hscrollbar_policy(gtk4::PolicyType::Never).build();
    let list = gtk4::ListBox::new();
    list.set_selection_mode(gtk4::SelectionMode::None);
    list.add_css_class("boxed-list");
    let status = gtk4::Label::new(None);
    status.set_wrap(true);
    status.add_css_class("dim-label");

    let mut apps: Vec<gio::DesktopAppInfo> = gio::AppInfo::all().into_iter().filter(|a| a.should_show()).filter_map(|a| a.downcast::<gio::DesktopAppInfo>().ok()).collect();
    apps.sort_by_key(|a| a.name().to_lowercase());
    for info in apps {
        let r = adw::ActionRow::new();
        r.set_title(&glib::markup_escape_text(&info.name()));
        if let Some(icon) = info.icon() {
            let img = gtk4::Image::from_gicon(&icon);
            img.set_pixel_size(32);
            r.add_prefix(&img);
        }
        r.set_activatable(true);
        let (holder, win, add_win, status) = (holder.clone(), win.clone(), add_win.cloned(), status.clone());
        r.connect_activated(move |_| match mark_installed(&info) {
            Ok(()) => {
                fill(&holder);
                win.close();
                if let Some(w) = &add_win {
                    w.close();
                }
            }
            Err(e) => status.set_text(&e),
        });
        list.append(&r);
    }
    {
        let s = search.clone();
        list.set_filter_func(move |row| {
            let q = s.text().to_lowercase();
            q.is_empty() || row.downcast_ref::<adw::ActionRow>().map_or(true, |r| r.title().to_lowercase().contains(&q))
        });
        let l = list.clone();
        search.connect_search_changed(move |_| l.invalidate_filter());
    }
    scroll.set_child(Some(&list));
    col.append(&scroll);
    col.append(&status);
    win.set_child(Some(&col));
    win.present();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_become_file_names() {
        assert_eq!(slug("Half-Life 2: Remastered!"), "half-life-2-remastered");
        assert_eq!(slug("  --  "), "");
    }

    #[test]
    fn paths_are_quoted_for_exec() {
        assert_eq!(exec_quote("/home/me/My Games/run.sh"), "\"/home/me/My Games/run.sh\"");
        assert_eq!(exec_quote("/a/$b\"c"), "\"/a/\\$b\\\"c\"");
        assert_eq!(exec_quote("/a/100%"), "\"/a/100%%\"");
    }

    #[test]
    fn exe_files_run_through_wine_and_game_mode_goes_first() {
        assert_eq!(launch_command("/g/Game.EXE", false), "wine \"/g/Game.EXE\"");
        assert_eq!(launch_command("/g/run", true), "gamemoderun \"/g/run\"");
    }

    #[test]
    fn entry_is_a_marked_game() {
        let e = desktop_entry("Cool Game", "/g/cool/run.sh", false);
        assert!(e.contains("Categories=Game;"));
        assert!(e.contains("Path=/g/cool\n"));
        assert!(is_marked(&e));
        assert_eq!(key_of(&e, "Name").as_deref(), Some("Cool Game"));
    }

    #[test]
    fn installed_apps_get_the_game_category_without_losing_anything() {
        let orig = "[Desktop Entry]\nName=Foo\nExec=foo\nCategories=Utility;Network;\n\n[Desktop Action new]\nName=New\nExec=foo --new\n";
        let m = mark_as_game(orig);
        assert!(m.contains("Categories=Game;Utility;Network;"));
        assert!(is_marked(&m));
        assert!(m.contains("[Desktop Action new]\nName=New\nExec=foo --new"));
        // already a game: nothing is added twice
        assert_eq!(mark_as_game(&m), m);
        // no categories at all
        let m2 = mark_as_game("[Desktop Entry]\nName=Bar\nExec=bar\n");
        assert!(m2.contains("Categories=Game;"));
        assert!(is_marked(&m2));
    }
}
