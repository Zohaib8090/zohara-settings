//! Windows programs with a double-click (Gaming > Windows games > "Open Windows programs with a double-click").
//!
//! When it is on, the file manager opens `.exe`, `.bat`, `.msi` and Windows shortcuts with `zohara-windows.desktop`,
//! which runs `zohara-settings --run-windows <file>`. That builds the launch line from the defaults chosen in Settings
//! (Proton-GE or plain Wine, DXVK, graphics chip, Game Mode, overlay) and starts the program from its own folder. No
//! Steam is involved: Proton-GE is run directly, with one shared Windows folder (`~/.local/share/zohara/windows`, its own
//! C: drive), so a program that an installer put there can find what it installed.
//!
//! On/off is the person's own file-type choice (`~/.config/mimeapps.list`, written with `xdg-mime`); turning it off
//! removes our entry again, so Wine's own handler is used as before. No administrator rights are needed.

use super::game_launch::{self, Context, GameSettings, Runner};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const DESKTOP_ID: &str = "zohara-windows.desktop";
/// The file types Windows programs have (the same list the `wine` package registers, plus the newer .exe type).
pub const MIME_TYPES: [&str; 6] = [
    "application/x-ms-dos-executable",
    "application/x-msdownload",
    "application/x-msi",
    "application/x-ms-shortcut",
    "application/x-bat",
    "application/x-mswinurl",
];

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
}

fn config_home() -> PathBuf {
    std::env::var("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|_| home().join(".config"))
}

fn data_home() -> PathBuf {
    std::env::var("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|_| home().join(".local/share"))
}

pub fn config_path() -> PathBuf {
    config_home().join("zohara/windows-apps.conf")
}

/// Proton's Windows folder (Proton keeps the Windows files in `pfx/` inside it).
pub fn proton_folder() -> PathBuf {
    data_home().join("zohara/windows")
}

/// Plain Wine's Windows folder for double-clicked programs (kept apart from Proton's: the two must not share one).
pub fn wine_folder() -> PathBuf {
    data_home().join("zohara/windows-wine")
}

/// The C: drive of whichever layer is chosen, for "Open the Windows C: drive".
pub fn c_drive(s: &GameSettings) -> PathBuf {
    if s.runner == Runner::Proton { proton_folder().join("pfx/drive_c") } else { wine_folder().join("drive_c") }
}

/// Defaults: Proton-GE with DXVK, the system's choice of graphics chip, Game Mode on when it is installed.
pub fn defaults() -> GameSettings {
    GameSettings { runner: Runner::Proton, dxvk: true, gamemode: false, overlay: false, dedicated_gpu: false, options: String::new(), env: String::new() }
}

/// `key=value` lines -> settings. Unknown or missing keys keep the defaults.
pub fn parse_config(text: &str) -> GameSettings {
    let mut s = defaults();
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        let on = v.trim() == "true";
        match k.trim() {
            "layer" => s.runner = if v.trim() == "wine" { Runner::Wine } else { Runner::Proton },
            "dxvk" => s.dxvk = on,
            "gamemode" => s.gamemode = on,
            "overlay" => s.overlay = on,
            "dedicated_gpu" => s.dedicated_gpu = on,
            _ => {}
        }
    }
    s
}

pub fn config_text(s: &GameSettings) -> String {
    format!(
        "# Zohara Settings > Gaming > Windows games: how double-clicked Windows programs run.\nlayer={}\ndxvk={}\ngamemode={}\noverlay={}\ndedicated_gpu={}\n",
        if s.runner == Runner::Wine { "wine" } else { "proton" },
        s.dxvk,
        s.gamemode,
        s.overlay,
        s.dedicated_gpu
    )
}

pub fn load() -> GameSettings {
    std::fs::read_to_string(config_path()).map(|t| parse_config(&t)).unwrap_or_else(|_| defaults())
}

pub fn save(s: &GameSettings) -> Result<(), String> {
    let p = config_path();
    std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&p, config_text(s)).map_err(|e| e.to_string())
}

// ── Running a double-clicked program ───────────────────────────────────────

fn notify(title: &str, body: &str) {
    let _ = Command::new("notify-send").args(["-a", "Zohara", "-i", "applications-games", title, body]).status();
}

/// The words that start `path` with the chosen defaults. When Proton-GE is chosen but not installed, plain Wine is used
/// and the second value says so.
pub fn launch_words(path: &str, s: &GameSettings, ctx: &Context) -> Result<(Vec<String>, bool), String> {
    let mut s = s.clone();
    let mut fell_back = false;
    if s.runner == Runner::Proton && ctx.proton_dir.is_none() {
        s.runner = Runner::Wine;
        fell_back = true;
    }
    if s.runner != Runner::Proton {
        s.runner = Runner::Wine; // a double-clicked Windows file always goes to a Windows layer
    }
    Ok((game_launch::argv(path, &s, ctx)?, fell_back))
}

/// `zohara-settings --run-windows FILE`: starts the program and returns at once (exit code for the file manager).
pub fn run_file(path: &str) -> i32 {
    let p = Path::new(path);
    if !p.is_file() {
        notify("Can't open this Windows program", &format!("{path} was not found."));
        return 1;
    }
    let s = load();
    let ctx = Context::here(proton_folder(), wine_folder(), s.dedicated_gpu);
    let (words, fell_back) = match launch_words(path, &s, &ctx) {
        Ok(w) => w,
        Err(e) => {
            notify("Can't open this Windows program", &e);
            return 1;
        }
    };
    if fell_back {
        notify("Running with Wine", "Proton-GE isn't installed yet, so plain Wine is used. Install it in Settings > Gaming > Windows games.");
    }
    let _ = std::fs::create_dir_all(proton_folder());
    let _ = std::fs::create_dir_all(wine_folder());
    // Proton needs this folder to exist even when there is no Steam.
    let _ = std::fs::create_dir_all(&ctx.steam_root);
    let log_dir = std::env::var("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|_| home().join(".cache")).join("zohara");
    let _ = std::fs::create_dir_all(&log_dir);
    let log = std::fs::File::create(log_dir.join("windows-run.log")).ok();
    let mut cmd = Command::new(&words[0]);
    cmd.args(&words[1..]).current_dir(p.parent().unwrap_or(Path::new("/"))).stdin(Stdio::null());
    if let Some(l) = log {
        if let Ok(l2) = l.try_clone() {
            cmd.stdout(l).stderr(l2);
        }
    }
    match cmd.spawn() {
        Ok(_) => 0,
        Err(e) => {
            notify("Can't open this Windows program", &format!("{}: {e}", words[0]));
            1
        }
    }
}

// ── The file-type switch ───────────────────────────────────────────────────

pub fn desktop_entry(exe: &str) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=Windows program (Zohara)\nComment=Run a Windows program with Proton-GE or Wine\nExec={} --run-windows %f\nIcon=applications-games\nTerminal=false\nNoDisplay=true\nMimeType={};\n",
        game_launch::exec_quote(exe),
        MIME_TYPES.join(";")
    )
}

fn entry_path() -> PathBuf {
    data_home().join("applications").join(DESKTOP_ID)
}

fn mimeapps_path() -> PathBuf {
    config_home().join("mimeapps.list")
}

/// The text of `mimeapps.list` with `id` taken out of every list, and keys left empty removed.
pub fn without_handler(text: &str, id: &str) -> String {
    let mut out = Vec::new();
    for line in text.lines() {
        if let Some((k, v)) = line.split_once('=') {
            if !line.trim_start().starts_with('[') && v.split(';').any(|x| x.trim() == id) {
                let kept: Vec<&str> = v.split(';').map(str::trim).filter(|x| !x.is_empty() && *x != id).collect();
                if !kept.is_empty() {
                    out.push(format!("{k}={};", kept.join(";")));
                }
                continue;
            }
        }
        out.push(line.to_string());
    }
    let mut t = out.join("\n");
    if text.ends_with('\n') {
        t.push('\n');
    }
    t
}

pub fn enabled() -> bool {
    std::fs::read_to_string(mimeapps_path())
        .map(|t| t.lines().any(|l| l.starts_with("application/x-ms-dos-executable=") && l.split('=').nth(1).is_some_and(|v| v.split(';').next() == Some(DESKTOP_ID))))
        .unwrap_or(false)
        && entry_path().exists()
}

fn refresh_menu() {
    let _ = Command::new("update-desktop-database").arg(data_home().join("applications")).status();
    let _ = Command::new("kbuildsycoca6").stdout(Stdio::null()).stderr(Stdio::null()).status();
}

pub fn set_enabled(on: bool) -> Result<(), String> {
    if on {
        let exe = std::env::current_exe().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| "zohara-settings".into());
        // The installed program, not a copy that was run from somewhere else for a test.
        let exe = if Path::new("/usr/bin/zohara-settings").exists() && !exe.contains("/tmp/") { "/usr/bin/zohara-settings".to_string() } else { exe };
        let p = entry_path();
        std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&p, desktop_entry(&exe)).map_err(|e| e.to_string())?;
        refresh_menu();
        let ok = Command::new("xdg-mime").arg("default").arg(DESKTOP_ID).args(MIME_TYPES).status().map(|s| s.success()).unwrap_or(false);
        if !ok {
            return Err("Couldn't set the file types (xdg-mime failed).".into());
        }
    } else {
        let m = mimeapps_path();
        if let Ok(t) = std::fs::read_to_string(&m) {
            std::fs::write(&m, without_handler(&t, DESKTOP_ID)).map_err(|e| e.to_string())?;
        }
        let _ = std::fs::remove_file(entry_path());
        refresh_menu();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(proton: bool) -> Context {
        Context {
            proton_dir: proton.then(|| PathBuf::from("/h/ct/GE-Proton11-9")),
            steam_root: PathBuf::from("/h/.local/share/Steam"),
            prefix_dir: PathBuf::from("/h/.local/share/zohara/windows"),
            gpu_env: Vec::new(),
            wine_prefix: PathBuf::from("/h/.local/share/zohara/windows-wine"),
        }
    }

    #[test]
    fn settings_are_saved_and_read_back() {
        let s = GameSettings { runner: Runner::Wine, dxvk: false, gamemode: true, overlay: true, dedicated_gpu: true, ..defaults() };
        assert_eq!(parse_config(&config_text(&s)), s);
        assert_eq!(parse_config(""), defaults());
        assert_eq!(parse_config("layer=something\nnoise\n").runner, Runner::Proton);
    }

    #[test]
    fn a_double_clicked_file_runs_with_proton_from_the_shared_folder() {
        let (w, fell) = launch_words("/d/Game/setup.exe", &defaults(), &ctx(true)).unwrap();
        assert!(!fell);
        assert_eq!(w.last().unwrap(), "/d/Game/setup.exe");
        assert!(w.contains(&"STEAM_COMPAT_DATA_PATH=/h/.local/share/zohara/windows".to_string()));
        assert!(w.contains(&"/h/ct/GE-Proton11-9/proton".to_string()));
        let (b, _) = launch_words("/d/Game/Run Me!.bat", &defaults(), &ctx(true)).unwrap();
        assert!(b.ends_with(&["run".into(), "start".into(), "/unix".into(), "/d/Game/Run Me!.bat".into()]));
    }

    #[test]
    fn without_proton_it_falls_back_to_wine_in_its_own_folder() {
        let (w, fell) = launch_words("/d/a.exe", &defaults(), &ctx(false)).unwrap();
        assert!(fell);
        assert_eq!(w, vec!["env", "WINEPREFIX=/h/.local/share/zohara/windows-wine", "wine", "/d/a.exe"]);
        // Wine chosen on purpose: no warning
        let wine = GameSettings { runner: Runner::Wine, ..defaults() };
        assert!(!launch_words("/d/a.exe", &wine, &ctx(true)).unwrap().1);
    }

    #[test]
    fn turning_it_off_takes_only_our_handler_out() {
        let t = "[Default Applications]\napplication/x-ms-dos-executable=zohara-windows.desktop;\napplication/x-bat=zohara-windows.desktop;wine.desktop;\ntext/plain=org.kde.kate.desktop;\n\n[Added Associations]\napplication/x-msi=wine.desktop;zohara-windows.desktop;\n";
        let out = without_handler(t, DESKTOP_ID);
        assert_eq!(out, "[Default Applications]\napplication/x-bat=wine.desktop;\ntext/plain=org.kde.kate.desktop;\n\n[Added Associations]\napplication/x-msi=wine.desktop;\n");
        assert_eq!(without_handler("", DESKTOP_ID), "");
    }

    #[test]
    fn the_desktop_entry_handles_every_windows_type() {
        let e = desktop_entry("/usr/bin/zohara-settings");
        assert!(e.contains("Exec=\"/usr/bin/zohara-settings\" --run-windows %f\n"));
        for m in MIME_TYPES {
            assert!(e.contains(m));
        }
        assert!(e.contains("NoDisplay=true"));
    }
}
