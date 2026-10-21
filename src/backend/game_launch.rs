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
pub const KEY_PROTON_TAG: &str = "X-Zohara-ProtonTag";
/// An installed app marked as a game: the app's own `Exec=` line, kept so the settings can be wrapped around it and taken off again.
const KEY_APP_EXEC: &str = "X-Zohara-AppExec";
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
    /// Use this Proton-GE version (when it is installed) instead of the newest.
    pub fn use_proton_tag(&mut self, tag: &str) {
        if let Some(d) = super::proton::tool_dirs().into_iter().map(|d| d.join(tag)).find(|p| p.join("proton").exists()) {
            self.proton_dir = Some(d);
        }
    }

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
/// The variable WebView2 reads extra browser options from.
const WEBVIEW2_ARGS: &str = "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS";

pub fn argv(path: &str, s: &GameSettings, ctx: &Context) -> Result<Vec<String>, String> {
    let options = split_words(&s.options).ok_or("A quote in the game options is not closed.")?;
    let extra_env = parse_env(&s.env)?;
    let mut env: Vec<(String, String)> = Vec::new();
    if s.dedicated_gpu {
        env.extend(ctx.gpu_env.iter().cloned());
    }
    env.extend(extra_env);
    // WebView2 (the web part many Windows programs embed) cannot use Chromium's own sandbox under Wine; programs that do not use WebView2
    // ignore this. The person's own value for it wins.
    if (s.runner == Runner::Proton || uses_wine(path, s.runner)) && !env.iter().any(|(k, _)| k == WEBVIEW2_ARGS) {
        env.push((WEBVIEW2_ARGS.into(), "--no-sandbox".into()));
    }

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

/// The Proton-GE version this game was pinned to (None = the newest installed).
pub fn proton_tag_of(text: &str) -> Option<String> {
    value_of(text, KEY_PROTON_TAG).filter(|t| super::proton::valid_tag(t))
}

/// The entry with its Proton-GE line set (None = removed).
pub fn with_proton_tag(text: &str, tag: Option<&str>) -> String {
    let mut lines: Vec<String> = text.lines().filter(|l| !l.starts_with(&format!("{KEY_PROTON_TAG}="))).map(|l| l.to_string()).collect();
    if let Some(t) = tag {
        lines.push(format!("{KEY_PROTON_TAG}={t}"));
    }
    lines.join("\n") + "\n"
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

// ── Installed apps marked as games ─────────────────────────────────────────

/// Sets (`Some`), changes or removes (`None`) one key of the `[Desktop Entry]` section; other sections (the app's actions) are
/// never touched. A new key goes at the end of that section.
fn set_main_key(text: &str, key: &str, value: Option<&str>) -> String {
    let mut out: Vec<String> = Vec::new();
    let (mut in_main, mut seen_main, mut done) = (false, false, false);
    let prefix = format!("{key}=");
    for l in text.lines() {
        let t = l.trim();
        if t.starts_with('[') {
            if in_main && !done {
                if let Some(v) = value {
                    push_before_blank_lines(&mut out, format!("{key}={v}"));
                }
                done = true;
            }
            in_main = t == "[Desktop Entry]";
            seen_main |= in_main;
        } else if in_main && t.starts_with(&prefix) {
            if !done {
                if let Some(v) = value {
                    out.push(format!("{key}={v}"));
                }
                done = true;
            }
            continue;
        }
        out.push(l.to_string());
    }
    if seen_main && in_main && !done {
        if let Some(v) = value {
            push_before_blank_lines(&mut out, format!("{key}={v}"));
        }
    }
    out.join("\n") + "\n"
}

/// Adds a line at the end of a section but before the blank lines that separate it from the next one.
fn push_before_blank_lines(out: &mut Vec<String>, line: String) {
    let mut blanks = 0;
    while out.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
        out.pop();
        blanks += 1;
    }
    out.push(line);
    out.extend(std::iter::repeat(String::new()).take(blanks));
}

/// Removes the translated names (`Name[de]=...`) of the main section: a new name must show in every language.
fn drop_translated_names(text: &str) -> String {
    let mut in_main = false;
    let kept: Vec<&str> = text
        .lines()
        .filter(|l| {
            let t = l.trim();
            if t.starts_with('[') {
                in_main = t == "[Desktop Entry]";
            }
            !(in_main && t.starts_with("Name["))
        })
        .collect();
    kept.join("\n") + "\n"
}

/// Whether this is an installed app marked as a game (its own launcher, not a program file we wrote).
pub fn is_app_entry(text: &str) -> bool {
    value_of(text, KEY_FILE).is_none() && value_of(text, "Exec").is_some()
}

/// The settings an installed app was given (all off and empty when none were set yet).
pub fn app_settings_of(text: &str) -> GameSettings {
    let flag = |k: &str| value_of(text, k).as_deref() == Some("true");
    GameSettings {
        gamemode: flag(KEY_GAMEMODE),
        overlay: flag(KEY_OVERLAY),
        dedicated_gpu: flag(KEY_DEDICATED),
        options: unesc(&value_of(text, KEY_OPTIONS).unwrap_or_default()),
        env: unesc(&value_of(text, KEY_ENV).unwrap_or_default()),
        ..GameSettings::default()
    }
}

/// The app's own `Exec=` line: the one saved when settings were first set, else the line now.
fn app_exec_of(text: &str) -> Option<String> {
    value_of(text, KEY_APP_EXEC).map(|v| unesc(&v)).or_else(|| value_of(text, "Exec"))
}

/// Trailing field codes of an `Exec=` line (`%U`, `%f` ...): options go before them.
fn split_field_codes(exec: &str) -> (&str, &str) {
    let mut end = exec.trim_end().len();
    let mut start = end;
    loop {
        let head = exec[..end].trim_end();
        let Some(i) = head.rfind(char::is_whitespace) else { break };
        let tok = &head[i + 1..];
        if tok.len() == 2 && tok.starts_with('%') && "uUfFik".contains(&tok[1..]) {
            start = i;
            end = i;
        } else {
            break;
        }
    }
    (exec[..start.min(exec.len())].trim_end(), exec[start.min(exec.len())..].trim())
}

/// The entry with the settings wrapped around the app's own launcher: `env NAME=value … [gamemoderun] [mangohud] <the app's Exec> <options>`.
/// The app's original `Exec=` is kept in the entry, so changing the settings again (or turning them all off) starts from it.
/// `gpu_env`: the variables that put the app on the stronger chip. `name` changes the name shown in the Start menu.
pub fn with_app_settings(text: &str, name: &str, s: &GameSettings, gpu_env: &[(String, String)]) -> Result<String, String> {
    let original = app_exec_of(text).ok_or("This entry has no program to start.")?;
    let options = split_words(&s.options).ok_or("A quote in the game options is not closed.")?;
    let mut env: Vec<(String, String)> = Vec::new();
    if s.dedicated_gpu {
        env.extend(gpu_env.iter().cloned());
    }
    env.extend(parse_env(&s.env)?);

    let mut prefix: Vec<String> = Vec::new();
    if !env.is_empty() {
        prefix.push("env".into());
        prefix.extend(env.into_iter().map(|(k, v)| format!("{k}={v}")));
    }
    if s.gamemode {
        prefix.push("gamemoderun".into());
    }
    if s.overlay {
        prefix.push("mangohud".into());
    }
    let (body, codes) = split_field_codes(&original);
    let mut exec = String::new();
    for w in &prefix {
        exec.push_str(&exec_quote(w));
        exec.push(' ');
    }
    exec.push_str(body);
    for w in &options {
        exec.push(' ');
        exec.push_str(&exec_quote(w));
    }
    if !codes.is_empty() {
        exec.push(' ');
        exec.push_str(codes);
    }

    let untouched = prefix.is_empty() && options.is_empty();
    let mut t = set_main_key(text, "Exec", Some(&exec));
    let kept = esc(&original);
    t = set_main_key(&t, KEY_APP_EXEC, if untouched { None } else { Some(kept.as_str()) });
    let name = name.replace('\n', " ");
    if !name.trim().is_empty() {
        if value_of(&t, "Name").as_deref() != Some(name.trim()) {
            t = drop_translated_names(&t);
        }
        t = set_main_key(&t, "Name", Some(name.trim()));
    }
    t = set_main_key(&t, KEY_GAMEMODE, s.gamemode.then_some("true"));
    t = set_main_key(&t, KEY_OVERLAY, s.overlay.then_some("true"));
    t = set_main_key(&t, KEY_DEDICATED, s.dedicated_gpu.then_some("true"));
    t = set_main_key(&t, KEY_OPTIONS, (!s.options.trim().is_empty()).then(|| esc(s.options.trim())).as_deref());
    t = set_main_key(&t, KEY_ENV, (!s.env.trim().is_empty()).then(|| esc(s.env.trim())).as_deref());
    Ok(t)
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

    /// The words without the WebView2 variable every Windows launch carries (it has its own test below).
    fn plain(words: Vec<String>) -> Vec<String> {
        let mut w: Vec<String> = words.into_iter().filter(|x| !x.starts_with("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=")).collect();
        if w.first().map(|x| x == "env").unwrap_or(false) && !w.get(1).map(|x| x.contains('=')).unwrap_or(false) {
            w.remove(0);
        }
        w
    }

    fn cmd(path: &str, s: &GameSettings) -> String {
        plain(argv(path, s, &ctx()).unwrap()).join(" ")
    }

    #[test]
    fn webview2_gets_no_sandbox_unless_the_person_sets_it_and_native_programs_are_left_alone() {
        let s = GameSettings::default();
        assert!(argv("/g/Game.exe", &s, &ctx()).unwrap().contains(&"WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--no-sandbox".to_string()));
        let proton = GameSettings { runner: Runner::Proton, ..GameSettings::default() };
        assert!(argv("/g/Game.exe", &proton, &ctx()).unwrap().contains(&"WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--no-sandbox".to_string()));
        let own = GameSettings { env: "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--disable-gpu".into(), ..GameSettings::default() };
        let w = argv("/g/Game.exe", &own, &ctx()).unwrap();
        assert_eq!(w.iter().filter(|x| x.starts_with("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=")).count(), 1);
        assert!(w.contains(&"WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--disable-gpu".to_string()));
        // a Linux program started directly is not a Windows launch
        let direct = GameSettings { runner: Runner::Direct, ..GameSettings::default() };
        assert_eq!(argv("/g/native", &direct, &ctx()).unwrap(), vec!["/g/native"]);
    }

    #[test]
    fn exe_files_run_through_wine_and_game_mode_goes_first() {
        let mut s = GameSettings::default();
        assert_eq!(cmd("/g/Game.EXE", &s), "wine /g/Game.EXE");
        s.gamemode = true;
        assert_eq!(cmd("/g/run", &s), "gamemoderun /g/run");
        assert_eq!(command("/g/My Game.exe", &GameSettings::default(), &ctx()).unwrap(), "\"env\" \"WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--no-sandbox\" \"wine\" \"/g/My Game.exe\"");
    }

    #[test]
    fn bat_files_and_other_windows_files_go_through_wine_start() {
        let s = GameSettings::default();
        assert_eq!(cmd("/g/Run Me!.bat", &s), "wine start /unix /g/Run Me!.bat");
        assert_eq!(cmd("/g/setup.MSI", &s), "wine start /unix /g/setup.MSI");
        let with_prefix = Context { wine_prefix: PathBuf::from("/h/win"), ..ctx() };
        assert_eq!(plain(argv("/g/a.exe", &s, &with_prefix).unwrap()), vec!["env", "WINEPREFIX=/h/win", "wine", "/g/a.exe"]);
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
        let a = plain(argv("/g/Game.exe", &s, &ctx()).unwrap());
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
            plain(argv("/g/Game.exe", &s, &ctx()).unwrap()),
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

    #[test]
    fn a_pinned_proton_version_is_kept_in_the_entry() {
        let t = entry_text("G", "/g/a.exe", &GameSettings::default(), &Context::default()).unwrap();
        assert_eq!(proton_tag_of(&t), None);
        let pinned = with_proton_tag(&t, Some("GE-Proton9-5"));
        assert_eq!(proton_tag_of(&pinned).as_deref(), Some("GE-Proton9-5"));
        assert_eq!(proton_tag_of(&with_proton_tag(&pinned, Some("GE-Proton10-1"))).as_deref(), Some("GE-Proton10-1"));
        assert_eq!(proton_tag_of(&with_proton_tag(&pinned, None)), None);
        assert_eq!(proton_tag_of("[Desktop Entry]\nX-Zohara-ProtonTag=../../x\n"), None, "only a real version name");
        assert!(parse_entry(&pinned).is_some());
    }

    const APP: &str = "[Desktop Entry]\nName=Modrinth App\nExec=ModrinthApp %U\nIcon=modrinth\nCategories=Game;\nX-Zohara-Game=true\n\n[Desktop Action new]\nName=New\nExec=ModrinthApp --new\n";
    const GPU: &[(&str, &str)] = &[("__NV_PRIME_RENDER_OFFLOAD", "1"), ("__GLX_VENDOR_LIBRARY_NAME", "nvidia")];
    fn gpu() -> Vec<(String, String)> {
        GPU.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    #[test]
    fn an_installed_app_is_told_apart_from_a_program_file_game() {
        assert!(is_app_entry(APP));
        let own = entry_text("G", "/g/run.sh", &GameSettings::default(), &ctx()).unwrap();
        assert!(!is_app_entry(&own));
        assert_eq!(app_settings_of(APP), GameSettings::default());
    }

    #[test]
    fn settings_wrap_the_apps_own_launcher_and_keep_its_field_codes_last() {
        let s = GameSettings { gamemode: true, overlay: true, dedicated_gpu: true, options: "--windowed".into(), env: "FOO=bar".into(), ..GameSettings::default() };
        let t = with_app_settings(APP, "Modrinth", &s, &gpu()).unwrap();
        let exec = value_of(&t, "Exec").unwrap();
        assert_eq!(exec, "\"env\" \"__NV_PRIME_RENDER_OFFLOAD=1\" \"__GLX_VENDOR_LIBRARY_NAME=nvidia\" \"FOO=bar\" \"gamemoderun\" \"mangohud\" ModrinthApp \"--windowed\" %U");
        assert_eq!(value_of(&t, "Name").as_deref(), Some("Modrinth"));
        // the app's actions are untouched and the original launcher is kept
        assert!(t.contains("[Desktop Action new]\nName=New\nExec=ModrinthApp --new"));
        assert_eq!(value_of(&t, KEY_APP_EXEC).as_deref(), Some("ModrinthApp %U"));
        // read back
        assert_eq!(app_settings_of(&t), GameSettings { runner: Runner::Auto, dxvk: true, ..s });
        assert!(is_app_entry(&t));
    }

    #[test]
    fn changing_again_starts_from_the_original_and_turning_everything_off_restores_it() {
        let on = GameSettings { gamemode: true, options: "-a".into(), ..GameSettings::default() };
        let once = with_app_settings(APP, "Modrinth App", &on, &gpu()).unwrap();
        let twice = with_app_settings(&once, "Modrinth App", &GameSettings { overlay: true, ..GameSettings::default() }, &gpu()).unwrap();
        assert_eq!(value_of(&twice, "Exec").unwrap(), "\"mangohud\" ModrinthApp %U", "no gamemode or options left over");
        let off = with_app_settings(&twice, "Modrinth App", &GameSettings::default(), &gpu()).unwrap();
        assert_eq!(value_of(&off, "Exec").as_deref(), Some("ModrinthApp %U"));
        assert!(value_of(&off, KEY_APP_EXEC).is_none() && value_of(&off, KEY_OPTIONS).is_none() && value_of(&off, KEY_GAMEMODE).is_none());
        assert_eq!(off.trim_end(), APP.trim_end(), "back to exactly what it was");
    }

    #[test]
    fn bad_typing_is_refused_and_nothing_is_written() {
        let quote = GameSettings { options: "\"open".into(), ..GameSettings::default() };
        assert!(with_app_settings(APP, "x", &quote, &gpu()).unwrap_err().contains("quote"));
        let env = GameSettings { env: "NOEQUALS".into(), ..GameSettings::default() };
        assert!(with_app_settings(APP, "x", &env, &gpu()).unwrap_err().contains("NAME=value"));
        assert!(with_app_settings("[Desktop Entry]\nName=x\n", "x", &GameSettings::default(), &gpu()).is_err());
    }

    #[test]
    fn field_codes_and_a_path_with_spaces_survive() {
        assert_eq!(split_field_codes("app --flag %U"), ("app --flag", "%U"));
        assert_eq!(split_field_codes("app %f %U"), ("app", "%f %U"));
        assert_eq!(split_field_codes("app \"/a b/c\""), ("app \"/a b/c\"", ""));
        assert_eq!(split_field_codes("app"), ("app", ""));
        let e = "[Desktop Entry]\nName=A\nExec=\"/opt/My App/run\" --x %U\nX-Zohara-Game=true\n";
        let t = with_app_settings(e, "A", &GameSettings { gamemode: true, ..GameSettings::default() }, &gpu()).unwrap();
        assert_eq!(value_of(&t, "Exec").unwrap(), "\"gamemoderun\" \"/opt/My App/run\" --x %U");
    }

    #[test]
    fn set_main_key_only_touches_the_main_section() {
        let t = "[Desktop Entry]\nName=A\nExec=a\n\n[Desktop Action x]\nExec=a --x\n";
        let r = set_main_key(t, "Exec", Some("b"));
        assert!(r.contains("Exec=b\n") && r.contains("[Desktop Action x]\nExec=a --x"));
        let r = set_main_key(t, "New", Some("1"));
        assert!(r.contains("Exec=a\nNew=1\n\n[Desktop Action x]"), "{r}");
        assert!(!set_main_key(r.as_str(), "New", None).contains("New="));
    }

    #[test]
    fn a_new_name_replaces_the_translated_names_too_but_an_unchanged_name_keeps_them() {
        let e = "[Desktop Entry]\nName=Foo\nName[de]=Fuu\nExec=foo\nX-Zohara-Game=true\n\n[Desktop Action a]\nName=A\nName[de]=Ah\nExec=foo -a\n";
        let renamed = with_app_settings(e, "Bar", &GameSettings::default(), &gpu()).unwrap();
        assert_eq!(value_of(&renamed, "Name").as_deref(), Some("Bar"));
        assert!(!renamed.contains("Name[de]=Fuu"));
        assert!(renamed.contains("Name[de]=Ah"), "the action keeps its own translated name");
        let same = with_app_settings(e, "Foo", &GameSettings { gamemode: true, ..GameSettings::default() }, &gpu()).unwrap();
        assert!(same.contains("Name[de]=Fuu"));
    }
}
