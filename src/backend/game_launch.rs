//! How a game started from "My games" is launched, and the per-game choices that change it.
//!
//! A game is a desktop entry in `~/.local/share/applications`. Its `Exec=` line is built from the game's settings, and
//! the settings are also kept in the same file as `X-Zohara-*` keys, so the page can read them back, show them, and
//! rebuild the line after a change. Nothing the person types reaches a shell: the extra options are split into words
//! and each word is quoted for the `Exec=` line; environment names are checked.
//!
//! The line is `env NAME=value … [gamemoderun] [mangohud] <runner> <file> <options>`.
//! * Runner **Wine** (Windows app support): `wine file.exe`, or `wine start /unix file` for a `.bat`, installer and so on.
//! * Runner **Proton-GE**: the installed Proton-GE's `proton run`, with its own private Windows folder per game
//!   (`~/.local/share/zohara/prefixes/<game>`). It brings DXVK and VKD3D (DirectX through Vulkan), which is faster than
//!   Wine's own DirectX; `PROTON_USE_WINED3D=1` switches that off for a game that misbehaves with it.
//! * Runner **Direct**: the file itself.

use std::path::{Path, PathBuf};

pub const KEY_FILE: &str = "X-Zohara-File";
const KEY_RUNNER: &str = "X-Zohara-Runner";
const KEY_WINED3D: &str = "X-Zohara-Wined3d";
const KEY_GAMEMODE: &str = "X-Zohara-GameMode";
const KEY_OVERLAY: &str = "X-Zohara-Overlay";
const KEY_DEDICATED: &str = "X-Zohara-Dedicated";
const KEY_OPTIONS: &str = "X-Zohara-Options";
const KEY_ENV: &str = "X-Zohara-Env";
pub const KEY_SANDBOX: &str = "X-Zohara-Sandbox";
pub const MARK: &str = "X-Zohara-Game=true";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Runner {
    /// Decide from the file: a Windows .exe, .bat, .cmd, .msi or .lnk goes to Wine, anything else runs as it is.
    Auto,
    /// Always through Windows app support (Wine), whatever the file is.
    Wine,
    /// Through the installed Proton-GE.
    Proton,
    /// Always run the file itself.
    Direct,
}

impl Runner {
    pub const ALL: [Runner; 4] = [Runner::Auto, Runner::Wine, Runner::Proton, Runner::Direct];
    pub fn label(self) -> &'static str {
        match self {
            Runner::Auto => "Automatic",
            Runner::Wine => "Windows app support (Wine)",
            Runner::Proton => "Proton-GE",
            Runner::Direct => "Run the file itself",
        }
    }
    fn word(self) -> &'static str {
        match self {
            Runner::Auto => "auto",
            Runner::Wine => "wine",
            Runner::Proton => "proton",
            Runner::Direct => "direct",
        }
    }
    fn from_word(w: &str) -> Runner {
        Runner::ALL.into_iter().find(|r| r.word() == w).unwrap_or(Runner::Auto)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameSettings {
    pub runner: Runner,
    /// Proton only: DirectX through Vulkan (DXVK/VKD3D). On by default; off = Wine's own DirectX (OpenGL).
    pub dxvk: bool,
    pub gamemode: bool,
    pub overlay: bool,
    pub dedicated_gpu: bool,
    /// Extra command-line options for the game, typed as one line.
    pub options: String,
    /// Extra environment variables, `NAME=value NAME2=value2`.
    pub env: String,
}

impl Default for GameSettings {
    fn default() -> Self {
        GameSettings { runner: Runner::Auto, dxvk: true, gamemode: false, overlay: false, dedicated_gpu: false, options: String::new(), env: String::new() }
    }
}

/// What the line needs to know about this computer. Built by the page; tests give their own.
#[derive(Clone, Debug, Default)]
pub struct Context {
    /// The Proton-GE folder to use (holds the `proton` script), if one is installed.
    pub proton_dir: Option<PathBuf>,
    pub steam_root: PathBuf,
    pub prefix_dir: PathBuf,
    /// Variables that put the game on the stronger graphics chip (from `switcherooctl list`).
    pub gpu_env: Vec<(String, String)>,
    /// Plain Wine's Windows folder; empty = Wine's own default (`~/.wine`).
    pub wine_prefix: PathBuf,
}

impl Context {
    /// This computer: the newest Proton-GE installed for the repository Steam, the given Windows folder, and the
    /// stronger chip's variables when they are wanted.
    pub fn here(prefix_dir: PathBuf, wine_prefix: PathBuf, dedicated: bool) -> Context {
        let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
        let proton_dir = super::proton::installed()
            .first()
            .and_then(|tag| super::proton::tool_dirs().into_iter().map(|d| d.join(tag)).find(|p| p.join("proton").exists()));
        Context {
            proton_dir,
            steam_root: home.join(".local/share/Steam"),
            prefix_dir,
            gpu_env: if dedicated { super::gpu_pref::dedicated_env() } else { Vec::new() },
            wine_prefix,
        }
    }
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

/// One word for a shell line (Steam's launch options run through a shell): bare when plain, else in single quotes.
pub fn exec_quote_shell(v: &str) -> String {
    if !v.is_empty() && v.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-./:=+,@".contains(&b)) {
        return v.to_string();
    }
    format!("'{}'", v.replace('\'', "'\\''"))
}

fn is_windows_program(path: &str) -> bool {
    let p = path.to_lowercase();
    [".exe", ".bat", ".cmd", ".msi", ".lnk"].iter().any(|e| p.ends_with(e))
}

/// Whether this choice sends the file to Wine (or Proton, which is Wine with extras).
pub fn uses_wine(path: &str, runner: Runner) -> bool {
    match runner {
        Runner::Wine | Runner::Proton => true,
        Runner::Direct => false,
        Runner::Auto => is_windows_program(path),
    }
}

/// Splits a typed line into words the way a shell would for quotes and backslashes. None = a quote was left open.
pub fn split_words(s: &str) -> Option<Vec<String>> {
    let mut words: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut have = false;
    let mut quote: Option<char> = None;
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (None, '\'') | (None, '"') => {
                quote = Some(c);
                have = true;
            }
            (Some(q), c) if c == q => quote = None,
            (Some('"'), '\\') => match chars.next() {
                Some(n) if n == '"' || n == '\\' || n == '$' || n == '`' => cur.push(n),
                Some(n) => {
                    cur.push('\\');
                    cur.push(n);
                }
                None => cur.push('\\'),
            },
            (None, '\\') => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                    have = true;
                }
            }
            (None, c) if c.is_whitespace() => {
                if have {
                    words.push(std::mem::take(&mut cur));
                    have = false;
                }
            }
            (_, c) => {
                cur.push(c);
                have = true;
            }
        }
    }
    if quote.is_some() {
        return None;
    }
    if have {
        words.push(cur);
    }
    Some(words)
}

/// `NAME=value NAME2="a b"` -> pairs. Names are letters, digits and `_`, not starting with a digit.
pub fn parse_env(s: &str) -> Result<Vec<(String, String)>, String> {
    let words = split_words(s).ok_or("A quote in the environment variables is not closed.")?;
    let mut out = Vec::new();
    for w in words {
        let (k, v) = w.split_once('=').ok_or_else(|| format!("\"{w}\" is not NAME=value."))?;
        let ok = !k.is_empty() && !k.starts_with(|c: char| c.is_ascii_digit()) && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
        if !ok {
            return Err(format!("\"{k}\" is not a valid variable name (letters, digits and _)."));
        }
        if v.contains(['\n', '\0']) {
            return Err("A value can't hold a line break.".into());
        }
        out.push((k.to_string(), v.to_string()));
    }
    Ok(out)
}

/// The words that start a game: `env NAME=value … [gamemoderun] [mangohud] <runner> <file> <options>`.
/// Err = what to tell the person (a typed field that doesn't parse, Proton not installed).
pub fn argv(path: &str, s: &GameSettings, ctx: &Context) -> Result<Vec<String>, String> {
    let options = split_words(&s.options).ok_or("A quote in the game options is not closed.")?;
    let extra_env = parse_env(&s.env)?;
    let mut env: Vec<(String, String)> = Vec::new();
    if s.dedicated_gpu {
        env.extend(ctx.gpu_env.iter().cloned());
    }
    env.extend(extra_env);

    let mut parts: Vec<String> = Vec::new();
    let is_exe = path.to_lowercase().ends_with(".exe");
    if s.runner == Runner::Proton {
        let dir = ctx.proton_dir.as_ref().ok_or("Proton-GE is not installed. Install it in Gaming > Windows games first.")?;
        env.push(("STEAM_COMPAT_CLIENT_INSTALL_PATH".into(), ctx.steam_root.to_string_lossy().to_string()));
        env.push(("STEAM_COMPAT_DATA_PATH".into(), ctx.prefix_dir.to_string_lossy().to_string()));
        if !s.dxvk {
            env.push(("PROTON_USE_WINED3D".into(), "1".into()));
        }
        parts.push(dir.join("proton").to_string_lossy().to_string());
        parts.push("run".into());
    } else if uses_wine(path, s.runner) {
        if !ctx.wine_prefix.as_os_str().is_empty() {
            env.push(("WINEPREFIX".into(), ctx.wine_prefix.to_string_lossy().to_string()));
        }
        parts.push("wine".into());
    }
    if uses_wine(path, s.runner) && !is_exe {
        parts.push("start".into());
        parts.push("/unix".into());
    }
    parts.push(path.to_string());
    parts.extend(options);

    let mut line: Vec<String> = Vec::new();
    if !env.is_empty() {
        line.push("env".into());
        line.extend(env.into_iter().map(|(k, v)| format!("{k}={v}")));
    }
    if s.gamemode {
        line.push("gamemoderun".into());
    }
    if s.overlay {
        line.push("mangohud".into());
    }
    line.extend(parts);
    Ok(line)
}

/// The words as an `Exec=` line: every word quoted, so nothing in them is read as a field code or a shell character.
pub fn command(path: &str, s: &GameSettings, ctx: &Context) -> Result<String, String> {
    Ok(argv(path, s, ctx)?.iter().map(|w| exec_quote(w)).collect::<Vec<_>>().join(" "))
}

fn esc(v: &str) -> String {
    v.replace('\\', "\\\\").replace('\n', " ")
}

fn unesc(v: &str) -> String {
    v.replace("\\\\", "\\")
}

/// The whole desktop entry for a game, settings included.
pub fn entry_text(name: &str, path: &str, s: &GameSettings, ctx: &Context) -> Result<String, String> {
    let exec = command(path, s, ctx)?;
    let dir = Path::new(path).parent().map(|d| d.to_string_lossy().to_string()).unwrap_or_default();
    let mut t = String::from("[Desktop Entry]\nType=Application\n");
    t.push_str(&format!("Name={}\n", name.replace('\n', " ")));
    t.push_str(&format!("Exec={exec}\n"));
    if !dir.is_empty() {
        t.push_str(&format!("Path={dir}\n"));
    }
    t.push_str("Icon=applications-games\nCategories=Game;\nTerminal=false\n");
    t.push_str(MARK);
    t.push('\n');
    t.push_str(&format!("{KEY_FILE}={}\n{KEY_RUNNER}={}\n", esc(path), s.runner.word()));
    t.push_str(&format!("{KEY_WINED3D}={}\n", !s.dxvk));
    t.push_str(&format!("{KEY_GAMEMODE}={}\n{KEY_OVERLAY}={}\n{KEY_DEDICATED}={}\n", s.gamemode, s.overlay, s.dedicated_gpu));
    if !s.options.trim().is_empty() {
        t.push_str(&format!("{KEY_OPTIONS}={}\n", esc(s.options.trim())));
    }
    if !s.env.trim().is_empty() {
        t.push_str(&format!("{KEY_ENV}={}\n", esc(s.env.trim())));
    }
    Ok(t)
}

fn value_of(text: &str, key: &str) -> Option<String> {
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

/// Whether a game's entry asks for Safety mode (off unless the person turned it on for this game).
pub fn sandbox_of(text: &str) -> bool {
    value_of(text, KEY_SANDBOX).as_deref() == Some("true")
}

/// The entry with its Safety mode line set (added, or changed).
pub fn with_sandbox(text: &str, on: bool) -> String {
    let line = format!("{KEY_SANDBOX}={on}");
    if value_of(text, KEY_SANDBOX).is_some() {
        text.lines().map(|l| if l.starts_with(&format!("{KEY_SANDBOX}=")) { line.clone() } else { l.to_string() }).collect::<Vec<_>>().join("\n") + "\n"
    } else {
        format!("{}{line}\n", if text.ends_with('\n') { text.to_string() } else { format!("{text}\n") })
    }
}

/// The `Name=` of a desktop entry.
pub fn entry_name(text: &str) -> Option<String> {
    value_of(text, "Name")
}

/// The program file and settings of a game this page wrote. None for an installed app that was only marked as a game.
pub fn parse_entry(text: &str) -> Option<(String, GameSettings)> {
    let file = unesc(&value_of(text, KEY_FILE)?);
    let flag = |k: &str| value_of(text, k).as_deref() == Some("true");
    Some((
        file,
        GameSettings {
            runner: Runner::from_word(&value_of(text, KEY_RUNNER).unwrap_or_default()),
            dxvk: !flag(KEY_WINED3D),
            gamemode: flag(KEY_GAMEMODE),
            overlay: flag(KEY_OVERLAY),
            dedicated_gpu: flag(KEY_DEDICATED),
            options: unesc(&value_of(text, KEY_OPTIONS).unwrap_or_default()),
            env: unesc(&value_of(text, KEY_ENV).unwrap_or_default()),
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Context {
        Context {
            proton_dir: Some(PathBuf::from("/h/.local/share/Steam/compatibilitytools.d/GE-Proton11-9")),
            steam_root: PathBuf::from("/h/.local/share/Steam"),
            prefix_dir: PathBuf::from("/h/.local/share/zohara/prefixes/cool-game"),
            gpu_env: vec![("__NV_PRIME_RENDER_OFFLOAD".into(), "1".into()), ("__GLX_VENDOR_LIBRARY_NAME".into(), "nvidia".into())],
            wine_prefix: PathBuf::new(),
        }
    }

    fn cmd(path: &str, s: &GameSettings) -> String {
        argv(path, s, &ctx()).unwrap().join(" ")
    }

    #[test]
    fn exe_files_run_through_wine_and_game_mode_goes_first() {
        let mut s = GameSettings::default();
        assert_eq!(cmd("/g/Game.EXE", &s), "wine /g/Game.EXE");
        s.gamemode = true;
        assert_eq!(cmd("/g/run", &s), "gamemoderun /g/run");
        assert_eq!(command("/g/My Game.exe", &GameSettings::default(), &ctx()).unwrap(), "\"wine\" \"/g/My Game.exe\"");
    }

    #[test]
    fn bat_files_and_other_windows_files_go_through_wine_start() {
        let s = GameSettings::default();
        assert_eq!(cmd("/g/Run Me!.bat", &s), "wine start /unix /g/Run Me!.bat");
        assert_eq!(cmd("/g/setup.MSI", &s), "wine start /unix /g/setup.MSI");
        let with_prefix = Context { wine_prefix: PathBuf::from("/h/win"), ..ctx() };
        assert_eq!(argv("/g/a.exe", &s, &with_prefix).unwrap(), vec!["env", "WINEPREFIX=/h/win", "wine", "/g/a.exe"]);
    }

    #[test]
    fn the_person_can_send_any_file_to_wine_or_run_it_directly() {
        let mut s = GameSettings { runner: Runner::Wine, ..GameSettings::default() };
        assert_eq!(cmd("/g/game.dat", &s), "wine start /unix /g/game.dat");
        assert_eq!(cmd("/g/Game.exe", &s), "wine /g/Game.exe");
        s.runner = Runner::Direct;
        assert_eq!(cmd("/g/Game.exe", &s), "/g/Game.exe");
        assert!(uses_wine("/x/a.BAT", Runner::Auto) && !uses_wine("/x/a.sh", Runner::Auto));
    }

    #[test]
    fn proton_gets_its_own_folder_and_dxvk_can_be_switched_off() {
        let mut s = GameSettings { runner: Runner::Proton, ..GameSettings::default() };
        let a = argv("/g/Game.exe", &s, &ctx()).unwrap();
        assert_eq!(
            a,
            vec![
                "env",
                "STEAM_COMPAT_CLIENT_INSTALL_PATH=/h/.local/share/Steam",
                "STEAM_COMPAT_DATA_PATH=/h/.local/share/zohara/prefixes/cool-game",
                "/h/.local/share/Steam/compatibilitytools.d/GE-Proton11-9/proton",
                "run",
                "/g/Game.exe"
            ]
        );
        s.dxvk = false;
        assert!(cmd("/g/Game.exe", &s).contains("PROTON_USE_WINED3D=1"));
        assert!(cmd("/g/go.bat", &s).ends_with("proton run start /unix /g/go.bat"));
        // not installed: a plain message, not a broken launcher
        let none = Context { proton_dir: None, ..ctx() };
        assert!(argv("/g/Game.exe", &s, &none).unwrap_err().contains("not installed"));
    }

    #[test]
    fn graphics_chip_overlay_and_options_are_added_in_order() {
        let s = GameSettings { dedicated_gpu: true, gamemode: true, overlay: true, options: "-windowed -w \"1280 720\"".into(), env: "FOO=bar".into(), ..GameSettings::default() };
        assert_eq!(
            argv("/g/Game.exe", &s, &ctx()).unwrap(),
            vec!["env", "__NV_PRIME_RENDER_OFFLOAD=1", "__GLX_VENDOR_LIBRARY_NAME=nvidia", "FOO=bar", "gamemoderun", "mangohud", "wine", "/g/Game.exe", "-windowed", "-w", "1280 720"]
        );
        // the stronger chip is only used when asked for
        let off = GameSettings { dedicated_gpu: false, ..s };
        assert!(!cmd("/g/Game.exe", &off).contains("NV_PRIME"));
    }

    #[test]
    fn typed_text_never_reaches_a_shell() {
        let s = GameSettings { options: "$(rm -rf ~) `x` ;&|".into(), ..GameSettings::default() };
        let c = command("/g/Game.exe", &s, &ctx()).unwrap();
        assert!(c.contains("\"\\$(rm\"") && c.contains("\"\\`x\\`\""), "{c}");
        assert!(command("/g/a", &GameSettings { options: "'open".into(), ..GameSettings::default() }, &ctx()).is_err());
        assert!(parse_env("1BAD=x").is_err());
        assert!(parse_env("A-B=x").is_err());
        assert!(parse_env("novalue").is_err());
        assert_eq!(parse_env("A=1 B=\"two words\"").unwrap(), vec![("A".to_string(), "1".to_string()), ("B".to_string(), "two words".to_string())]);
    }

    #[test]
    fn words_are_split_like_a_shell_would() {
        assert_eq!(split_words("a b  c").unwrap(), vec!["a", "b", "c"]);
        assert_eq!(split_words("-x \"a b\" 'c d' e\\ f").unwrap(), vec!["-x", "a b", "c d", "e f"]);
        assert_eq!(split_words("\"\"").unwrap(), vec![""]);
        assert_eq!(split_words("").unwrap(), Vec::<String>::new());
        assert!(split_words("\"open").is_none());
    }

    #[test]
    fn settings_survive_a_round_trip_through_the_entry() {
        let s = GameSettings {
            runner: Runner::Proton,
            dxvk: false,
            gamemode: true,
            overlay: true,
            dedicated_gpu: true,
            options: "-windowed \"C:\\x y\"".into(),
            env: "A=1".into(),
        };
        let t = entry_text("Cool Game", "/g/My Games/Cool.exe", &s, &ctx()).unwrap();
        assert!(t.contains("Path=/g/My Games\n") && t.contains("Categories=Game;") && t.contains(MARK));
        let (file, back) = parse_entry(&t).unwrap();
        assert_eq!(file, "/g/My Games/Cool.exe");
        assert_eq!(back, s);
        // an entry that is just a marked installed app has no settings to show
        assert!(parse_entry("[Desktop Entry]\nName=Foo\nExec=foo\nX-Zohara-Game=true\n").is_none());
    }

    #[test]
    fn safety_mode_is_kept_in_the_entry_and_is_off_by_default() {
        let t = entry_text("G", "/g/a.exe", &GameSettings::default(), &Context::default()).unwrap();
        assert!(!sandbox_of(&t));
        let on = with_sandbox(&t, true);
        assert!(sandbox_of(&on));
        let off = with_sandbox(&on, false);
        assert!(!sandbox_of(&off));
        assert_eq!(off.matches(KEY_SANDBOX).count(), 1);
        assert!(parse_entry(&on).is_some(), "the entry still reads back");
    }
}
