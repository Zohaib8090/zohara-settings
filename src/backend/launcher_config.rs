//! Per-game settings of Lutris and Heroic, so the same settings window as My games and Steam can manage their games
//! (graphics chip, Game Mode, MangoHud, DXVK, options, variables).
//!
//! * **Lutris** keeps each game in `~/.config/lutris/games/<slug>-<id>.yml`. Only the few keys we manage are read and
//!   written (`system: gamemode / mangohud / env`, `game: args`, `wine: dxvk`); everything else in the file is kept
//!   exactly as it was. A file that does not look the way Lutris writes it is refused, not guessed at.
//! * **Heroic** keeps each game in `GamesConfig/<app>.json`: `{"<app>": {overrides}, "version": "v0.1", "explicit": false}`.
//!   Only the keys we manage are set (`showMangohud`, `useGameMode`, `enviromentOptions`, `launcherArgs`,
//!   `autoInstallDxvk`). (Key names from Heroic's own source, `src/common/types.ts` and `src/backend/game_config.ts`.)
//!
//! The launchers rewrite these files while they run, so nothing is written while the launcher is open, and a
//! `.zohara-backup` copy is kept the first time.

use super::game_launch::{parse_env, split_words, GameSettings, Runner};
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------------------------------------------
// Shared
// ---------------------------------------------------------------------------------------------------------------

/// Whether a program with this name is running (its process name, or its command line for Electron apps).
pub fn launcher_running(word: &str) -> bool {
    let Ok(rd) = std::fs::read_dir("/proc") else { return false };
    rd.flatten().any(|e| {
        if !e.file_name().to_string_lossy().bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
        let comm = std::fs::read_to_string(e.path().join("comm")).unwrap_or_default();
        if comm.trim() == word {
            return true;
        }
        std::fs::read(e.path().join("cmdline"))
            .map(|c| c.split(|b| *b == 0).next().map(|a| String::from_utf8_lossy(a).to_lowercase().rsplit('/').next().unwrap_or("").starts_with(word)).unwrap_or(false))
            .unwrap_or(false)
    })
}

fn backup_then_write(path: &Path, text: &str) -> Result<(), String> {
    if path.exists() {
        let backup = PathBuf::from(format!("{}.zohara-backup", path.display()));
        if !backup.exists() {
            std::fs::copy(path, &backup).map_err(|e| format!("Couldn't keep a backup of {} ({e})", path.display()))?;
        }
    } else if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let tmp = PathBuf::from(format!("{}.zohara-new", path.display()));
    std::fs::write(&tmp, text).map_err(|e| format!("Couldn't write {} ({e})", path.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("Couldn't replace {} ({e})", path.display()))
}

/// The settings with the chip variables split out: all of `gpu_env` present = Dedicated.
fn split_gpu(mut env: Vec<(String, String)>, gpu_env: &[(String, String)]) -> (bool, Vec<(String, String)>) {
    if !gpu_env.is_empty() && gpu_env.iter().all(|p| env.contains(p)) {
        env.retain(|p| !gpu_env.contains(p));
        (true, env)
    } else {
        (false, env)
    }
}

fn env_text(env: &[(String, String)]) -> String {
    env.iter()
        .map(|(k, v)| if v.is_empty() || v.chars().any(|c| c.is_whitespace() || c == '"' || c == '\'') { format!("{k}=\"{}\"", v.replace('"', "\\\"")) } else { format!("{k}={v}") })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The variables to write: the chip's (when Dedicated) then the typed ones.
fn wanted_env(s: &GameSettings, gpu_env: &[(String, String)]) -> Result<Vec<(String, String)>, String> {
    let mut env = Vec::new();
    if s.dedicated_gpu {
        env.extend(gpu_env.iter().cloned());
    }
    for (k, v) in parse_env(&s.env)? {
        if let Some(i) = env.iter().position(|(ek, _)| *ek == k) {
            env.remove(i);
        }
        env.push((k, v));
    }
    Ok(env)
}

// ---------------------------------------------------------------------------------------------------------------
// Heroic (JSON)
// ---------------------------------------------------------------------------------------------------------------

pub fn heroic_read(text: &str, app: &str, gpu_env: &[(String, String)]) -> GameSettings {
    let v: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
    let g = v.get(app).cloned().unwrap_or_default();
    let env: Vec<(String, String)> = g
        .get("enviromentOptions")
        .and_then(|e| e.as_array())
        .map(|a| a.iter().filter_map(|o| Some((o.get("key")?.as_str()?.to_string(), o.get("value")?.as_str()?.to_string()))).collect())
        .unwrap_or_default();
    let (dedicated, env) = split_gpu(env, gpu_env);
    GameSettings {
        runner: Runner::Auto,
        dxvk: g.get("autoInstallDxvk").and_then(|b| b.as_bool()).unwrap_or(true),
        gamemode: g.get("useGameMode").and_then(|b| b.as_bool()).unwrap_or(false),
        overlay: g.get("showMangohud").and_then(|b| b.as_bool()).unwrap_or(false),
        dedicated_gpu: dedicated,
        options: g.get("launcherArgs").and_then(|s| s.as_str()).unwrap_or("").to_string(),
        env: env_text(&env),
    }
}

/// The file's text with the game's managed keys set. An empty or missing file becomes a fresh v0.1 override file.
pub fn heroic_write(text: &str, app: &str, s: &GameSettings, gpu_env: &[(String, String)]) -> Result<String, String> {
    split_words(&s.options).ok_or("A quote in the game options is not closed.")?;
    let env = wanted_env(s, gpu_env)?;
    let mut v: serde_json::Value = if text.trim().is_empty() {
        serde_json::json!({ "version": "v0.1", "explicit": false })
    } else {
        serde_json::from_str(text).map_err(|_| "Heroic's settings file for this game is not in the format Heroic writes, so it was left alone.")?
    };
    let root = v.as_object_mut().ok_or("Heroic's settings file for this game is not in the format Heroic writes, so it was left alone.")?;
    let game = root.entry(app.to_string()).or_insert_with(|| serde_json::json!({}));
    let game = game.as_object_mut().ok_or("Heroic's settings for this game are not in the format Heroic writes, so they were left alone.")?;
    game.insert("autoInstallDxvk".into(), s.dxvk.into());
    game.insert("useGameMode".into(), s.gamemode.into());
    game.insert("showMangohud".into(), s.overlay.into());
    game.insert("launcherArgs".into(), s.options.trim().into());
    game.insert("enviromentOptions".into(), env.iter().map(|(k, v)| serde_json::json!({ "key": k, "value": v })).collect::<Vec<_>>().into());
    serde_json::to_string_pretty(&v).map(|t| t + "\n").map_err(|e| e.to_string())
}

pub fn heroic_config_path(base: &Path, app: &str) -> Option<PathBuf> {
    let ok = !app.is_empty() && app.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.') && !app.contains("..");
    ok.then(|| base.join("GamesConfig").join(format!("{app}.json")))
}

pub fn heroic_apply(path: &Path, app: &str, s: &GameSettings, gpu_env: &[(String, String)]) -> Result<(), String> {
    if launcher_running("heroic") {
        return Err("Heroic is open. Close it completely (right-click its tray icon > Quit), then press Save again.".into());
    }
    let text = std::fs::read_to_string(path).unwrap_or_default();
    backup_then_write(path, &heroic_write(&text, app, s, gpu_env)?)
}

// ---------------------------------------------------------------------------------------------------------------
// Lutris (YAML, the part Lutris writes)
// ---------------------------------------------------------------------------------------------------------------

fn indent_of(l: &str) -> usize {
    l.len() - l.trim_start().len()
}

fn is_blank(l: &str) -> bool {
    let t = l.trim();
    t.is_empty() || t.starts_with('#')
}

/// A YAML plain or quoted scalar -> its text.
fn unquote(v: &str) -> String {
    let v = v.trim();
    if v.len() >= 2 && v.starts_with('\'') && v.ends_with('\'') {
        v[1..v.len() - 1].replace("''", "'")
    } else if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        v[1..v.len() - 1].replace("\\\"", "\"").replace("\\\\", "\\")
    } else {
        v.to_string()
    }
}

/// Text -> a YAML scalar that reads back the same.
fn quote(v: &str) -> String {
    let plain = !v.is_empty()
        && !v.starts_with(|c: char| " -?:,[]{}#&*!|>'\"%@`".contains(c))
        && !v.contains(": ")
        && !v.contains(" #")
        && !v.ends_with([' ', ':'])
        && !matches!(v.to_lowercase().as_str(), "true" | "false" | "yes" | "no" | "on" | "off" | "null" | "~")
        && v.parse::<f64>().is_err();
    if plain { v.to_string() } else { format!("'{}'", v.replace('\'', "''")) }
}

/// The lines of the top-level section `name`: (index of its header line, index after its last line).
fn section(lines: &[String], name: &str) -> Option<(usize, usize)> {
    let head = lines.iter().position(|l| indent_of(l) == 0 && (l == &format!("{name}:") || l.starts_with(&format!("{name}: "))))?;
    let end = lines[head + 1..].iter().position(|l| !is_blank(l) && indent_of(l) == 0).map(|i| head + 1 + i).unwrap_or(lines.len());
    Some((head, end))
}

/// `key: value` directly inside a section (one level in): (line index, value text).
fn find_key(lines: &[String], range: (usize, usize), key: &str) -> Option<(usize, String)> {
    let child = lines[range.0 + 1..range.1].iter().filter(|l| !is_blank(l)).map(|l| indent_of(l)).min()?;
    (range.0 + 1..range.1).find_map(|i| {
        let l = &lines[i];
        if is_blank(l) || indent_of(l) != child {
            return None;
        }
        let rest = l.trim_start().strip_prefix(&format!("{key}:"))?;
        (rest.is_empty() || rest.starts_with(' ')).then(|| (i, rest.trim().to_string()))
    })
}

/// The child indent of a section (2 when empty).
fn child_indent(lines: &[String], range: (usize, usize)) -> usize {
    lines[range.0 + 1..range.1].iter().filter(|l| !is_blank(l)).map(|l| indent_of(l)).min().unwrap_or(2)
}

fn as_bool(v: &str, default: bool) -> bool {
    match v.trim().to_lowercase().as_str() {
        "true" | "yes" | "on" => true,
        "false" | "no" | "off" => false,
        _ => default,
    }
}

/// `env:` children of `system:` as pairs. None = something we do not understand is in there.
fn lutris_env(lines: &[String], system: (usize, usize)) -> Option<Vec<(String, String)>> {
    let Some((i, rest)) = find_key(lines, system, "env") else { return Some(Vec::new()) };
    if rest == "{}" || rest.is_empty() && i + 1 >= system.1 {
        return Some(Vec::new());
    }
    if !rest.is_empty() {
        return None; // a one-line mapping: not what Lutris writes
    }
    let base = indent_of(&lines[i]);
    let mut out = Vec::new();
    for l in &lines[i + 1..system.1] {
        if is_blank(l) {
            continue;
        }
        if indent_of(l) <= base {
            break;
        }
        let (k, v) = l.trim().split_once(':')?;
        if v.starts_with(|c: char| !c.is_whitespace()) && !v.is_empty() {
            return None;
        }
        out.push((unquote(k), unquote(v)));
    }
    Some(out)
}

pub fn lutris_read(text: &str, gpu_env: &[(String, String)]) -> Option<GameSettings> {
    let lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
    let mut s = GameSettings::default();
    let flag = |sec: &str, key: &str, default: bool| -> bool {
        section(&lines, sec).and_then(|r| find_key(&lines, r, key)).map(|(_, v)| as_bool(&v, default)).unwrap_or(default)
    };
    s.gamemode = flag("system", "gamemode", false);
    s.overlay = flag("system", "mangohud", false);
    s.dxvk = flag("wine", "dxvk", true);
    s.options = section(&lines, "game").and_then(|r| find_key(&lines, r, "args")).map(|(_, v)| unquote(&v)).unwrap_or_default();
    if let Some(sys) = section(&lines, "system") {
        let (dedicated, env) = split_gpu(lutris_env(&lines, sys)?, gpu_env);
        s.dedicated_gpu = dedicated;
        s.env = env_text(&env);
    }
    Some(s)
}

/// Sets `key: value` in a top-level section (made when missing). `value` None removes the key.
fn set_key(lines: &mut Vec<String>, sec: &str, key: &str, value: Option<&str>) {
    let Some(range) = section(lines, sec) else {
        if let Some(v) = value {
            if lines.last().map(|l| !l.is_empty()).unwrap_or(false) {
                // sections follow each other directly in Lutris's files
            }
            lines.push(format!("{sec}:"));
            lines.push(format!("  {key}: {v}"));
        }
        return;
    };
    // `system: {}` (an empty section) becomes a block
    if lines[range.0].trim_end() != format!("{sec}:") {
        lines[range.0] = format!("{sec}:");
    }
    match (find_key(lines, range, key), value) {
        (Some((i, _)), Some(v)) => {
            let pad = " ".repeat(indent_of(&lines[i]));
            lines[i] = format!("{pad}{key}: {v}");
        }
        (Some((i, _)), None) => {
            lines.remove(i);
        }
        (None, Some(v)) => {
            let pad = " ".repeat(child_indent(lines, range));
            // after the section's last non-blank line
            let mut at = range.1;
            while at > range.0 + 1 && is_blank(&lines[at - 1]) {
                at -= 1;
            }
            lines.insert(at, format!("{pad}{key}: {v}"));
        }
        (None, None) => {}
    }
}

/// Replaces the whole `system: env:` mapping with `env`.
fn set_env(lines: &mut Vec<String>, env: &[(String, String)]) -> Result<(), String> {
    if env.is_empty() {
        if let Some(sys) = section(lines, "system") {
            if let Some((i, rest)) = find_key(lines, sys, "env") {
                if !rest.is_empty() && rest != "{}" {
                    return Err("odd env".into());
                }
                let base = indent_of(&lines[i]);
                let mut end = i + 1;
                while end < sys.1 && (is_blank(&lines[end]) || indent_of(&lines[end]) > base) {
                    end += 1;
                }
                lines.drain(i..end);
            }
        }
        return Ok(());
    }
    if section(lines, "system").is_none() {
        lines.push("system:".into());
    }
    let sys = section(lines, "system").unwrap();
    if lines[sys.0].trim_end() != "system:" {
        lines[sys.0] = "system:".into();
    }
    let sys = section(lines, "system").unwrap();
    let pad = " ".repeat(child_indent(lines, sys));
    let mut block = vec![format!("{pad}env:")];
    block.extend(env.iter().map(|(k, v)| format!("{pad}  {}: {}", quote(k), quote(v))));
    match find_key(lines, sys, "env") {
        Some((i, _)) => {
            let base = indent_of(&lines[i]);
            let mut end = i + 1;
            while end < sys.1 && (is_blank(&lines[end]) || indent_of(&lines[end]) > base) {
                end += 1;
            }
            lines.splice(i..end, block);
        }
        None => {
            let mut at = sys.1;
            while at > sys.0 + 1 && is_blank(&lines[at - 1]) {
                at -= 1;
            }
            for (n, b) in block.into_iter().enumerate() {
                lines.insert(at + n, b);
            }
        }
    }
    Ok(())
}

/// The file's text with the managed keys set. Err = the file is not in a shape we can edit safely.
pub fn lutris_write(text: &str, s: &GameSettings, gpu_env: &[(String, String)]) -> Result<String, String> {
    split_words(&s.options).ok_or("A quote in the game options is not closed.")?;
    let env = wanted_env(s, gpu_env)?;
    let refuse = || "Lutris's settings file for this game is not in the format Lutris writes, so it was left alone.".to_string();
    // what we cannot read back safely, we do not write over
    lutris_read(text, gpu_env).ok_or_else(refuse)?;
    if text.contains('\t') {
        return Err(refuse());
    }
    let mut lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
    let on = |b: bool| Some(if b { "true" } else { "false" });
    set_key(&mut lines, "system", "gamemode", on(s.gamemode));
    set_key(&mut lines, "system", "mangohud", on(s.overlay));
    set_env(&mut lines, &env).map_err(|_| refuse())?;
    let is_wine = lines.iter().any(|l| l.trim() == "runner: wine") || section(&lines, "wine").is_some();
    if is_wine {
        set_key(&mut lines, "wine", "dxvk", on(s.dxvk));
    }
    if s.options.trim().is_empty() {
        if section(&lines, "game").is_some() {
            set_key(&mut lines, "game", "args", None);
        }
    } else {
        set_key(&mut lines, "game", "args", Some(&quote(s.options.trim())));
    }
    Ok(lines.join("\n") + "\n")
}

pub fn lutris_apply(path: &Path, s: &GameSettings, gpu_env: &[(String, String)]) -> Result<(), String> {
    if launcher_running("lutris") {
        return Err("Lutris is open. Close it completely, then press Save again.".into());
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("Couldn't read {} ({e})", path.display()))?;
    backup_then_write(path, &lutris_write(&text, s, gpu_env)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gpu() -> Vec<(String, String)> {
        vec![("DRI_PRIME".into(), "1".into())]
    }

    const LUTRIS: &str = "game:\n  exe: /home/u/Games/Hades/Hades.exe\n  prefix: /home/u/Games/hades\nrunner: wine\nscript:\n  files:\n  - exe: https://example.org/x\nsystem:\n  env:\n    DXVK_HUD: fps\n  gamemode: true\nwine:\n  dxvk: true\n  version: lutris-7.2\n";

    #[test]
    fn lutris_settings_are_read() {
        let s = lutris_read(LUTRIS, &gpu()).unwrap();
        assert!(s.gamemode && !s.overlay && s.dxvk && !s.dedicated_gpu);
        assert_eq!(s.env, "DXVK_HUD=fps");
        assert_eq!(s.options, "");
    }

    #[test]
    fn lutris_changes_touch_only_what_they_manage() {
        let s = GameSettings { gamemode: false, overlay: true, dxvk: false, dedicated_gpu: true, options: "-windowed".into(), env: "DXVK_HUD=fps FOO=\"a b\"".into(), ..GameSettings::default() };
        let out = lutris_write(LUTRIS, &s, &gpu()).unwrap();
        // untouched parts are byte for byte the same
        assert!(out.contains("game:\n  exe: /home/u/Games/Hades/Hades.exe\n  prefix: /home/u/Games/hades\n"));
        assert!(out.contains("script:\n  files:\n  - exe: https://example.org/x\n"));
        assert!(out.contains("  version: lutris-7.2\n"));
        // changed parts
        assert!(out.contains("  gamemode: false\n") && out.contains("  mangohud: true\n") && out.contains("  dxvk: false\n"));
        assert!(out.contains("  args: '-windowed'\n"));
        assert!(out.contains("    DRI_PRIME: '1'\n") && out.contains("    FOO: a b\n") && out.contains("    DXVK_HUD: fps\n"));
        // and it reads back as what was chosen
        let back = lutris_read(&out, &gpu()).unwrap();
        assert_eq!((back.gamemode, back.overlay, back.dxvk, back.dedicated_gpu), (false, true, false, true));
        assert_eq!(back.options, "-windowed");
        assert_eq!(back.env, "DXVK_HUD=fps FOO=\"a b\"");
    }

    #[test]
    fn lutris_empty_sections_and_missing_sections_are_made() {
        let text = "game:\n  exe: /g/a\nrunner: linux\nsystem: {}\n";
        let s = GameSettings { gamemode: true, dedicated_gpu: true, ..GameSettings::default() };
        let out = lutris_write(text, &s, &gpu()).unwrap();
        assert!(out.contains("system:\n  gamemode: true\n"));
        assert!(!out.contains("wine:"), "a Linux game gets no Wine section");
        assert!(lutris_read(&out, &gpu()).unwrap().dedicated_gpu);
        // no system section at all
        let out2 = lutris_write("game:\n  exe: /g/a\n", &s, &gpu()).unwrap();
        assert!(out2.contains("system:\n  gamemode: true\n  mangohud: false\n"));
        // turning everything off again removes the variables and the options
        let back = lutris_write(&out, &GameSettings::default(), &gpu()).unwrap();
        assert!(!back.contains("DRI_PRIME") && !back.contains("env:"));
    }

    #[test]
    fn lutris_files_we_do_not_understand_are_left_alone() {
        assert!(lutris_write("system:\n\tenv: x\n", &GameSettings::default(), &gpu()).is_err());
        assert!(lutris_write("system:\n  env: {A: b}\n", &GameSettings::default(), &gpu()).is_err());
        assert!(lutris_write(LUTRIS, &GameSettings { env: "9bad=1".into(), ..GameSettings::default() }, &gpu()).is_err());
        assert!(lutris_write(LUTRIS, &GameSettings { options: "'open".into(), ..GameSettings::default() }, &gpu()).is_err());
    }

    #[test]
    fn heroic_keeps_other_keys_and_makes_a_fresh_file() {
        let text = r#"{"Hades": {"wineVersion": {"name": "GE-Proton"}, "showMangohud": false}, "version": "v0.1", "explicit": false}"#;
        let s = GameSettings { overlay: true, gamemode: true, dedicated_gpu: true, options: "--fullscreen".into(), env: "A=b".into(), dxvk: false, ..GameSettings::default() };
        let out = heroic_write(text, "Hades", &s, &gpu()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["Hades"]["wineVersion"]["name"], "GE-Proton", "other settings kept");
        assert_eq!(v["Hades"]["showMangohud"], true);
        assert_eq!(v["Hades"]["useGameMode"], true);
        assert_eq!(v["Hades"]["autoInstallDxvk"], false);
        assert_eq!(v["Hades"]["launcherArgs"], "--fullscreen");
        assert_eq!(v["Hades"]["enviromentOptions"][0], serde_json::json!({"key": "DRI_PRIME", "value": "1"}));
        assert_eq!(v["version"], "v0.1");
        let back = heroic_read(&out, "Hades", &gpu());
        assert_eq!((back.overlay, back.gamemode, back.dedicated_gpu, back.dxvk), (true, true, true, false));
        assert_eq!(back.env, "A=b");
        // a game that has no file yet
        let fresh = heroic_write("", "Foo", &GameSettings::default(), &gpu()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&fresh).unwrap();
        assert_eq!((v["version"].as_str(), v["explicit"].as_bool()), (Some("v0.1"), Some(false)));
        assert!(v["Foo"].is_object());
        assert!(heroic_write("[1,2]", "Foo", &GameSettings::default(), &gpu()).is_err());
        assert!(heroic_write("not json", "Foo", &GameSettings::default(), &gpu()).is_err());
    }

    #[test]
    fn heroic_app_names_make_safe_paths() {
        let b = Path::new("/h");
        assert!(heroic_config_path(b, "Fortnite_abc-1.2").is_some());
        assert!(heroic_config_path(b, "../x").is_none());
        assert!(heroic_config_path(b, "a/b").is_none());
        assert!(heroic_config_path(b, "").is_none());
    }
}

#[cfg(test)]
mod dump {
    use super::*;
    /// `cargo test dump_yaml -- --ignored --nocapture`: prints the edited files so a real YAML parser can check them.
    #[test]
    #[ignore]
    fn dump_yaml() {
        let gpu = vec![("DRI_PRIME".to_string(), "1".to_string())];
        let a = "game:\n  exe: /home/u/Games/Hades/Hades.exe\nrunner: wine\nsystem:\n  env:\n    DXVK_HUD: fps\n  gamemode: true\nwine:\n  dxvk: true\n";
        let s = GameSettings { overlay: true, dxvk: false, dedicated_gpu: true, options: "-windowed -w \"1 2\"".into(), env: "FOO=\"a b: c\" BAR=yes".into(), ..GameSettings::default() };
        println!("=====A\n{}", lutris_write(a, &s, &gpu).unwrap());
        println!("=====B\n{}", lutris_write("game:\n  exe: /g/a\nsystem: {}\n", &s, &gpu).unwrap());
    }
}
