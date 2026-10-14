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
    // the Safety mode line is kept
    let keep = sandbox_default();
    std::fs::write(&p, format!("{}sandbox={keep}\n", config_text(s))).map_err(|e| e.to_string())
}

/// Safety mode for programs that are not one of My games: on unless the person turned it off.
pub fn sandbox_in(text: &str) -> bool {
    !text.lines().any(|l| l.trim() == "sandbox=false")
}

pub fn sandbox_default() -> bool {
    std::fs::read_to_string(config_path()).map(|t| sandbox_in(&t)).unwrap_or(true)
}

pub fn set_sandbox_default(on: bool) -> Result<(), String> {
    let p = config_path();
    let text = std::fs::read_to_string(&p).unwrap_or_else(|_| config_text(&defaults()));
    let mut lines: Vec<String> = text.lines().filter(|l| !l.trim().starts_with("sandbox=")).map(|l| l.to_string()).collect();
    lines.push(format!("sandbox={on}"));
    std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&p, lines.join("\n") + "\n").map_err(|e| e.to_string())
}

// ── Matching a double-clicked file to a game in My games ───────────────────

/// A game added in Gaming > My games, as far as matching needs it.
#[derive(Clone, Debug, PartialEq)]
pub struct KnownGame {
    pub id: String,
    pub name: String,
    /// The program file that was added.
    pub path: String,
    pub settings: GameSettings,
    /// Safety mode asked for by this game.
    pub sandbox: bool,
}

/// The games in `dir` (the user's applications folder) that have settings.
pub fn known_games(dir: &Path) -> Vec<KnownGame> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut v: Vec<KnownGame> = rd
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let id = p.file_name()?.to_string_lossy().strip_prefix("zohara-game-")?.strip_suffix(".desktop")?.to_string();
            let text = std::fs::read_to_string(&p).ok()?;
            let (path, settings) = game_launch::parse_entry(&text)?;
            Some(KnownGame { id, name: game_launch::entry_name(&text).unwrap_or_else(|| "this game".into()), path, settings, sandbox: game_launch::sandbox_of(&text) })
        })
        .collect();
    v.sort_by(|a, b| a.id.cmp(&b.id));
    v
}

/// Folders that hold many unrelated programs: a game added from one of them matches by its own file only.
fn too_broad(dir: &Path, home: &Path) -> bool {
    dir == Path::new("/")
        || dir == home
        || ["Downloads", "Desktop", "Documents", "Music", "Pictures", "Videos", "Public"].iter().any(|d| dir == home.join(d))
}

/// Which game a double-clicked `file` belongs to: the game whose added file it is, else the one whose folder holds it
/// (the closest folder wins; a broad folder like Downloads never matches by folder).
pub fn match_game<'a>(file: &str, games: &'a [KnownGame], home: &Path) -> Option<&'a KnownGame> {
    if let Some(g) = games.iter().find(|g| g.path == file) {
        return Some(g);
    }
    let f = Path::new(file);
    games
        .iter()
        .filter_map(|g| {
            let dir = Path::new(&g.path).parent()?;
            (!too_broad(dir, home) && f.starts_with(dir)).then_some((dir.components().count(), g))
        })
        .max_by_key(|(depth, _)| *depth)
        .map(|(_, g)| g)
}

// ── Running a double-clicked program ───────────────────────────────────────

fn notify(title: &str, body: &str) {
    let _ = Command::new("notify-send").args(["-a", "Zohara", "-i", "applications-games", title, body]).status();
}

/// A notification with one button. True = the person pressed it. Waits for the answer (up to `secs`).
fn notify_action(title: &str, body: &str, label: &str, secs: u32) -> bool {
    Command::new("notify-send")
        .args(["-a", "Zohara", "-i", "applications-games", "-t", &(secs * 1000).to_string(), "-A", &format!("go={label}"), title, body])
        .stdin(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "go")
        .unwrap_or(false)
}

/// How long after the start a program that quits with an error still counts as "did not start".
pub const QUICK_FAIL_SECS: u64 = 10;

/// Whether a program that ended after `secs` with `code` (None = ended by a signal, for example the person closing it)
/// should be reported as failed.
pub fn failed_to_start(code: Option<i32>, secs: u64) -> bool {
    matches!(code, Some(c) if c != 0) && secs < QUICK_FAIL_SECS
}

/// The file a program's output goes to: one per game, one for everything else.
pub fn log_path(game_id: Option<&str>) -> PathBuf {
    let dir = std::env::var("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|_| home().join(".cache")).join("zohara/windows");
    let name: String = game_id.unwrap_or("shared").chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    dir.join(format!("{name}.log"))
}

fn open_log(path: &Path) {
    let _ = Command::new("xdg-open").arg(path).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status();
}

fn open_gaming_settings() {
    let exe = std::env::current_exe().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| "zohara-settings".into());
    let _ = Command::new(exe).args(["--page", "Gaming"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
}

/// What Safety mode lets a program reach: its own Windows folders, Proton-GE, and its own program folder.
pub fn sandbox_plan(path: &str, ctx: &Context) -> super::windows_sandbox::Plan {
    let home = home();
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|_| {
        use std::os::unix::fs::MetadataExt;
        PathBuf::from(format!("/run/user/{}", std::fs::metadata("/proc/self").map(|m| m.uid()).unwrap_or(1000)))
    });
    let program_dir = Path::new(path).parent().unwrap_or(Path::new("/")).to_path_buf();
    let wayland = std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| "wayland-0".into());
    let mut writable = vec![ctx.prefix_dir.clone()];
    if !ctx.wine_prefix.as_os_str().is_empty() {
        writable.push(ctx.wine_prefix.clone());
    }
    super::windows_sandbox::Plan {
        program_dir_writable: !too_broad(&program_dir, &home),
        program_dir,
        writable,
        readable: ctx.proton_dir.iter().cloned().collect(),
        empty_dirs: vec![ctx.steam_root.clone()],
        no_network: false,
        display_files: vec![std::env::var("XAUTHORITY").map(PathBuf::from).unwrap_or_else(|_| home.join(".Xauthority"))],
        sockets: vec![runtime_dir.join(wayland), runtime_dir.join("pipewire-0"), runtime_dir.join("pulse")],
        runtime_dir,
        home,
    }
}

/// The words that start `path`. `s` is the matched game's settings or the shared defaults. When Proton-GE is chosen but
/// not installed, plain Wine is used and the second value says so.
pub fn launch_words(path: &str, s: &GameSettings, ctx: &Context) -> Result<(Vec<String>, bool), String> {
    let mut s = s.clone();
    let mut fell_back = false;
    if s.runner == Runner::Proton && ctx.proton_dir.is_none() {
        s.runner = Runner::Wine;
        fell_back = true;
    }
    if s.runner == Runner::Auto {
        s.runner = Runner::Wine; // a double-clicked Windows file always goes to a Windows layer
    }
    Ok((game_launch::argv(path, &s, ctx)?, fell_back))
}

/// The Windows folders (Proton's, plain Wine's) for a program. A program an installer put into the shared Windows folder
/// has to keep running in it (its settings, libraries and saves are there); anything else gets a folder of its own.
pub fn prefixes_for(path: &str, game_id: &str) -> (PathBuf, PathBuf) {
    prefixes_in(&data_home(), path, game_id)
}

pub fn prefixes_in(data: &Path, path: &str, game_id: &str) -> (PathBuf, PathBuf) {
    let shared_proton = data.join("zohara/windows");
    let shared_wine = data.join("zohara/windows-wine");
    let own = data.join("zohara/prefixes").join(game_id);
    if Path::new(path).starts_with(&shared_proton) {
        // plain Wine (when Proton-GE is missing) can open Proton's folder too: its `pfx` is an ordinary Wine prefix
        let wine = shared_proton.join("pfx");
        (shared_proton, wine)
    } else if Path::new(path).starts_with(&shared_wine) {
        (own, shared_wine)
    } else {
        (own, PathBuf::new())
    }
}

/// A choice for this one start only (right-click menu), over the game's or the shared settings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Once {
    pub runner: Option<Runner>,
    pub dedicated: bool,
    /// Safety mode for this start: Some(true) = on, Some(false) = off, None = as set.
    pub sandbox: Option<bool>,
}

/// Reads `--layer proton|wine` and `--dedicated` from the command line.
pub fn once_from_args(args: &[String]) -> Once {
    let runner = args.iter().position(|a| a == "--layer").and_then(|i| args.get(i + 1)).and_then(|v| match v.as_str() {
        "proton" => Some(Runner::Proton),
        "wine" => Some(Runner::Wine),
        _ => None,
    });
    let sandbox = if args.iter().any(|a| a == "--safe") {
        Some(true)
    } else if args.iter().any(|a| a == "--unsafe") {
        Some(false)
    } else {
        None
    };
    Once { runner, dedicated: args.iter().any(|a| a == "--dedicated"), sandbox }
}

/// `zohara-settings --run-windows FILE`: starts the program and returns at once (exit code for the file manager).
pub fn run_file(path: &str) -> i32 {
    run_file_once(path, &Once::default())
}

pub fn run_file_once(path: &str, once: &Once) -> i32 {
    let p = Path::new(path);
    if !p.is_file() {
        notify("Can't open this Windows program", &format!("{path} was not found."));
        return 1;
    }
    // A game from My games keeps its own settings and its own Windows folder, the same as its Play button.
    let games = known_games(&data_home().join("applications"));
    let (mut s, mut ctx, game) = match match_game(path, &games, &home()) {
        Some(g) => {
            let (prefix, wine) = prefixes_for(path, &g.id);
            let ctx = Context::here(prefix, wine, g.settings.dedicated_gpu);
            (g.settings.clone(), ctx, Some((g.id.clone(), g.name.clone(), g.sandbox)))
        }
        None => {
            let s = load();
            let ctx = Context::here(proton_folder(), wine_folder(), s.dedicated_gpu);
            (s, ctx, None)
        }
    };
    if let Some(r) = once.runner {
        s.runner = r;
    }
    if once.dedicated {
        s.dedicated_gpu = true;
        ctx.gpu_env = super::gpu_pref::dedicated_env();
    }
    let (words, fell_back) = match launch_words(path, &s, &ctx) {
        Ok(w) => w,
        Err(e) => {
            notify("Can't open this Windows program", &e);
            return 1;
        }
    };
    let _ = std::fs::create_dir_all(&ctx.prefix_dir);
    if !ctx.wine_prefix.as_os_str().is_empty() {
        let _ = std::fs::create_dir_all(&ctx.wine_prefix);
    }
    // Proton needs this folder to exist even when there is no Steam.
    let _ = std::fs::create_dir_all(&ctx.steam_root);
    // Safety mode: a game of My games as that game chose, anything else as the shared setting says.
    let mut sandboxed = once.sandbox.unwrap_or_else(|| game.as_ref().map(|(_, _, sb)| *sb).unwrap_or_else(sandbox_default));
    if sandboxed && !super::windows_sandbox::available() {
        if !notify_action("Safety mode isn't available", "This program would run inside a protected space, but this computer can't make one. Run it without protection?", "Run without it", 30) {
            return 1;
        }
        sandboxed = false;
    }
    let words = if sandboxed { super::windows_sandbox::wrap(words, &sandbox_plan(path, &ctx)) } else { words };
    let log_file = log_path(game.as_ref().map(|(id, _, _)| id.as_str()));
    let _ = std::fs::create_dir_all(log_file.parent().unwrap());
    let log = std::fs::File::create(&log_file).ok();
    let label = game.as_ref().map(|(_, n, _)| n.clone()).unwrap_or_else(|| p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
    let mut cmd = Command::new(&words[0]);
    cmd.args(&words[1..]).current_dir(p.parent().unwrap_or(Path::new("/"))).stdin(Stdio::null());
    if let Some(mut l) = log {
        use std::io::Write;
        let _ = writeln!(l, "Zohara: {}{} with {}", game.as_ref().map(|(_, n, _)| format!("game \"{n}\"")).unwrap_or_else(|| "shared defaults".into()), if sandboxed { ", Safety mode" } else { "" }, words.join(" "));
        if let Ok(l2) = l.try_clone() {
            cmd.stdout(l).stderr(l2);
        }
    }
    let started = std::time::Instant::now();
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let what = if words[0] == "wine" { "Windows app support (Wine) isn't installed.".to_string() } else { format!("{}: {e}", words[0]) };
            if notify_action("Can't open this Windows program", &what, "Open Gaming settings", 20) {
                open_gaming_settings();
            }
            return 1;
        }
    };
    if fell_back && notify_action("Running with Wine", "Proton-GE isn't installed yet, so plain Wine is used. Many games run better with Proton-GE.", "Install Proton-GE", 15) {
        open_gaming_settings();
    }
    // Watch the first seconds: a program that quits at once with an error did not start, and the person should hear why.
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if failed_to_start(status.code(), started.elapsed().as_secs()) {
                    let mut body = format!("{label} closed right away (error {}).", status.code().unwrap_or(-1));
                    if sandboxed {
                        body.push_str(" Safety mode was on; some programs need it off (Settings > Gaming).");
                    }
                    if notify_action("A Windows program did not start", &body, "Open the log", 30) {
                        open_log(&log_file);
                    }
                    return 1;
                }
                return 0;
            }
            Ok(None) if started.elapsed().as_secs() >= QUICK_FAIL_SECS => return 0,
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(250)),
            Err(_) => return 0,
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

/// Where the package puts the handler and the default that points the Windows file types at it.
pub const SYSTEM_ENTRY: &str = "/usr/share/applications/zohara-windows.desktop";
pub const SYSTEM_DEFAULTS: &str = "/usr/share/applications/mimeapps.list";

/// The `[Default Applications]` value of `key` in a `mimeapps.list` text (the first program of the list).
fn default_in(text: &str, key: &str) -> Option<String> {
    let mut in_defaults = false;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_defaults = l == "[Default Applications]";
        } else if in_defaults {
            if let Some((k, v)) = l.split_once('=') {
                if k.trim() == key {
                    return Some(v.split(';').next().unwrap_or("").trim().to_string());
                }
            }
        }
    }
    None
}

/// Whether double-clicking a Windows file runs it here. The person's own choice wins; with none, the system default
/// (shipped by the package) decides, so a fresh install works without anyone finding the switch.
pub fn decide(user_mimeapps: &str, system_mimeapps: &str, entry_exists: bool) -> bool {
    if !entry_exists {
        return false;
    }
    let key = MIME_TYPES[0];
    match default_in(user_mimeapps, key) {
        Some(v) => v == DESKTOP_ID,
        None => default_in(system_mimeapps, key).as_deref() == Some(DESKTOP_ID),
    }
}

/// `mimeapps.list` with the six Windows file types set to `id` (None = the lines removed, so the next list decides).
pub fn with_defaults(text: &str, id: Option<&str>) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut in_defaults = false;
    let mut had_section = false;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            if in_defaults {
                // end of [Default Applications]: the new lines go here
                if let Some(id) = id {
                    out.extend(MIME_TYPES.iter().map(|m| format!("{m}={id};")));
                }
            }
            in_defaults = l == "[Default Applications]";
            had_section |= in_defaults;
            out.push(line.to_string());
            continue;
        }
        if in_defaults && l.split_once('=').is_some_and(|(k, _)| MIME_TYPES.contains(&k.trim())) {
            continue;
        }
        out.push(line.to_string());
    }
    if in_defaults {
        if let Some(id) = id {
            out.extend(MIME_TYPES.iter().map(|m| format!("{m}={id};")));
        }
    } else if !had_section {
        if let Some(id) = id {
            if !out.is_empty() && !out.last().unwrap().is_empty() {
                out.push(String::new());
            }
            out.push("[Default Applications]".into());
            out.extend(MIME_TYPES.iter().map(|m| format!("{m}={id};")));
        }
    }
    let mut t = out.join("\n");
    if !t.is_empty() {
        t.push('\n');
    }
    t
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_default()
}

fn system_default_text() -> String {
    read(Path::new(SYSTEM_DEFAULTS))
}

pub fn enabled() -> bool {
    decide(&read(&mimeapps_path()), &system_default_text(), entry_path().exists() || Path::new(SYSTEM_ENTRY).exists())
}

fn refresh_menu() {
    let _ = Command::new("update-desktop-database").arg(data_home().join("applications")).status();
    let _ = Command::new("kbuildsycoca6").stdout(Stdio::null()).stderr(Stdio::null()).status();
}

fn write_user_mimeapps(id: Option<&str>) -> Result<(), String> {
    let m = mimeapps_path();
    std::fs::create_dir_all(m.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&m, with_defaults(&read(&m), id)).map_err(|e| e.to_string())
}

pub fn set_enabled(on: bool) -> Result<(), String> {
    if on {
        // The package's handler (installed program, always current) when it is there; the person's own copy would
        // shadow it, and could point at a program that has since moved.
        if Path::new(SYSTEM_ENTRY).exists() && default_in(&system_default_text(), MIME_TYPES[0]).as_deref() == Some(DESKTOP_ID) {
            let _ = std::fs::remove_file(entry_path());
            write_user_mimeapps(None)?;
            refresh_menu();
            return Ok(());
        }
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
        // Off is the person's own choice, so it has to beat the system default: Wine's handler is named for each type.
        write_user_mimeapps(Some("wine.desktop"))?;
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

    fn game(id: &str, path: &str) -> KnownGame {
        KnownGame { id: id.into(), name: id.into(), path: path.into(), settings: GameSettings { dedicated_gpu: true, ..defaults() }, sandbox: false }
    }

    #[test]
    fn a_double_clicked_file_finds_its_game_by_file_then_by_folder() {
        let home = Path::new("/home/me");
        let games = vec![
            game("city", "/home/me/Games/City/Run Me!.bat"),
            game("city-mod", "/home/me/Games/City/Mods/Tool.bat"),
            game("dl", "/home/me/Downloads/setup.exe"),
        ];
        // the exact file that was added
        assert_eq!(match_game("/home/me/Games/City/Run Me!.bat", &games, home).unwrap().id, "city");
        // another program in the game's folder
        assert_eq!(match_game("/home/me/Games/City/Cities_Skylines/Cities.exe", &games, home).unwrap().id, "city");
        // the closest folder wins
        assert_eq!(match_game("/home/me/Games/City/Mods/x/y.exe", &games, home).unwrap().id, "city-mod");
        // a folder like Downloads only matches the exact file
        assert_eq!(match_game("/home/me/Downloads/setup.exe", &games, home).unwrap().id, "dl");
        assert!(match_game("/home/me/Downloads/other.exe", &games, home).is_none());
        // a different game's folder with a similar name does not match
        assert!(match_game("/home/me/Games/City2/a.exe", &games, home).is_none());
        assert!(match_game("/anywhere/a.exe", &[], home).is_none());
    }

    #[test]
    fn games_are_read_from_their_desktop_entries() {
        let dir = std::env::temp_dir().join(format!("zs-known-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let s = GameSettings { runner: Runner::Proton, dedicated_gpu: true, ..defaults() };
        let text = game_launch::entry_text("City Skylines", "/g/City/Run Me!.bat", &s, &Context::default());
        std::fs::write(dir.join("zohara-game-city-skylines.desktop"), text.unwrap_or_else(|_| {
            // Proton is not installed in the test: build the entry with Wine, then mark it as Proton
            let t = game_launch::entry_text("City Skylines", "/g/City/Run Me!.bat", &GameSettings { runner: Runner::Wine, dedicated_gpu: true, ..defaults() }, &Context::default()).unwrap();
            t.replace("X-Zohara-Runner=wine", "X-Zohara-Runner=proton")
        })).unwrap();
        std::fs::write(dir.join("other.desktop"), "[Desktop Entry]\nName=Other\nX-Zohara-Game=true\n").unwrap(); // marked app: no settings
        let g = known_games(&dir);
        assert_eq!(g.len(), 1);
        assert_eq!((g[0].id.as_str(), g[0].name.as_str(), g[0].path.as_str()), ("city-skylines", "City Skylines", "/g/City/Run Me!.bat"));
        assert!(g[0].settings.dedicated_gpu && g[0].settings.runner == Runner::Proton);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_matched_game_uses_its_own_windows_folder_and_chip() {
        let g = game("city", "/g/City/Run Me!.bat");
        let c = Context { prefix_dir: PathBuf::from("/h/.local/share/zohara/prefixes/city"), gpu_env: vec![("__NV_PRIME_RENDER_OFFLOAD".into(), "1".into())], ..ctx(true) };
        let (w, _) = launch_words("/g/City/Cities.exe", &g.settings, &c).unwrap();
        assert!(w.contains(&"STEAM_COMPAT_DATA_PATH=/h/.local/share/zohara/prefixes/city".to_string()));
        assert!(w.contains(&"__NV_PRIME_RENDER_OFFLOAD=1".to_string()));
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

    #[test]
    fn the_system_default_decides_until_the_person_chooses() {
        let sys = "[Default Applications]\napplication/x-ms-dos-executable=zohara-windows.desktop;\n";
        assert!(decide("", sys, true), "fresh install: on");
        assert!(!decide("", sys, false), "no handler installed: off");
        assert!(!decide("", "", true), "no system default: off");
        assert!(!decide("[Default Applications]\napplication/x-ms-dos-executable=wine.desktop;\n", sys, true), "turned off by the person");
        assert!(decide("[Default Applications]\napplication/x-ms-dos-executable=zohara-windows.desktop;\n", "", true));
    }

    #[test]
    fn the_six_types_are_set_and_removed_without_touching_the_rest() {
        let user = "[Default Applications]\ntext/html=chrome.desktop;\napplication/x-bat=old.desktop;\n\n[Added Associations]\ntext/html=chrome.desktop;\n";
        let off = with_defaults(user, Some("wine.desktop"));
        assert!(off.contains("text/html=chrome.desktop;"));
        assert!(!off.contains("old.desktop"));
        for m in MIME_TYPES {
            assert!(off.contains(&format!("{m}=wine.desktop;")), "{m}");
        }
        assert!(off.contains("[Added Associations]\ntext/html=chrome.desktop;"), "later sections kept");
        let on = with_defaults(&off, None);
        assert_eq!(on, "[Default Applications]\ntext/html=chrome.desktop;\n\n[Added Associations]\ntext/html=chrome.desktop;\n");
        // no file at all, and no section yet
        assert!(with_defaults("", Some("wine.desktop")).starts_with("[Default Applications]\n"));
        assert_eq!(with_defaults("", None), "");
        assert!(with_defaults("[Added Associations]\na=b;\n", Some("x.desktop")).ends_with("application/x-mswinurl=x.desktop;\n"));
    }

    #[test]
    fn only_a_quick_error_exit_counts_as_failed() {
        assert!(failed_to_start(Some(1), 2));
        assert!(!failed_to_start(Some(0), 2), "a clean quick exit is normal (installers, launchers)");
        assert!(!failed_to_start(Some(1), 60), "a crash after a long play is not 'did not start'");
        assert!(!failed_to_start(None, 2), "the person closed it");
    }

    #[test]
    fn each_game_has_its_own_log() {
        assert!(log_path(Some("city-skylines")).ends_with("zohara/windows/city-skylines.log"));
        assert!(log_path(None).ends_with("zohara/windows/shared.log"));
        assert!(log_path(Some("../x y")).ends_with("zohara/windows/___x_y.log"));
    }

    #[test]
    fn the_right_click_choices_are_read_from_the_command_line() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(once_from_args(&a(&["x", "--run-windows", "f.exe", "--layer", "wine"])), Once { runner: Some(Runner::Wine), dedicated: false, sandbox: None });
        assert_eq!(once_from_args(&a(&["--dedicated"])), Once { runner: None, dedicated: true, sandbox: None });
        assert_eq!(once_from_args(&a(&["--layer", "bogus"])).runner, None);
    }

    #[test]
    fn a_program_in_the_shared_windows_folder_keeps_using_it() {
        let d = Path::new("/d");
        let (p, w) = prefixes_in(d, "/d/zohara/windows/pfx/drive_c/Program Files/Game/g.exe", "game");
        assert_eq!((p, w), (PathBuf::from("/d/zohara/windows"), PathBuf::from("/d/zohara/windows/pfx")));
        let (p, w) = prefixes_in(d, "/d/zohara/windows-wine/drive_c/Game/g.exe", "game");
        assert_eq!((p, w), (PathBuf::from("/d/zohara/prefixes/game"), PathBuf::from("/d/zohara/windows-wine")));
        let (p, w) = prefixes_in(d, "/home/me/Games/g.exe", "game");
        assert_eq!((p, w), (PathBuf::from("/d/zohara/prefixes/game"), PathBuf::new()));
    }
}
