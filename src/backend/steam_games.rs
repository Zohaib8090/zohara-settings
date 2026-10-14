//! Steam games: list the installed ones and read/write their launch options and compatibility tool, so the same
//! settings window as My games can manage them (graphics chip, Game Mode, MangoHud, DXVK, Proton-GE).
//!
//! Steam keeps these in plain text "VDF" files: `steamapps/appmanifest_<id>.acf` (the game), `userdata/<user>/config/
//! localconfig.vdf` (launch options) and `config/config.vdf` (compatibility tool per game). Steam rewrites them when it
//! quits, so they are only changed while Steam is not running, and a `.zohara-backup` copy is kept the first time.

use super::game_launch::{exec_quote_shell, parse_env, split_words, GameSettings, Runner};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------------------------------------------
// VDF text format
// ---------------------------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    Str(String),
    Map(Vec<(String, Node)>),
}

impl Node {
    fn map(&self) -> Option<&Vec<(String, Node)>> {
        if let Node::Map(m) = self { Some(m) } else { None }
    }

    /// Child by key, ignoring case (Steam writes `apps` and `Apps` in different versions).
    pub fn get(&self, key: &str) -> Option<&Node> {
        self.map()?.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v)
    }

    pub fn path(&self, keys: &[&str]) -> Option<&Node> {
        keys.iter().try_fold(self, |n, k| n.get(k))
    }

    pub fn text(&self) -> Option<&str> {
        if let Node::Str(s) = self { Some(s) } else { None }
    }

    /// The child map for `key`, made when missing.
    fn child_mut(&mut self, key: &str) -> &mut Node {
        let Node::Map(m) = self else { panic!("not a map") };
        let at = match m.iter().position(|(k, _)| k.eq_ignore_ascii_case(key)) {
            Some(i) => i,
            None => {
                m.push((key.to_string(), Node::Map(Vec::new())));
                m.len() - 1
            }
        };
        if !matches!(m[at].1, Node::Map(_)) {
            m[at].1 = Node::Map(Vec::new());
        }
        &mut m[at].1
    }

    fn set_str(&mut self, key: &str, value: &str) {
        let Node::Map(m) = self else { panic!("not a map") };
        match m.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(key)) {
            Some((_, v)) => *v = Node::Str(value.to_string()),
            None => m.push((key.to_string(), Node::Str(value.to_string()))),
        }
    }

    fn remove(&mut self, key: &str) {
        if let Node::Map(m) = self {
            m.retain(|(k, _)| !k.eq_ignore_ascii_case(key));
        }
    }
}

/// Parses VDF text into a map holding its top-level entries. None = the text is not valid VDF.
pub fn parse_vdf(text: &str) -> Option<Node> {
    let mut toks: Vec<Tok> = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(&c) = chars.peek() {
        match c {
            c if c.is_whitespace() => {
                chars.next();
            }
            '/' => {
                // a comment to the end of the line
                while let Some(c) = chars.next() {
                    if c == '\n' {
                        break;
                    }
                }
            }
            '{' => {
                chars.next();
                toks.push(Tok::Open);
            }
            '}' => {
                chars.next();
                toks.push(Tok::Close);
            }
            '"' => {
                chars.next();
                let mut s = String::new();
                loop {
                    match chars.next()? {
                        '"' => break,
                        '\\' => match chars.next()? {
                            'n' => s.push('\n'),
                            't' => s.push('\t'),
                            other => s.push(other),
                        },
                        c => s.push(c),
                    }
                }
                toks.push(Tok::Str(s));
            }
            _ => return None,
        }
    }
    let mut at = 0;
    let top = read_map(&toks, &mut at, true)?;
    (at == toks.len()).then_some(Node::Map(top))
}

enum Tok {
    Open,
    Close,
    Str(String),
}

fn read_map(toks: &[Tok], at: &mut usize, top: bool) -> Option<Vec<(String, Node)>> {
    let mut out = Vec::new();
    loop {
        match toks.get(*at) {
            None => return top.then_some(out),
            Some(Tok::Close) => {
                if top {
                    return None;
                }
                *at += 1;
                return Some(out);
            }
            Some(Tok::Str(k)) => {
                *at += 1;
                match toks.get(*at)? {
                    Tok::Str(v) => {
                        out.push((k.clone(), Node::Str(v.clone())));
                        *at += 1;
                    }
                    Tok::Open => {
                        *at += 1;
                        let m = read_map(toks, at, false)?;
                        out.push((k.clone(), Node::Map(m)));
                    }
                    Tok::Close => return None,
                }
            }
            Some(Tok::Open) => return None,
        }
    }
}

fn quote(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\t' => o.push_str("\\t"),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// Steam's own layout: tabs, `"key"\t\t"value"`, braces on their own lines.
pub fn write_vdf(root: &Node) -> String {
    fn entries(m: &[(String, Node)], depth: usize, out: &mut String) {
        let pad = "\t".repeat(depth);
        for (k, v) in m {
            match v {
                Node::Str(s) => out.push_str(&format!("{pad}{}\t\t{}\n", quote(k), quote(s))),
                Node::Map(c) => {
                    out.push_str(&format!("{pad}{}\n{pad}{{\n", quote(k)));
                    entries(c, depth + 1, out);
                    out.push_str(&format!("{pad}}}\n"));
                }
            }
        }
    }
    let mut out = String::new();
    if let Node::Map(m) = root {
        entries(m, 0, &mut out);
    }
    out
}

// ---------------------------------------------------------------------------------------------------------------
// Where Steam is and which games it has
// ---------------------------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SteamRoot {
    pub path: PathBuf,
    /// The Flatpak Steam runs in a sandbox: Game Mode and MangoHud are not inside it.
    pub flatpak: bool,
}

pub fn roots_in(home: &Path) -> Vec<SteamRoot> {
    [(".local/share/Steam", false), (".var/app/com.valvesoftware.Steam/.local/share/Steam", true)]
        .into_iter()
        .map(|(p, flatpak)| SteamRoot { path: home.join(p), flatpak })
        .filter(|r| r.path.join("steamapps").is_dir() || r.path.join("userdata").is_dir())
        .collect()
}

pub fn roots() -> Vec<SteamRoot> {
    roots_in(&PathBuf::from(std::env::var("HOME").unwrap_or_default()))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SteamGame {
    pub appid: String,
    pub name: String,
    pub root: SteamRoot,
}

/// Tools Steam lists next to games: not something a person plays.
fn is_tool(name: &str) -> bool {
    let n = name.to_lowercase();
    n.starts_with("proton") || n.starts_with("steam linux runtime") || n.starts_with("steamworks common") || n.starts_with("steam runtime")
}

fn library_paths(root: &SteamRoot) -> Vec<PathBuf> {
    let mut v = vec![root.path.clone()];
    let text = std::fs::read_to_string(root.path.join("steamapps/libraryfolders.vdf")).unwrap_or_default();
    if let Some(Node::Map(libs)) = parse_vdf(&text).and_then(|t| t.get("libraryfolders").cloned()) {
        for (_, lib) in libs.iter().filter(|(_, l)| l.map().is_some()) {
            if let Some(p) = lib.get("path").and_then(|p| p.text()) {
                let p = PathBuf::from(p);
                if !v.contains(&p) {
                    v.push(p);
                }
            }
        }
    }
    v
}

/// The installed games of one Steam, by name.
pub fn installed_games(root: &SteamRoot) -> Vec<SteamGame> {
    let mut out: Vec<SteamGame> = Vec::new();
    for lib in library_paths(root) {
        let Ok(rd) = std::fs::read_dir(lib.join("steamapps")) else { continue };
        for e in rd.flatten() {
            let f = e.file_name().to_string_lossy().to_string();
            let Some(id) = f.strip_prefix("appmanifest_").and_then(|r| r.strip_suffix(".acf")) else { continue };
            let Some(top) = std::fs::read_to_string(e.path()).ok().and_then(|t| parse_vdf(&t)) else { continue };
            let Some(name) = top.path(&["AppState", "name"]).and_then(|n| n.text()) else { continue };
            if is_tool(name) || out.iter().any(|g| g.appid == id) {
                continue;
            }
            out.push(SteamGame { appid: id.to_string(), name: name.to_string(), root: root.clone() });
        }
    }
    out.sort_by_key(|g| g.name.to_lowercase());
    out
}

// ---------------------------------------------------------------------------------------------------------------
// Launch options <-> the settings window
// ---------------------------------------------------------------------------------------------------------------

/// Launch options for a game: `[env A=b] [gamemoderun] [mangohud] %command% <options>`.
/// `gpu_env` are the variables that put the game on the stronger chip.
pub fn launch_text(s: &GameSettings, gpu_env: &[(String, String)]) -> Result<String, String> {
    split_words(&s.options).ok_or("A quote in the game options is not closed.")?;
    let mut env: Vec<(String, String)> = Vec::new();
    if s.dedicated_gpu {
        env.extend(gpu_env.iter().cloned());
    }
    if s.runner == Runner::Proton && !s.dxvk {
        env.push(("PROTON_USE_WINED3D".into(), "1".into()));
    }
    env.extend(parse_env(&s.env)?);
    let mut words: Vec<String> = Vec::new();
    if !env.is_empty() {
        words.push("env".into());
        words.extend(env.iter().map(|(k, v)| format!("{k}={}", exec_quote_shell(v))));
    }
    if s.gamemode {
        words.push("gamemoderun".into());
    }
    if s.overlay {
        words.push("mangohud".into());
    }
    if words.is_empty() && s.options.trim().is_empty() {
        return Ok(String::new());
    }
    words.push("%command%".into());
    if !s.options.trim().is_empty() {
        words.push(s.options.trim().to_string());
    }
    Ok(words.join(" "))
}

/// Reads launch options back into settings. None = the text is something we did not write (the person's own).
pub fn parse_launch(text: &str, gpu_env: &[(String, String)]) -> Option<GameSettings> {
    let mut s = GameSettings::default();
    let text = text.trim();
    if text.is_empty() {
        return Some(s);
    }
    let (head, options) = match text.find("%command%") {
        Some(i) => (&text[..i], text[i + "%command%".len()..].trim()),
        None => ("", text),
    };
    s.options = options.to_string();
    let words = split_words(head)?;
    let mut env: Vec<(String, String)> = Vec::new();
    let mut it = words.into_iter().peekable();
    if it.peek().map(|w| w == "env").unwrap_or(false) {
        it.next();
    }
    for w in it {
        match w.as_str() {
            "gamemoderun" => s.gamemode = true,
            "mangohud" => s.overlay = true,
            other => {
                let (k, v) = other.split_once('=')?;
                if k.is_empty() || !k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                    return None;
                }
                env.push((k.to_string(), v.to_string()));
            }
        }
    }
    if !gpu_env.is_empty() && gpu_env.iter().all(|p| env.contains(p)) {
        s.dedicated_gpu = true;
        env.retain(|p| !gpu_env.contains(p));
    }
    if let Some(i) = env.iter().position(|(k, v)| k == "PROTON_USE_WINED3D" && v == "1") {
        s.dxvk = false;
        s.runner = Runner::Proton;
        env.remove(i);
    }
    s.env = env.iter().map(|(k, v)| format!("{k}={}", exec_quote_shell(v))).collect::<Vec<_>>().join(" ");
    Some(s)
}

// ---------------------------------------------------------------------------------------------------------------
// Reading and writing Steam's files
// ---------------------------------------------------------------------------------------------------------------

/// Steam rewrites its files when it quits, so nothing is written while it runs.
pub fn steam_running() -> bool {
    std::fs::read_dir("/proc")
        .map(|rd| {
            rd.flatten().any(|e| {
                e.file_name().to_string_lossy().bytes().all(|b| b.is_ascii_digit())
                    && std::fs::read_to_string(e.path().join("comm")).map(|c| c.trim() == "steam").unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

fn localconfigs(root: &SteamRoot) -> Vec<PathBuf> {
    std::fs::read_dir(root.path.join("userdata"))
        .map(|rd| rd.flatten().map(|e| e.path().join("config/localconfig.vdf")).filter(|p| p.is_file()).collect())
        .unwrap_or_default()
}

fn config_vdf(root: &SteamRoot) -> PathBuf {
    root.path.join("config/config.vdf")
}

fn load(path: &Path) -> Result<Node, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("Couldn't read {} ({e})", path.display()))?;
    parse_vdf(&text).ok_or_else(|| format!("{} is not in the format Steam writes, so it was left alone.", path.display()))
}

fn save(path: &Path, node: &Node) -> Result<(), String> {
    let backup = path.with_extension("vdf.zohara-backup");
    if !backup.exists() {
        std::fs::copy(path, &backup).map_err(|e| format!("Couldn't keep a backup of {} ({e})", path.display()))?;
    }
    let tmp = path.with_extension("vdf.zohara-new");
    std::fs::write(&tmp, write_vdf(node)).map_err(|e| format!("Couldn't write {} ({e})", path.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("Couldn't replace {} ({e})", path.display()))
}

const APPS: [&str; 4] = ["Software", "Valve", "Steam", "apps"];

/// The game's launch options as Steam has them (empty when none).
pub fn launch_options(game: &SteamGame) -> String {
    localconfigs(&game.root)
        .iter()
        .filter_map(|p| load(p).ok())
        .find_map(|n| {
            let mut path = vec!["UserLocalConfigStore"];
            path.extend(APPS);
            path.push(&game.appid);
            path.push("LaunchOptions");
            n.path(&path).and_then(|v| v.text()).map(|s| s.to_string())
        })
        .unwrap_or_default()
}

/// The Proton tool Steam is told to use for this game ("" = Steam's own default).
pub fn compat_tool(game: &SteamGame) -> String {
    load(&config_vdf(&game.root))
        .ok()
        .and_then(|n| {
            let mut path = vec!["InstallConfigStore"];
            path.extend(&APPS[..3]);
            path.push("CompatToolMapping");
            path.push(&game.appid);
            path.push("name");
            n.path(&path).and_then(|v| v.text()).map(|s| s.to_string())
        })
        .unwrap_or_default()
}

fn set_launch_in(node: &mut Node, appid: &str, text: &str) {
    let mut n = node.child_mut("UserLocalConfigStore");
    for k in APPS {
        n = n.child_mut(k);
    }
    let app = n.child_mut(appid);
    if text.is_empty() {
        app.remove("LaunchOptions");
    } else {
        app.set_str("LaunchOptions", text);
    }
}

fn set_tool_in(node: &mut Node, appid: &str, tool: &str) {
    let mut n = node.child_mut("InstallConfigStore");
    for k in &APPS[..3] {
        n = n.child_mut(k);
    }
    let map = n.child_mut("CompatToolMapping");
    if tool.is_empty() {
        map.remove(appid);
    } else {
        let e = map.child_mut(appid);
        e.set_str("name", tool);
        e.set_str("config", "");
        e.set_str("priority", "250");
    }
}

/// Writes the game's launch options and Proton tool. `tool` "" = Steam's default. Err = what to tell the person.
pub fn apply(game: &SteamGame, launch: &str, tool: &str) -> Result<(), String> {
    if steam_running() {
        return Err("Steam is running. Close Steam completely (Steam > Exit), then press Save again.".into());
    }
    let cfgs = localconfigs(&game.root);
    if cfgs.is_empty() {
        return Err("Steam has no signed-in user yet. Start Steam and sign in once first.".into());
    }
    // everything is read and checked before the first write, so a file we can't read leaves all of them as they were
    let mut loaded: Vec<(PathBuf, Node)> = Vec::new();
    for p in cfgs {
        loaded.push((p.clone(), load(&p)?));
    }
    let cfg = config_vdf(&game.root);
    let mut global = if cfg.is_file() { Some(load(&cfg)?) } else { None };
    for (p, mut n) in loaded {
        set_launch_in(&mut n, &game.appid, launch);
        save(&p, &n)?;
    }
    if let Some(g) = global.as_mut() {
        set_tool_in(g, &game.appid, tool);
        save(&cfg, g)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vdf_round_trips_with_escapes_and_comments() {
        let text = "// c\n\"A\"\n{\n\t\"k\"\t\t\"va\\\"l\\\\ue\"\n\t\"sub\"\n\t{\n\t\t\"x\"\t\t\"\"\n\t}\n}\n";
        let n = parse_vdf(text).unwrap();
        assert_eq!(n.path(&["a", "k"]).unwrap().text(), Some("va\"l\\ue"));
        let again = parse_vdf(&write_vdf(&n)).unwrap();
        assert_eq!(n, again);
        assert!(parse_vdf("\"A\" {").is_none());
        assert!(parse_vdf("}").is_none());
    }

    #[test]
    fn launch_options_round_trip_through_settings() {
        let gpu = vec![("DRI_PRIME".to_string(), "1".to_string())];
        let s = GameSettings { runner: Runner::Proton, dxvk: false, gamemode: true, overlay: true, dedicated_gpu: true, options: "-windowed -w 1280".into(), env: "FOO=bar".into() };
        let t = launch_text(&s, &gpu).unwrap();
        assert_eq!(t, "env DRI_PRIME=1 PROTON_USE_WINED3D=1 FOO=bar gamemoderun mangohud %command% -windowed -w 1280");
        let back = parse_launch(&t, &gpu).unwrap();
        assert_eq!((back.gamemode, back.overlay, back.dedicated_gpu, back.dxvk), (true, true, true, false));
        assert_eq!(back.options, "-windowed -w 1280");
        assert_eq!(back.env, "FOO=bar");
        // nothing chosen = no launch options at all
        assert_eq!(launch_text(&GameSettings::default(), &gpu).unwrap(), "");
        // options only
        let o = GameSettings { options: "-novid".into(), ..GameSettings::default() };
        assert_eq!(launch_text(&o, &gpu).unwrap(), "%command% -novid");
        assert_eq!(parse_launch("-novid", &gpu).unwrap().options, "-novid");
        // somebody else's text is not ours to rewrite quietly
        assert!(parse_launch("SOMETHING; rm x %command%", &gpu).is_none());
        assert!(launch_text(&GameSettings { env: "9bad=1".into(), ..GameSettings::default() }, &gpu).is_err());
    }

    #[test]
    fn installed_games_skip_tools_and_read_other_libraries() {
        let base = std::env::temp_dir().join(format!("zs-steam-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let other = base.join("lib2");
        let steam = base.join(".local/share/Steam");
        std::fs::create_dir_all(steam.join("steamapps")).unwrap();
        std::fs::create_dir_all(other.join("steamapps")).unwrap();
        let lib = format!("\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n", steam.display(), other.display());
        std::fs::write(steam.join("steamapps/libraryfolders.vdf"), lib).unwrap();
        let acf = |n: &str| format!("\"AppState\"\n{{\n\t\"appid\"\t\t\"1\"\n\t\"name\"\t\t\"{n}\"\n}}\n");
        std::fs::write(steam.join("steamapps/appmanifest_10.acf"), acf("Half-Life")).unwrap();
        std::fs::write(steam.join("steamapps/appmanifest_11.acf"), acf("Proton 9.0")).unwrap();
        std::fs::write(other.join("steamapps/appmanifest_20.acf"), acf("Celeste")).unwrap();
        let root = SteamRoot { path: steam.clone(), flatpak: false };
        let names: Vec<_> = installed_games(&root).into_iter().map(|g| (g.appid, g.name)).collect();
        assert_eq!(names, vec![("20".into(), "Celeste".into()), ("10".into(), "Half-Life".into())]);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn apply_edits_only_what_it_should_and_keeps_a_backup() {
        let base = std::env::temp_dir().join(format!("zs-steam-apply-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let steam = base.join("Steam");
        std::fs::create_dir_all(steam.join("userdata/1/config")).unwrap();
        std::fs::create_dir_all(steam.join("config")).unwrap();
        let local = steam.join("userdata/1/config/localconfig.vdf");
        let orig = "\"UserLocalConfigStore\"\n{\n\t\"friends\"\n\t{\n\t\t\"PersonaName\"\t\t\"me\"\n\t}\n}\n";
        std::fs::write(&local, orig).unwrap();
        std::fs::write(steam.join("config/config.vdf"), "\"InstallConfigStore\"\n{\n\t\"Software\"\n\t{\n\t}\n}\n").unwrap();
        let game = SteamGame { appid: "730".into(), name: "G".into(), root: SteamRoot { path: steam.clone(), flatpak: false } };
        if steam_running() {
            return; // a real Steam on this machine would (rightly) refuse
        }
        apply(&game, "gamemoderun %command%", "GE-Proton11-9").unwrap();
        assert_eq!(launch_options(&game), "gamemoderun %command%");
        assert_eq!(compat_tool(&game), "GE-Proton11-9");
        let after = parse_vdf(&std::fs::read_to_string(&local).unwrap()).unwrap();
        assert_eq!(after.path(&["UserLocalConfigStore", "friends", "PersonaName"]).unwrap().text(), Some("me"));
        assert_eq!(std::fs::read_to_string(local.with_extension("vdf.zohara-backup")).unwrap(), orig);
        // clearing removes both again
        apply(&game, "", "").unwrap();
        assert_eq!(launch_options(&game), "");
        assert_eq!(compat_tool(&game), "");
        let _ = std::fs::remove_dir_all(&base);
    }
}
