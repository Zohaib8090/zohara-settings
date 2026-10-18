//! Gaming > My games: games that were not installed from a store. A game is a desktop entry in
//! `~/.local/share/applications` with `Categories=Game;` (so it shows under Games in the Start menu) and the marker
//! `X-Zohara-Game=true` (so this page can list and remove only what it added). Two ways to add one: pick a program file
//! (a native program, an AppImage or a Windows .exe), or mark an app that is already installed as a game.

use crate::backend::game_launch::{self, Context, GameSettings, Runner};
use crate::backend::worker::in_background;
use crate::backend::{gpu_pref, proton};
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

/// What the launch line needs to know about this computer, for the game with this id and program file.
fn context_for(id: &str, path: &str, settings: &GameSettings, tag: Option<&str>) -> Context {
    let (prefix, wine) = crate::backend::windows_apps::prefixes_for(path, id);
    let mut ctx = Context::here(prefix, wine, settings.dedicated_gpu);
    if let Some(t) = tag {
        ctx.use_proton_tag(t);
    }
    ctx
}

/// Choices that live next to the game's settings in its entry. None = keep what the entry has.
#[derive(Default)]
struct Extras {
    sandbox: Option<bool>,
    /// Some(None) = back to the newest Proton-GE.
    proton_tag: Option<Option<String>>,
}

/// The icon file for a game, when its program has one of its own. Made once, kept when the game is edited.
fn icon_for(file: &Path, path: &str, old_entry: &str) -> Option<String> {
    let current = old_entry.lines().find_map(|l| l.strip_prefix("Icon=")).map(|v| v.trim().to_string());
    if let Some(c) = current.filter(|c| c.starts_with('/') && Path::new(c).exists()) {
        return Some(c);
    }
    if !path.to_lowercase().ends_with(".exe") {
        return None;
    }
    let id = file.file_stem()?.to_string_lossy().trim_start_matches("zohara-game-").to_string();
    let data = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share"));
    let out = data.join("zohara/icons").join(format!("{id}.png"));
    crate::backend::pe_icon::extract_png(Path::new(path), &out).ok().map(|_| out.to_string_lossy().to_string())
}

/// Writes the game's desktop entry (new or changed) and refreshes the menu.
fn save_game(file: &Path, name: &str, path: &str, settings: &GameSettings, extras: &Extras) -> Result<(), String> {
    let id = file.file_stem().map(|s| s.to_string_lossy().trim_start_matches("zohara-game-").to_string()).unwrap_or_default();
    let old = std::fs::read_to_string(file).unwrap_or_default();
    let tag = match &extras.proton_tag {
        Some(t) => t.clone(),
        None => game_launch::proton_tag_of(&old),
    };
    let ctx = context_for(&id, path, settings, tag.as_deref());
    let mut text = game_launch::entry_text(name.trim(), path, settings, &ctx)?;
    // Safety mode: what was chosen now, else what the game already had (off for a new game)
    text = game_launch::with_sandbox(&text, extras.sandbox.unwrap_or_else(|| game_launch::sandbox_of(&old)));
    text = game_launch::with_proton_tag(&text, tag.as_deref());
    if let Some(icon) = icon_for(file, path, &old) {
        text = text.replacen("Icon=applications-games", &format!("Icon={icon}"), 1);
    }
    if settings.runner == Runner::Proton {
        std::fs::create_dir_all(&ctx.prefix_dir).map_err(|e| format!("Couldn't make the game's Windows folder: {e}"))?;
    } else if !ctx.wine_prefix.as_os_str().is_empty() {
        std::fs::create_dir_all(&ctx.wine_prefix).map_err(|e| format!("Couldn't make the game's Windows folder: {e}"))?;
    }
    std::fs::write(file, text).map_err(|e| e.to_string())?;
    refresh_menu();
    Ok(())
}

fn add_program(name: &str, path: &str, settings: &GameSettings) -> Result<(), String> {
    let p = Path::new(path);
    if !p.is_file() {
        return Err("That file doesn't exist".into());
    }
    if name.trim().is_empty() {
        return Err("Give the game a name".into());
    }
    // An AppImage or a program needs the executable bit; Wine runs a Windows file without it.
    if !game_launch::uses_wine(path, settings.runner) {
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
    save_game(&apps_dir().join(format!("zohara-game-{id}.desktop")), name, path, settings, &Extras::default())
}

/// The shared Windows-program settings for a game that is added with one click. Plain Wine until Proton-GE is installed.
fn default_windows_settings() -> GameSettings {
    let mut settings = crate::backend::windows_apps::load();
    if settings.runner == Runner::Proton && proton::installed().is_empty() {
        settings.runner = Runner::Wine; // Proton-GE is a download away; the game can be switched to it later
    }
    settings
}

/// The programs Windows installers made, with an Add button each.
fn found_window(from: &impl IsA<gtk4::Widget>, holder: &gtk4::Box) {
    let Some(parent) = from.root().and_downcast::<gtk4::Window>() else { return };
    let win = adw::Window::builder().title("Windows programs found").transient_for(&parent).modal(true).default_width(520).default_height(480).build();
    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&adw::HeaderBar::new());
    let spin = gtk4::Spinner::new();
    spin.start();
    spin.set_halign(gtk4::Align::Center);
    spin.set_valign(gtk4::Align::Center);
    tv.set_content(Some(&spin));
    win.set_content(Some(&tv));
    win.present();
    let known: Vec<String> = my_games().iter().filter_map(|g| std::fs::read_to_string(&g.file).ok().and_then(|t| game_launch::parse_entry(&t)).map(|(p, _)| p)).collect();
    let (tv, holder) = (tv.clone(), holder.clone());
    in_background(
        move || {
            let data = std::env::var("XDG_DATA_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share"));
            crate::backend::installed_programs::find(&data, &known)
        },
        move |found| {
            let page = adw::PreferencesPage::new();
            let g = adw::PreferencesGroup::new();
            g.set_title("Installed with a Windows installer");
            g.set_description(Some("These are in the shared Windows folder that double-clicked installers use. Add the ones you want as games."));
            if found.is_empty() {
                let r = adw::ActionRow::new();
                r.set_title("Nothing new found");
                r.set_subtitle("Run a Windows installer by double-clicking it, finish it, then look again.");
                g.add(&r);
            }
            for f in found {
                let r = adw::ActionRow::new();
                r.set_title(&glib::markup_escape_text(&f.name));
                r.set_subtitle(&glib::markup_escape_text(&f.exe.to_string_lossy()));
                r.set_subtitle_lines(1);
                let add = gtk4::Button::with_label("Add");
                add.add_css_class("suggested-action");
                add.set_valign(gtk4::Align::Center);
                let (holder, name, path) = (holder.clone(), f.name.clone(), f.exe.to_string_lossy().to_string());
                add.connect_clicked(move |b| {
                    b.set_sensitive(false);
                    b.set_label("Adding…");
                    let (holder, b, name, path) = (holder.clone(), b.clone(), name.clone(), path.clone());
                    in_background(
                        move || {
                            let mut id_name = name.clone();
                            let mut n = 2;
                            while apps_dir().join(format!("zohara-game-{}.desktop", slug(&id_name))).exists() {
                                id_name = format!("{name} {n}");
                                n += 1;
                            }
                            add_program(&id_name, &path, &default_windows_settings())
                        },
                        move |res| match res {
                            Ok(()) => {
                                b.set_label("Added");
                                fill(&holder);
                            }
                            Err(e) => {
                                b.set_label("Failed");
                                b.set_tooltip_text(Some(&e));
                            }
                        },
                    );
                });
                r.add_suffix(&add);
                g.add(&r);
            }
            page.add(&g);
            tv.set_content(Some(&page));
        },
    );
}

/// The Proton-GE version pinned for the game with this id (read from its entry).
fn tools_pin(id: &str) -> Option<String> {
    std::fs::read_to_string(apps_dir().join(format!("zohara-game-{id}.desktop"))).ok().and_then(|t| game_launch::proton_tag_of(&t))
}

/// "Hades_II-setup.exe" -> "Hades II setup".
fn name_from_file(path: &str) -> String {
    let stem = Path::new(path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    stem.replace(['_', '-', '.'], " ").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Right-click > Zohara > Add as a game: the file goes into My games with the shared Windows-program settings.
/// Exit code for the caller; the answer is shown as a notification.
pub fn add_from_file_manager(path: &str) -> i32 {
    let base = name_from_file(path);
    let settings = default_windows_settings();
    let mut name = base.clone();
    let mut n = 2;
    while apps_dir().join(format!("zohara-game-{}.desktop", slug(&name))).exists() {
        name = format!("{base} {n}");
        n += 1;
    }
    let (title, body, code) = match add_program(&name, path, &settings) {
        Ok(()) => ("Added to My games".to_string(), format!("{name} is in Settings > Gaming > My games and in the Start menu."), 0),
        Err(e) => ("Couldn't add the game".to_string(), e, 1),
    };
    let _ = Command::new("notify-send").args(["-a", "Zohara", "-i", "applications-games", &title, &body]).status();
    code
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
    g.set_description(Some("Added by hand, for games that no launcher knows about. They show up under Games in the Start menu."));

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
    let find = gtk4::Button::with_label("Find Windows programs");
    find.set_valign(gtk4::Align::Center);
    find.set_tooltip_text(Some("Look for programs that a Windows installer put on this computer"));
    let h = holder.clone();
    find.connect_clicked(move |b| found_window(b, &h));
    let buttons = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    buttons.append(&from_apps);
    buttons.append(&add);
    buttons.append(&find);
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
        // a game's own icon is a picture file; the others are icon names
        let icon = if game.icon.starts_with('/') { gtk4::Image::from_file(&game.icon) } else { gtk4::Image::from_icon_name(&game.icon) };
        icon.set_pixel_size(32);
        r.add_prefix(&icon);
        let play = gtk4::Button::with_label("Play");
        play.add_css_class("suggested-action");
        play.set_valign(gtk4::Align::Center);
        // Only games added from a program file have settings to change; an installed app that was marked as a game keeps
        // its own launcher.
        let entry = std::fs::read_to_string(&game.file).ok().and_then(|t| game_launch::parse_entry(&t));
        // A Windows program goes through the same path as a double-click: its own settings, a log, and a notice when it
        // closes right away.
        let windows_file = entry.as_ref().filter(|(path, s)| game_launch::uses_wine(path, s.runner)).map(|(path, _)| path.clone());
        let f = game.file.clone();
        play.connect_clicked(move |_| {
            if let Some(path) = &windows_file {
                if let Ok(exe) = std::env::current_exe() {
                    if Command::new(exe).arg("--run-windows").arg(path).spawn().is_ok() {
                        return;
                    }
                }
            }
            if let Some(info) = gio::DesktopAppInfo::from_filename(&f) {
                let _ = info.launch(&[], None::<&gio::AppLaunchContext>);
            }
        });
        if let Some((path, current)) = entry {
            let gear = gtk4::Button::from_icon_name("emblem-system-symbolic");
            gear.set_valign(gtk4::Align::Center);
            gear.set_tooltip_text(Some("Settings for this game"));
            gear.update_property(&[gtk4::accessible::Property::Label("Settings for this game")]);
            let (f, h, name) = (game.file.clone(), holder.clone(), game.name.clone());
            gear.connect_clicked(move |b| settings_window(b, &h, &f, &name, &path, &current));
            r.add_suffix(&gear);
        }
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

/// One game's settings: how it runs, which graphics chip, Game Mode, the overlay, options and variables.
fn settings_window(from: &impl IsA<gtk4::Widget>, holder: &gtk4::Box, file: &Path, name: &str, path: &str, current: &GameSettings) {
    let Some(parent) = from.root().and_downcast::<gtk4::Window>() else { return };
    let win = adw::Window::builder().title(format!("{name}: settings")).transient_for(&parent).modal(true).default_width(560).default_height(640).build();
    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::new();
    group.set_title("How this game runs");
    group.set_description(Some(&glib::markup_escape_text(path)));

    let name_row = adw::EntryRow::new();
    name_row.set_title("Name");
    name_row.set_text(name);

    let installed = proton::installed();
    let run_row = adw::ComboRow::new();
    run_row.set_title("Run with");
    run_row.set_subtitle_lines(0);
    run_row.set_subtitle(&match installed.first() {
        Some(v) => format!("Proton-GE is installed ({v}). Windows app support is plain Wine."),
        None => "Proton-GE is not installed yet (Gaming > Windows games). Windows app support is plain Wine.".to_string(),
    });
    run_row.set_model(Some(&gtk4::StringList::new(&Runner::ALL.iter().map(|r| r.label()).collect::<Vec<_>>())));
    run_row.set_selected(Runner::ALL.iter().position(|r| *r == current.runner).unwrap_or(0) as u32);

    let dxvk = adw::SwitchRow::new();
    dxvk.set_title("DirectX through Vulkan (DXVK)");
    dxvk.set_subtitle("Proton only. Faster for most games; turn it off if a game shows glitches.");
    dxvk.set_subtitle_lines(0);
    dxvk.set_active(current.dxvk);
    let sync_dxvk = {
        let (dxvk, run_row) = (dxvk.clone(), run_row.clone());
        move || dxvk.set_sensitive(Runner::ALL.get(run_row.selected() as usize) == Some(&Runner::Proton))
    };
    sync_dxvk();
    run_row.connect_selected_notify(move |_| sync_dxvk());

    let gpu = adw::ComboRow::new();
    gpu.set_title("Graphics chip");
    gpu.set_subtitle_lines(0);
    gpu.set_model(Some(&gtk4::StringList::new(&["Automatic", "Dedicated"])));
    gpu.set_selected(current.dedicated_gpu as u32);
    if !gpu_pref::has_two_gpus() {
        gpu.set_sensitive(false);
        gpu.set_subtitle("This computer has one graphics chip, so there is nothing to choose.");
    } else {
        gpu.set_subtitle("Automatic lets the system decide. On a laptop with two chips, Dedicated runs the game on the stronger one.");
    }

    let has = |cmd: &str| Command::new("sh").args(["-c", &format!("command -v {cmd}")]).output().map(|o| o.status.success()).unwrap_or(false);
    let gm = adw::SwitchRow::new();
    gm.set_title("Game Mode");
    gm.set_subtitle(if has("gamemoderun") { "Give the game priority while it runs" } else { "Game Mode isn't installed" });
    gm.set_active(current.gamemode && has("gamemoderun"));
    gm.set_sensitive(has("gamemoderun"));
    let ov = adw::SwitchRow::new();
    ov.set_title("Performance overlay (MangoHud)");
    ov.set_subtitle(if has("mangohud") { "Shows frame rate and load on screen while playing" } else { "MangoHud isn't installed" });
    ov.set_active(current.overlay && has("mangohud"));
    ov.set_sensitive(has("mangohud"));

    let options = adw::EntryRow::new();
    options.set_title("Game options (for example -windowed)");
    options.set_text(&current.options);
    let env = adw::EntryRow::new();
    env.set_title("Environment variables (NAME=value NAME2=value)");
    env.set_text(&current.env);

    for r in [name_row.clone().upcast::<gtk4::Widget>(), run_row.clone().upcast(), dxvk.clone().upcast(), gpu.clone().upcast(), gm.clone().upcast(), ov.clone().upcast(), options.clone().upcast(), env.clone().upcast()] {
        group.add(&r);
    }
    let safe = adw::SwitchRow::new();
    safe.set_title("Safety mode");
    safe.set_subtitle("Run this game in a protected space: it sees an empty home folder, not your files, passwords or browser. Anti-cheat games will not run in it, and Game Mode does not reach into it.");
    safe.set_subtitle_lines(0);
    safe.set_active(std::fs::read_to_string(file).map(|t| game_launch::sandbox_of(&t)).unwrap_or(false));
    safe.set_sensitive(game_launch::uses_wine(path, current.runner) && crate::backend::windows_sandbox::available());
    group.add(&safe);

    let log_row = adw::ActionRow::new();
    log_row.set_title("What happened last time");
    log_row.set_subtitle("The game's own messages from its last start. Useful when it closes at once.");
    log_row.set_subtitle_lines(0);
    let show = gtk4::Button::with_label("Show log");
    show.set_valign(gtk4::Align::Center);
    let id = file.file_stem().map(|s| s.to_string_lossy().trim_start_matches("zohara-game-").to_string()).unwrap_or_default();
    let log_file = crate::backend::windows_apps::log_path(Some(&id));
    show.set_sensitive(log_file.exists());
    show.connect_clicked(move |_| {
        let _ = Command::new("xdg-open").arg(&log_file).spawn();
    });
    log_row.add_suffix(&show);
    group.add(&log_row);
    page.add(&group);

    // ── Tools for this game (Windows programs only) ──
    let tools_tag: std::rc::Rc<std::cell::RefCell<Option<String>>> =
        std::rc::Rc::new(std::cell::RefCell::new(std::fs::read_to_string(file).ok().and_then(|t| game_launch::proton_tag_of(&t))));
    if game_launch::uses_wine(path, current.runner) {
        let id = file.file_stem().map(|s| s.to_string_lossy().trim_start_matches("zohara-game-").to_string()).unwrap_or_default();
        let (prefix_dir, wine_dir) = crate::backend::windows_apps::prefixes_for(path, &id);
        let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
        let data = std::env::var("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|_| home.join(".local/share"));
        let runner = if current.runner == Runner::Proton { Runner::Proton } else { Runner::Wine };
        let wine_prefix = if runner == Runner::Proton {
            prefix_dir.join("pfx")
        } else if wine_dir.as_os_str().is_empty() {
            home.join(".wine")
        } else {
            wine_dir.clone()
        };

        let tg = adw::PreferencesGroup::new();
        tg.set_title("Tools for this game");

        let versions = proton::installed();
        let ver = adw::ComboRow::new();
        ver.set_title("Proton-GE version");
        ver.set_subtitle_lines(0);
        let mut names: Vec<String> = vec!["Newest installed".into()];
        names.extend(versions.iter().cloned());
        if let Some(t) = tools_tag.borrow().as_ref().filter(|t| !versions.contains(t)) {
            names.push(format!("{t} (not installed)"));
        }
        ver.set_model(Some(&gtk4::StringList::new(&names.iter().map(|s| s.as_str()).collect::<Vec<_>>())));
        let at = tools_tag.borrow().as_ref().and_then(|t| versions.iter().position(|v| v == t).map(|i| i + 1)).unwrap_or(if tools_tag.borrow().is_some() { names.len() - 1 } else { 0 });
        ver.set_selected(at as u32);
        ver.set_subtitle("Some games only work with an older version. It is used the next time the game starts.");
        ver.set_sensitive(runner == Runner::Proton && !versions.is_empty());
        {
            let (tools_tag, versions) = (tools_tag.clone(), versions.clone());
            ver.connect_selected_notify(move |r| {
                let i = r.selected() as usize;
                if i == 0 {
                    *tools_tag.borrow_mut() = None;
                } else if let Some(v) = versions.get(i - 1) {
                    *tools_tag.borrow_mut() = Some(v.clone());
                }
            });
        }
        tg.add(&ver);

        let comp = adw::ComboRow::new();
        comp.set_title("Add a Windows component");
        comp.set_subtitle_lines(0);
        comp.set_subtitle("For a game that asks for a missing file or shows no text or sound. Needs the internet.");
        comp.set_model(Some(&gtk4::StringList::new(&crate::backend::game_tools::COMPONENTS.iter().map(|(_, l)| *l).collect::<Vec<_>>())));
        let go = gtk4::Button::with_label("Install");
        go.set_valign(gtk4::Align::Center);
        comp.add_suffix(&go);
        {
            let (comp, wine_prefix, id) = (comp.clone(), wine_prefix.clone(), id.clone());
            go.connect_clicked(move |b| {
                let verb = crate::backend::game_tools::COMPONENTS[(comp.selected() as usize).min(crate::backend::game_tools::COMPONENTS.len() - 1)].0;
                b.set_sensitive(false);
                b.set_label("Installing…");
                comp.set_subtitle("Working. This can take several minutes; the window can stay open.");
                let proton_dir = Context::here(PathBuf::new(), PathBuf::new(), false).proton_dir;
                let proton_dir = match tools_pin(&id) {
                    Some(t) => {
                        let mut c = Context::here(PathBuf::new(), PathBuf::new(), false);
                        c.use_proton_tag(&t);
                        c.proton_dir
                    }
                    None => proton_dir,
                };
                let log = crate::backend::windows_apps::log_path(Some(&format!("{id}-winetricks")));
                let (wine_prefix, comp, b) = (wine_prefix.clone(), comp.clone(), b.clone());
                in_background(
                    move || crate::backend::game_tools::install_component(verb, runner, proton_dir.as_deref(), &wine_prefix, &log),
                    move |res| {
                        b.set_sensitive(true);
                        b.set_label("Install");
                        comp.set_subtitle(&match res {
                            Ok(()) => format!("{verb} is installed."),
                            Err(e) => e,
                        });
                    },
                );
            });
        }
        tg.add(&comp);

        let cdrow = adw::ActionRow::new();
        cdrow.set_title("Windows C: drive of this game");
        cdrow.set_subtitle("Open it to look at saves, settings and what the game installed.");
        let open = gtk4::Button::with_label("Open");
        open.set_valign(gtk4::Align::Center);
        cdrow.add_suffix(&open);
        {
            let wine_prefix = wine_prefix.clone();
            open.connect_clicked(move |_| {
                let c = wine_prefix.join("drive_c");
                let target = if c.exists() { c } else { wine_prefix.clone() };
                if target.exists() {
                    let _ = Command::new("xdg-open").arg(target).spawn();
                }
            });
        }
        tg.add(&cdrow);

        // the folder this game's runner uses: Proton's, or plain Wine's own
        let reset_target = if runner == Runner::Proton { prefix_dir.clone() } else { wine_prefix.clone() };
        let private = crate::backend::game_tools::is_private_prefix(&reset_target, &data);
        let reset = adw::ActionRow::new();
        reset.set_title("Reset this game's Windows folder");
        reset.set_subtitle(if private {
            "Starts the game with a fresh Windows folder. Use it when a game stopped working after changes. Saves kept inside the Windows folder go to the trash with it."
        } else {
            "This game uses the shared Windows folder of double-clicked programs, which other programs use too, so it is not reset here."
        });
        reset.set_subtitle_lines(0);
        let rbtn = gtk4::Button::with_label("Reset…");
        rbtn.add_css_class("destructive-action");
        rbtn.set_valign(gtk4::Align::Center);
        rbtn.set_sensitive(private);
        reset.add_suffix(&rbtn);
        {
            let (prefix_dir, data) = (reset_target.clone(), data.clone());
            rbtn.connect_clicked(move |b| {
                let d = adw::AlertDialog::new(Some("Reset this game's Windows folder?"), Some("It is moved to the trash, so it can still be taken back from there. The game itself is not touched."));
                d.add_response("cancel", "Cancel");
                d.add_response("reset", "Reset");
                d.set_response_appearance("reset", adw::ResponseAppearance::Destructive);
                let (prefix_dir, data, b2) = (prefix_dir.clone(), data.clone(), b.clone());
                d.connect_response(None, move |_, resp| {
                    if resp == "reset" {
                        let (p, dd, b3) = (prefix_dir.clone(), data.clone(), b2.clone());
                        in_background(move || crate::backend::game_tools::reset_prefix(&p, &dd), move |res| {
                            b3.set_label(if res.is_ok() { "Done" } else { "Failed" });
                            if let Err(e) = res {
                                b3.set_tooltip_text(Some(&e));
                            }
                        });
                    }
                });
                d.present(Some(b));
            });
        }
        tg.add(&reset);
        page.add(&tg);
    }

    let status = gtk4::Label::new(None);
    status.set_wrap(true);
    status.set_halign(gtk4::Align::Start);
    status.add_css_class("dim-label");
    let save = gtk4::Button::with_label("Save");
    save.add_css_class("suggested-action");
    save.set_halign(gtk4::Align::End);
    let cancel = gtk4::Button::with_label("Cancel");
    let (w2, c2) = (win.clone(), cancel.clone());
    c2.connect_clicked(move |_| w2.close());
    {
        let (win, holder, status, file, path) = (win.clone(), holder.clone(), status.clone(), file.to_path_buf(), path.to_string());
        let (name_row, run_row, dxvk, gpu, gm, ov, options, env, safe) = (name_row.clone(), run_row.clone(), dxvk.clone(), gpu.clone(), gm.clone(), ov.clone(), options.clone(), env.clone(), safe.clone());
        save.connect_clicked(move |_| {
            let settings = GameSettings {
                runner: Runner::ALL[(run_row.selected() as usize).min(Runner::ALL.len() - 1)],
                dxvk: dxvk.is_active(),
                gamemode: gm.is_active(),
                overlay: ov.is_active(),
                dedicated_gpu: gpu.selected() == 1,
                options: options.text().to_string(),
                env: env.text().to_string(),
            };
            let (name, file, path) = (name_row.text().to_string(), file.clone(), path.clone());
            if name.trim().is_empty() {
                status.set_text("Give the game a name.");
                return;
            }
            let (win, holder, status) = (win.clone(), holder.clone(), status.clone());
            let extras = Extras { sandbox: Some(safe.is_active()), proton_tag: Some(tools_tag.borrow().clone()) };
            in_background(move || save_game(&file, &name, &path, &settings, &extras), move |res| match res {
                Ok(()) => {
                    fill(&holder);
                    win.close();
                }
                Err(e) => status.set_text(&e),
            });
        });
    }
    let bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    bar.set_margin_top(8);
    bar.set_margin_bottom(12);
    bar.set_margin_start(18);
    bar.set_margin_end(18);
    status.set_hexpand(true);
    bar.append(&status);
    bar.append(&cancel);
    bar.append(&save);

    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&adw::HeaderBar::new());
    tv.set_content(Some(&page));
    tv.add_bottom_bar(&bar);
    win.set_content(Some(&tv));
    win.present();
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
    file_group.set_description(Some("Any program file: a program, an AppImage, or a Windows game (.exe, .bat, installer). Pick how to run it below."));
    let name_row = adw::EntryRow::new();
    name_row.set_title("Name");
    let file_row = adw::ActionRow::new();
    file_row.set_title("Program file");
    file_row.set_subtitle("Nothing chosen yet");
    let choose = gtk4::Button::with_label("Choose…");
    choose.set_valign(gtk4::Align::Center);
    file_row.add_suffix(&choose);
    let run_row = adw::ComboRow::new();
    run_row.set_title("Run with");
    run_row.set_subtitle("Automatic sends Windows files (.exe, .bat, .cmd, .msi, .lnk) to Windows app support and runs everything else itself");
    run_row.set_subtitle_lines(0);
    run_row.set_model(Some(&gtk4::StringList::new(&Runner::ALL.iter().map(|r| r.label()).collect::<Vec<_>>())));
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
    file_group.add(&run_row);
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
        let (win, holder, status, name_row, gm, chosen, run_row) = (win.clone(), holder.clone(), status.clone(), name_row.clone(), gm.clone(), chosen.clone(), run_row.clone());
        add_btn.connect_clicked(move |_| {
            let Some(path) = chosen.borrow().clone() else {
                status.set_text("Choose the game's program file first.");
                return;
            };
            let (name, gamemode, p) = (name_row.text().to_string(), gm.is_active() && gm.is_sensitive(), path.to_string_lossy().to_string());
            let settings = GameSettings { gamemode, runner: Runner::ALL[(run_row.selected() as usize).min(Runner::ALL.len() - 1)], ..GameSettings::default() };
            let (win, holder, status) = (win.clone(), holder.clone(), status.clone());
            in_background(move || add_program(&name, &p, &settings), move |res| match res {
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
    // Apps already added are not offered again.
    let added: std::collections::HashSet<std::ffi::OsString> = my_games().into_iter().filter_map(|g| g.file.file_name().map(|n| n.to_os_string())).collect();
    apps.retain(|a| a.filename().and_then(|f| f.file_name().map(|n| n.to_os_string())).map_or(true, |n| !added.contains(&n)));
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
    #[test]
    fn a_game_name_is_made_from_the_file_name() {
        assert_eq!(super::name_from_file("/g/Hades_II-setup.exe"), "Hades II setup");
        assert_eq!(super::name_from_file("/g/Cities Skylines!.bat"), "Cities Skylines!");
    }

    use super::*;

    #[test]
    fn names_become_file_names() {
        assert_eq!(slug("Half-Life 2: Remastered!"), "half-life-2-remastered");
        assert_eq!(slug("  --  "), "");
    }

    #[test]
    fn paths_are_quoted_for_exec() {
        use crate::backend::game_launch::exec_quote;
        assert_eq!(exec_quote("/home/me/My Games/run.sh"), "\"/home/me/My Games/run.sh\"");
        assert_eq!(exec_quote("/a/$b\"c"), "\"/a/\\$b\\\"c\"");
        assert_eq!(exec_quote("/a/100%"), "\"/a/100%%\"");
    }

    #[test]
    fn entry_is_a_marked_game() {
        let e = game_launch::entry_text("Cool Game", "/g/cool/run.sh", &GameSettings::default(), &Context::default()).unwrap();
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
