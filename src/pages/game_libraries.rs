//! Gaming > Your games: games found automatically in the launchers' own libraries, so nothing has to be added by
//! hand. Steam (every library folder it knows, including other drives), Lutris and Heroic (Epic and GOG), each looked
//! for both as a normal install and as the Flatpak version the Game stores section installs. Play starts the game
//! through its launcher, so the launcher's own settings (Proton, Wine, the library path) are used.

use crate::backend::worker::in_background;
use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone, PartialEq)]
pub struct Game {
    pub launcher: &'static str,
    pub name: String,
    /// The command that starts it.
    pub launch: Vec<String>,
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
}

// ── Steam ──────────────────────────────────────────────────────────────────

/// The value of `"key"  "value"` on a Valve data (VDF/ACF) line.
pub fn vdf_value(line: &str, key: &str) -> Option<String> {
    let mut parts = line.split('"').skip(1).step_by(2); // the quoted pieces
    let k = parts.next()?;
    let v = parts.next()?;
    (k.eq_ignore_ascii_case(key)).then(|| v.replace("\\\\", "\\"))
}

/// Every library folder in `libraryfolders.vdf`.
pub fn library_paths(vdf: &str) -> Vec<String> {
    vdf.lines().filter_map(|l| vdf_value(l, "path")).collect()
}

/// Steam keeps its own tools in the same folders as games.
pub fn is_steam_tool(name: &str) -> bool {
    ["Proton", "Steam Linux Runtime", "Steamworks Common", "Steam Controller Configs"].iter().any(|p| name.starts_with(p))
}

/// A fully installed game from an `appmanifest_*.acf`.
pub fn parse_acf(text: &str) -> Option<(String, String)> {
    let (mut id, mut name, mut flags) = (None, None, 0u32);
    for l in text.lines() {
        if let Some(v) = vdf_value(l, "appid") {
            id.get_or_insert(v);
        } else if let Some(v) = vdf_value(l, "name") {
            name.get_or_insert(v);
        } else if let Some(v) = vdf_value(l, "StateFlags") {
            flags = v.parse().unwrap_or(0);
        }
    }
    let (id, name) = (id?, name?);
    // Bit 4 is "fully installed"; the rest are updating, queued or uninstalling.
    (flags & 4 != 0 && !is_steam_tool(&name)).then_some((id, name))
}

fn steam_roots() -> Vec<PathBuf> {
    let h = home();
    vec![
        h.join(".local/share/Steam"),
        h.join(".steam/steam"),
        h.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
        h.join(".var/app/com.valvesoftware.Steam/data/Steam"),
    ]
}

fn steam_games() -> Vec<Game> {
    let mut libs: Vec<PathBuf> = Vec::new();
    let mut flatpak_only = true;
    for root in steam_roots() {
        let Ok(text) = std::fs::read_to_string(root.join("steamapps/libraryfolders.vdf")) else { continue };
        if !root.to_string_lossy().contains(".var/app") {
            flatpak_only = false;
        }
        libs.push(root.clone());
        libs.extend(library_paths(&text).into_iter().map(PathBuf::from));
    }
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for lib in libs {
        let Ok(rd) = std::fs::read_dir(lib.join("steamapps")) else { continue };
        for e in rd.flatten() {
            let f = e.file_name().to_string_lossy().to_string();
            if !(f.starts_with("appmanifest_") && f.ends_with(".acf")) {
                continue;
            }
            let Some((id, name)) = std::fs::read_to_string(e.path()).ok().as_deref().and_then(parse_acf) else { continue };
            if !seen.insert(id.clone()) {
                continue;
            }
            let url = format!("steam://rungameid/{id}");
            let launch = if flatpak_only {
                vec!["flatpak".into(), "run".into(), "com.valvesoftware.Steam".into(), url]
            } else {
                vec!["xdg-open".into(), url]
            };
            out.push(Game { launcher: "Steam", name, launch });
        }
    }
    out
}

// ── Lutris ─────────────────────────────────────────────────────────────────

/// "hades-1650000000" -> "hades".
pub fn lutris_slug(file_stem: &str) -> String {
    match file_stem.rsplit_once('-') {
        Some((slug, tail)) if !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit()) => slug.to_string(),
        _ => file_stem.to_string(),
    }
}

/// "the-witcher-3" -> "The Witcher 3".
pub fn pretty_slug(slug: &str) -> String {
    slug.split(['-', '_'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `sqlite3 -json` output of `select name, slug from games where installed = 1`.
pub fn parse_lutris_json(text: &str) -> Vec<(String, String)> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else { return Vec::new() };
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|g| Some((g.get("name")?.as_str()?.to_string(), g.get("slug")?.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

fn lutris_games() -> Vec<Game> {
    let h = home();
    // (data dir, config dir, how to start Lutris)
    let setups: [(PathBuf, PathBuf, Vec<String>); 2] = [
        (h.join(".local/share/lutris"), h.join(".config/lutris"), vec!["lutris".into()]),
        (
            h.join(".var/app/net.lutris.Lutris/data/lutris"),
            h.join(".var/app/net.lutris.Lutris/config/lutris"),
            vec!["flatpak".into(), "run".into(), "net.lutris.Lutris".into()],
        ),
    ];
    let mut out = Vec::new();
    for (data, config, start) in setups {
        let mut found: Vec<(String, String)> = Vec::new();
        let db = data.join("pga.db");
        if db.is_file() {
            if let Ok(o) = Command::new("sqlite3").args(["-readonly", "-json"]).arg(&db).arg("select name, slug from games where installed = 1").output() {
                if o.status.success() {
                    found = parse_lutris_json(&String::from_utf8_lossy(&o.stdout));
                }
            }
        }
        if found.is_empty() {
            // No sqlite3 tool: each installed game has a config file named after it.
            if let Ok(rd) = std::fs::read_dir(config.join("games")) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.extension().map_or(false, |x| x == "yml") {
                        let slug = lutris_slug(&p.file_stem().unwrap_or_default().to_string_lossy());
                        found.push((pretty_slug(&slug), slug));
                    }
                }
            }
        }
        for (name, slug) in found {
            let mut launch = start.clone();
            launch.push(format!("lutris:rungame/{slug}"));
            if !out.iter().any(|g: &Game| g.launcher == "Lutris" && g.name == name) {
                out.push(Game { launcher: "Lutris", name, launch });
            }
        }
    }
    out
}

// ── Heroic (Epic and GOG) ──────────────────────────────────────────────────

/// Epic: `legendary/installed.json` maps app name -> details with a `title`.
pub fn parse_legendary(text: &str) -> Vec<(String, String)> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else { return Vec::new() };
    v.as_object()
        .map(|o| {
            o.iter()
                .filter_map(|(app, d)| Some((app.clone(), d.get("title")?.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

/// GOG: `installed.json` lists app names; titles come from the library file (`{"games": [{app_name, title}]}`).
pub fn parse_gog(installed: &str, library: &str) -> Vec<(String, String)> {
    let Ok(i) = serde_json::from_str::<serde_json::Value>(installed) else { return Vec::new() };
    let lib: serde_json::Value = serde_json::from_str(library).unwrap_or_default();
    let title_of = |app: &str| -> String {
        lib.get("games")
            .and_then(|g| g.as_array())
            .and_then(|g| g.iter().find(|x| x.get("app_name").and_then(|a| a.as_str()) == Some(app)))
            .and_then(|x| x.get("title")?.as_str().map(str::to_string))
            .unwrap_or_else(|| app.to_string())
    };
    i.get("installed")
        .and_then(|a| a.as_array())
        .map(|a| a.iter().filter_map(|g| g.get("appName")?.as_str().map(|n| (n.to_string(), title_of(n)))).collect())
        .unwrap_or_default()
}

fn heroic_games() -> Vec<Game> {
    let h = home();
    let mut out = Vec::new();
    for base in [h.join(".config/heroic"), h.join(".var/app/com.heroicgameslauncher.hgl/config/heroic")] {
        let read = |rel: &str| std::fs::read_to_string(base.join(rel)).unwrap_or_default();
        let mut add = |runner: &str, list: Vec<(String, String)>| {
            for (app, title) in list {
                if !out.iter().any(|g: &Game| g.launcher == "Heroic" && g.name == title) {
                    out.push(Game { launcher: "Heroic", name: title, launch: vec!["xdg-open".into(), format!("heroic://launch/{runner}/{app}")] });
                }
            }
        };
        add("legendary", parse_legendary(&read("legendaryConfig/legendary/installed.json")));
        add("gog", parse_gog(&read("gog_store/installed.json"), &read("store_cache/gog_library.json")));
    }
    out
}

/// Everything found, grouped by launcher (Steam, Lutris, Heroic) and sorted by name.
pub fn detect() -> Vec<Game> {
    let mut all = steam_games();
    all.extend(lutris_games());
    all.extend(heroic_games());
    all.sort_by(|a, b| (a.launcher, a.name.to_lowercase()).cmp(&(b.launcher, b.name.to_lowercase())));
    all
}

// ── Page ───────────────────────────────────────────────────────────────────

pub fn build() -> gtk4::Box {
    let holder = gtk4::Box::new(gtk4::Orientation::Vertical, 24);
    fill(&holder);
    holder
}

fn fill(holder: &gtk4::Box) {
    while let Some(c) = holder.first_child() {
        holder.remove(&c);
    }
    let loading = adw::PreferencesGroup::new();
    loading.set_title("Your games");
    let r = adw::ActionRow::new();
    r.set_title("Looking for your games…");
    let spin = gtk4::Spinner::new();
    spin.start();
    r.add_suffix(&spin);
    loading.add(&r);
    holder.append(&loading);
    let holder = holder.clone();
    in_background(detect, move |games| show(&holder, games));
}

fn show(holder: &gtk4::Box, games: Vec<Game>) {
    while let Some(c) = holder.first_child() {
        holder.remove(&c);
    }
    let head = adw::PreferencesGroup::new();
    head.set_title("Your games");
    head.set_description(Some("Found automatically in your Steam, Lutris and Heroic libraries, wherever they are stored."));
    let rescan = gtk4::Button::from_icon_name("view-refresh-symbolic");
    rescan.set_valign(gtk4::Align::Center);
    rescan.set_tooltip_text(Some("Look for games again"));
    rescan.update_property(&[gtk4::accessible::Property::Label("Look for games again")]);
    let h = holder.clone();
    rescan.connect_clicked(move |_| fill(&h));
    head.set_header_suffix(Some(&rescan));
    if games.is_empty() {
        let r = adw::ActionRow::new();
        r.set_title("No games found yet");
        r.set_subtitle("Install a game in Steam, Lutris or Heroic and it shows up here. Get the launchers under Game stores below.");
        head.add(&r);
        holder.append(&head);
        return;
    }
    holder.append(&head);
    for launcher in ["Steam", "Lutris", "Heroic"] {
        let list: Vec<&Game> = games.iter().filter(|g| g.launcher == launcher).collect();
        if list.is_empty() {
            continue;
        }
        let g = adw::PreferencesGroup::new();
        g.set_title(&format!("{launcher} ({})", list.len()));
        for game in list {
            let r = adw::ActionRow::new();
            r.set_title(&glib::markup_escape_text(&game.name));
            r.add_prefix(&gtk4::Image::from_icon_name("applications-games-symbolic"));
            let play = gtk4::Button::with_label("Play");
            play.add_css_class("suggested-action");
            play.set_valign(gtk4::Align::Center);
            let cmd = game.launch.clone();
            play.connect_clicked(move |_| {
                if let Some((prog, args)) = cmd.split_first() {
                    let _ = Command::new(prog).args(args).spawn();
                }
            });
            r.add_suffix(&play);
            g.add(&r);
        }
        holder.append(&g);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vdf_lines_are_read() {
        assert_eq!(vdf_value("\t\t\"path\"\t\t\"/mnt/games/SteamLibrary\"", "path").as_deref(), Some("/mnt/games/SteamLibrary"));
        assert_eq!(vdf_value("\"name\"\t\"Half-Life\"", "name").as_deref(), Some("Half-Life"));
        assert_eq!(vdf_value("\"name\"\t\"x\"", "path"), None);
        assert_eq!(vdf_value("{", "path"), None);
        assert_eq!(vdf_value("\"path\" \"D:\\\\Games\"", "path").as_deref(), Some("D:\\Games"));
    }

    #[test]
    fn library_folders_include_other_drives() {
        let v = "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"/home/me/.local/share/Steam\"\n\t}\n\t\"1\"\n\t{\n\t\t\"path\"\t\t\"/mnt/ssd/SteamLibrary\"\n\t}\n}\n";
        assert_eq!(library_paths(v), ["/home/me/.local/share/Steam", "/mnt/ssd/SteamLibrary"]);
    }

    #[test]
    fn installed_steam_games_only() {
        let ok = "\"AppState\"\n{\n\t\"appid\"\t\t\"220\"\n\t\"name\"\t\t\"Half-Life 2\"\n\t\"StateFlags\"\t\t\"4\"\n}\n";
        assert_eq!(parse_acf(ok), Some(("220".into(), "Half-Life 2".into())));
        // still downloading
        assert_eq!(parse_acf(&ok.replace("\"4\"", "\"1026\"")), None);
        // Steam's own tools are not games
        assert_eq!(parse_acf(&ok.replace("Half-Life 2", "Proton 9.0")), None);
        assert_eq!(parse_acf(&ok.replace("Half-Life 2", "Steam Linux Runtime 3.0 (sniper)")), None);
    }

    #[test]
    fn lutris_names() {
        assert_eq!(lutris_slug("hades-1650000000"), "hades");
        assert_eq!(lutris_slug("the-witcher-3-1650000000"), "the-witcher-3");
        assert_eq!(lutris_slug("plain"), "plain");
        assert_eq!(pretty_slug("the-witcher-3"), "The Witcher 3");
        let j = "[{\"name\":\"Hades\",\"slug\":\"hades\"},{\"name\":\"Celeste\",\"slug\":\"celeste\"}]";
        assert_eq!(parse_lutris_json(j), [("Hades".into(), "hades".into()), ("Celeste".into(), "celeste".into())]);
        assert!(parse_lutris_json("").is_empty());
    }

    #[test]
    fn heroic_epic_and_gog() {
        let epic = "{\"Fortnite\":{\"title\":\"Fortnite\",\"install_path\":\"/x\"},\"abc\":{\"title\":\"Hades\"}}";
        let mut e = parse_legendary(epic);
        e.sort();
        assert_eq!(e, [("Fortnite".into(), "Fortnite".into()), ("abc".into(), "Hades".into())]);
        let installed = "{\"installed\":[{\"appName\":\"1207658924\"},{\"appName\":\"99\"}]}";
        let lib = "{\"games\":[{\"app_name\":\"1207658924\",\"title\":\"Witcher\"}]}";
        assert_eq!(parse_gog(installed, lib), [("1207658924".into(), "Witcher".into()), ("99".into(), "99".into())]);
        assert!(parse_gog("nope", "").is_empty());
    }

    #[test]
    fn detect_finds_games_in_fake_libraries() {
        let root = std::env::temp_dir().join(format!("zs-games-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let w = |rel: &str, text: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        };
        let ssd = root.join("ssd/SteamLibrary");
        w(".local/share/Steam/steamapps/libraryfolders.vdf", &format!("\"libraryfolders\"\n{{\n\"1\"\n{{\n\"path\"\t\"{}\"\n}}\n}}\n", ssd.display()));
        w(".local/share/Steam/steamapps/appmanifest_10.acf", "\"AppState\"\n{\n\"appid\"\t\"10\"\n\"name\"\t\"Counter-Strike\"\n\"StateFlags\"\t\"4\"\n}\n");
        w("ssd/SteamLibrary/steamapps/appmanifest_20.acf", "\"AppState\"\n{\n\"appid\"\t\"20\"\n\"name\"\t\"Portal\"\n\"StateFlags\"\t\"4\"\n}\n");
        w("ssd/SteamLibrary/steamapps/appmanifest_30.acf", "\"AppState\"\n{\n\"appid\"\t\"30\"\n\"name\"\t\"Proton 9.0\"\n\"StateFlags\"\t\"4\"\n}\n");
        w(".config/lutris/games/hades-1650000000.yml", "game:\n  exe: x\n");
        w(".var/app/com.heroicgameslauncher.hgl/config/heroic/legendaryConfig/legendary/installed.json", "{\"app1\":{\"title\":\"Rocket League\"}}");
        std::env::set_var("HOME", &root);
        let names: Vec<String> = detect().into_iter().map(|g| format!("{}:{}", g.launcher, g.name)).collect();
        // sorted by launcher then name; Proton (a Steam tool) is left out
        assert_eq!(names, ["Heroic:Rocket League", "Lutris:Hades", "Steam:Counter-Strike", "Steam:Portal"]);
        let _ = std::fs::remove_dir_all(&root);
    }
}
