//! Tools for one Windows game (its gear window): add Windows components with winetricks, reset its Windows folder, and
//! pick which Proton-GE it uses.

use super::game_launch::Runner;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Components games and programs commonly ask for: (name, what the person sees). Most are winetricks verbs; `webview2` is
/// Microsoft's own installer (winetricks has no verb for it), see `install_webview2`.
pub const COMPONENTS: [(&str, &str); 9] = [
    ("webview2", "Web view (WebView2)"),
    ("vcrun2022", "Visual C++ runtime (newer games)"),
    ("vcrun2019", "Visual C++ 2015-2019 runtime"),
    ("d3dx9", "DirectX 9 files (older games)"),
    ("d3dcompiler_47", "DirectX shader compiler"),
    ("corefonts", "Windows fonts"),
    ("dotnet48", ".NET Framework 4.8 (slow)"),
    ("dotnet8", ".NET 8 runtime (slow)"),
    ("xact", "DirectX sound files (older games)"),
];

/// Microsoft's "Evergreen" WebView2 installer: a small program that downloads the runtime itself.
pub const WEBVIEW2_URL: &str = "https://go.microsoft.com/fwlink/p/?LinkId=2124703";
/// The key every program looks at to find the WebView2 runtime.
const WEBVIEW2_KEY: &str = "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";

pub fn valid_component(id: &str) -> bool {
    COMPONENTS.iter().any(|(c, _)| *c == id)
}

/// The Wine folder inside a game's Windows folder: Proton keeps it in `pfx`, plain Wine uses the folder itself.
pub fn wine_prefix_of(prefix_dir: &Path, wine_prefix: &Path, runner: Runner) -> PathBuf {
    if runner == Runner::Proton { prefix_dir.join("pfx") } else { wine_prefix.to_path_buf() }
}

/// The command (words and variables) that installs `component` into the game's Windows folder, with the same Wine that
/// runs the game: Proton-GE's own for Proton, the system's for Wine. Mixing the two would damage the folder.
pub fn winetricks_command(component: &str, runner: Runner, proton_dir: Option<&Path>, wine_prefix: &Path) -> Result<(Vec<String>, Vec<(String, String)>), String> {
    if !valid_component(component) || component == "webview2" {
        return Err("That component is not on the list.".into());
    }
    let mut env = vec![("WINEPREFIX".to_string(), wine_prefix.to_string_lossy().to_string())];
    if runner == Runner::Proton {
        let dir = proton_dir.ok_or("Proton-GE is not installed. Install it in Gaming > Windows games first.")?;
        let bin = dir.join("files/bin");
        if !bin.join("wine").exists() {
            return Err("This Proton-GE has no Wine inside; install it again.".into());
        }
        env.push(("WINE".into(), bin.join("wine").to_string_lossy().to_string()));
        env.push(("WINESERVER".into(), bin.join("wineserver").to_string_lossy().to_string()));
        env.push(("WINEDLLPATH".into(), dir.join("files/lib64/wine").to_string_lossy().to_string()));
    }
    Ok((vec!["winetricks".into(), "-q".into(), component.to_string()], env))
}

/// Runs `winetricks` and returns its last lines on failure. Takes a while (it downloads the component).
pub fn install_component(component: &str, runner: Runner, proton_dir: Option<&Path>, wine_prefix: &Path, log: &Path) -> Result<(), String> {
    if component == "webview2" {
        return install_webview2(runner, proton_dir, wine_prefix, log);
    }
    let (argv, env) = winetricks_command(component, runner, proton_dir, wine_prefix)?;
    std::fs::create_dir_all(wine_prefix).map_err(|e| e.to_string())?;
    if let Some(d) = log.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let out = std::fs::File::create(log).map_err(|e| e.to_string())?;
    let err = out.try_clone().map_err(|e| e.to_string())?;
    let st = Command::new(&argv[0])
        .args(&argv[1..])
        .envs(env)
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
        .status()
        .map_err(|e| format!("winetricks is not installed ({e}). Use Set up in Gaming > Windows games."))?;
    if st.success() {
        Ok(())
    } else {
        let tail: Vec<String> = std::fs::read_to_string(log).unwrap_or_default().lines().rev().take(4).map(|l| l.to_string()).collect();
        Err(format!("winetricks did not finish: {}", tail.into_iter().rev().collect::<Vec<_>>().join(" / ")))
    }
}

/// Whether the registry text of a Windows folder (`system.reg`) lists the WebView2 runtime, with a version.
pub fn webview2_in_registry(system_reg: &str) -> bool {
    let mut in_key = false;
    for line in system_reg.lines() {
        if line.starts_with('[') {
            in_key = line.contains("EdgeUpdate\\\\Clients\\\\") && line.contains(WEBVIEW2_KEY);
        } else if in_key && line.trim_start().starts_with("\"pv\"=\"") {
            return true;
        }
    }
    false
}

/// Whether a component is already in this Windows folder (`wine_prefix` is the Wine folder: Proton's `pfx`, or Wine's own).
pub fn component_installed(component: &str, wine_prefix: &Path) -> bool {
    if component == "webview2" {
        return std::fs::read_to_string(wine_prefix.join("system.reg")).map(|t| webview2_in_registry(&t)).unwrap_or(false);
    }
    // winetricks writes every verb it installed into this file
    std::fs::read_to_string(wine_prefix.join("winetricks.log")).map(|t| t.lines().any(|l| l.trim() == component)).unwrap_or(false)
}

/// Where the WebView2 installer is kept after it was downloaded.
pub fn webview2_installer_path() -> PathBuf {
    let cache = std::env::var("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".cache"));
    cache.join("zohara/windows/downloads/MicrosoftEdgeWebview2Setup.exe")
}

/// A downloaded installer must at least be a Windows program of a sensible size (an error page is not).
pub fn looks_like_installer(bytes_len: u64, head: &[u8]) -> bool {
    bytes_len > 500_000 && head.starts_with(b"MZ")
}

fn download_webview2(dest: &Path) -> Result<(), String> {
    if let Ok(m) = std::fs::metadata(dest) {
        let head = std::fs::read(dest).map(|b| b.into_iter().take(2).collect::<Vec<u8>>()).unwrap_or_default();
        if looks_like_installer(m.len(), &head) {
            return Ok(());
        }
    }
    if let Some(d) = dest.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let part = dest.with_extension("part");
    let st = Command::new("curl")
        .args(["-L", "-f", "-sS", "--max-time", "600", "-o"])
        .arg(&part)
        .arg(WEBVIEW2_URL)
        .stdin(Stdio::null())
        .status()
        .map_err(|e| format!("Couldn't download the WebView2 installer ({e})"))?;
    let head = std::fs::read(&part).map(|b| b.into_iter().take(2).collect::<Vec<u8>>()).unwrap_or_default();
    let len = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    if !st.success() || !looks_like_installer(len, &head) {
        let _ = std::fs::remove_file(&part);
        return Err("Couldn't download the WebView2 installer. Check the internet connection and try again.".into());
    }
    std::fs::rename(&part, dest).map_err(|e| e.to_string())
}

/// The command that runs Microsoft's WebView2 installer silently in the game's Windows folder, with the Wine that runs the game.
pub fn webview2_command(runner: Runner, proton_dir: Option<&Path>, wine_prefix: &Path, installer: &Path) -> Result<(Vec<String>, Vec<(String, String)>), String> {
    let inst = installer.to_string_lossy().to_string();
    if runner == Runner::Proton {
        let dir = proton_dir.ok_or("Proton-GE is not installed. Install it in Gaming > Windows games first.")?;
        // Proton wants the folder that holds `pfx`
        let compat = wine_prefix.parent().ok_or("This Windows folder has no parent folder.")?;
        let home = std::env::var("HOME").unwrap_or_default();
        let env = vec![
            ("STEAM_COMPAT_DATA_PATH".to_string(), compat.to_string_lossy().to_string()),
            ("STEAM_COMPAT_CLIENT_INSTALL_PATH".to_string(), format!("{home}/.local/share/Steam")),
        ];
        Ok((vec![dir.join("proton").to_string_lossy().to_string(), "run".into(), inst, "/silent".into(), "/install".into()], env))
    } else {
        let env = vec![("WINEPREFIX".to_string(), wine_prefix.to_string_lossy().to_string())];
        Ok((vec!["wine".into(), inst, "/silent".into(), "/install".into()], env))
    }
}

fn install_webview2(runner: Runner, proton_dir: Option<&Path>, wine_prefix: &Path, log: &Path) -> Result<(), String> {
    let installer = webview2_installer_path();
    download_webview2(&installer)?;
    let (argv, env) = webview2_command(runner, proton_dir, wine_prefix, &installer)?;
    std::fs::create_dir_all(wine_prefix).map_err(|e| e.to_string())?;
    if let Some(d) = log.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let out = std::fs::File::create(log).map_err(|e| e.to_string())?;
    let err = out.try_clone().map_err(|e| e.to_string())?;
    let st = Command::new(&argv[0]).args(&argv[1..]).envs(env).stdin(Stdio::null()).stdout(out).stderr(err).status().map_err(|e| format!("Couldn't start the installer ({e})"))?;
    if st.success() && component_installed("webview2", wine_prefix) {
        Ok(())
    } else {
        Err("WebView2 did not install. Details are in the log of this Windows folder (Open the log).".into())
    }
}

/// Whether `prefix_dir` belongs to this game alone (so resetting it hurts nobody else).
pub fn is_private_prefix(prefix_dir: &Path, data_home: &Path) -> bool {
    // Proton's folders live under prefixes/, plain Wine's under prefixes-wine/; the shared ones (windows, windows-wine) are neither
    ["zohara/prefixes", "zohara/prefixes-wine"].iter().any(|d| prefix_dir.starts_with(data_home.join(d)) && prefix_dir != data_home.join(d))
}

/// Moves the game's Windows folder to the trash (so a mistake can be undone from there). It is made again at the next start.
pub fn reset_prefix(prefix_dir: &Path, data_home: &Path) -> Result<(), String> {
    if !is_private_prefix(prefix_dir, data_home) {
        return Err("This game shares its Windows folder with other programs, so it is not reset here.".into());
    }
    if !prefix_dir.exists() {
        return Ok(());
    }
    let st = Command::new("gio").arg("trash").arg(prefix_dir).status().map_err(|e| format!("Couldn't move it to the trash ({e})"))?;
    if st.success() { Ok(()) } else { Err("Couldn't move the Windows folder to the trash.".into()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proton_games_use_protons_wine_and_wine_games_the_systems() {
        let dir = std::env::temp_dir().join(format!("zs-tools-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("files/bin")).unwrap();
        std::fs::write(dir.join("files/bin/wine"), "").unwrap();
        let (argv, env) = winetricks_command("vcrun2022", Runner::Proton, Some(&dir), Path::new("/p/pfx")).unwrap();
        assert_eq!(argv, ["winetricks", "-q", "vcrun2022"]);
        assert!(env.iter().any(|(k, v)| k == "WINEPREFIX" && v == "/p/pfx"));
        assert!(env.iter().any(|(k, v)| k == "WINE" && v.ends_with("files/bin/wine")));
        let (_, env) = winetricks_command("corefonts", Runner::Wine, None, Path::new("/w")).unwrap();
        assert!(!env.iter().any(|(k, _)| k == "WINE"), "the system's wine");
        assert!(winetricks_command("corefonts", Runner::Proton, None, Path::new("/w")).is_err(), "Proton-GE missing");
        assert!(winetricks_command("rm -rf", Runner::Wine, None, Path::new("/w")).is_err(), "only listed components");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_a_games_own_windows_folder_can_be_reset() {
        let d = Path::new("/d");
        assert!(is_private_prefix(Path::new("/d/zohara/prefixes/hades"), d));
        assert!(!is_private_prefix(Path::new("/d/zohara/windows"), d), "the shared one");
        assert!(!is_private_prefix(Path::new("/d/zohara/prefixes"), d), "the folder of all games");
        assert!(is_private_prefix(Path::new("/d/zohara/prefixes-wine/hades"), d), "a game's own plain-Wine folder");
        assert!(!is_private_prefix(Path::new("/d/zohara/prefixes-wine"), d));
        assert!(!is_private_prefix(Path::new("/d/zohara/windows-wine"), d), "the shared plain-Wine folder");
        assert!(reset_prefix(Path::new("/d/zohara/windows"), d).is_err());
        assert_eq!(wine_prefix_of(Path::new("/p"), Path::new("/w"), Runner::Proton), PathBuf::from("/p/pfx"));
        assert_eq!(wine_prefix_of(Path::new("/p"), Path::new("/w"), Runner::Wine), PathBuf::from("/w"));
    }

    #[test]
    fn webview2_is_found_in_the_registry_by_its_key_and_version() {
        let reg = "[Software\\\\Wow6432Node\\\\Microsoft\\\\EdgeUpdate\\\\Clients\\\\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}] 1792335807\n#time=1dd5\n\"location\"=\"C:\\\\x\"\n\"pv\"=\"155.0.4283.62\"\n";
        assert!(webview2_in_registry(reg));
        let other = "[Software\\\\Wow6432Node\\\\Microsoft\\\\EdgeUpdate\\\\Clients\\\\{SOMETHING-ELSE}] 1\n\"pv\"=\"1.0\"\n";
        assert!(!webview2_in_registry(other));
        let no_version = "[Software\\\\Wow6432Node\\\\Microsoft\\\\EdgeUpdate\\\\Clients\\\\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}] 1\n\"name\"=\"x\"\n";
        assert!(!webview2_in_registry(no_version));
    }

    #[test]
    fn a_download_must_look_like_a_windows_program() {
        assert!(looks_like_installer(1_873_104, b"MZ\x90"));
        assert!(!looks_like_installer(1_873_104, b"<ht"), "an error page");
        assert!(!looks_like_installer(900, b"MZ"), "too small");
    }

    #[test]
    fn webview2_runs_silently_with_the_games_own_wine() {
        let (argv, env) = webview2_command(Runner::Proton, Some(Path::new("/gp")), Path::new("/d/prefixes/g/pfx"), Path::new("/c/setup.exe")).unwrap();
        assert_eq!(&argv[..], ["/gp/proton", "run", "/c/setup.exe", "/silent", "/install"]);
        assert!(env.iter().any(|(k, v)| k == "STEAM_COMPAT_DATA_PATH" && v == "/d/prefixes/g"), "Proton gets the folder that holds pfx");
        let (argv, env) = webview2_command(Runner::Wine, None, Path::new("/w"), Path::new("/c/setup.exe")).unwrap();
        assert_eq!(&argv[..], ["wine", "/c/setup.exe", "/silent", "/install"]);
        assert_eq!(env, vec![("WINEPREFIX".to_string(), "/w".to_string())]);
        assert!(webview2_command(Runner::Proton, None, Path::new("/d/pfx"), Path::new("/c/s.exe")).is_err());
        // winetricks has no such verb: it must not be asked for it
        assert!(winetricks_command("webview2", Runner::Wine, None, Path::new("/w")).is_err());
        assert!(valid_component("webview2"));
    }

    #[test]
    fn an_installed_winetricks_verb_is_read_from_its_log() {
        let d = std::env::temp_dir().join(format!("zs-wt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("winetricks.log"), "corefonts\nvcrun2022\n").unwrap();
        assert!(component_installed("vcrun2022", &d));
        assert!(!component_installed("dotnet48", &d));
        assert!(!component_installed("webview2", &d));
        let _ = std::fs::remove_dir_all(&d);
    }
}

#[cfg(test)]
mod real_prefix {
    use super::*;

    /// `ZS_TEST_PREFIX=/path/to/pfx cargo test real_webview2 -- --ignored`: a real Windows folder with WebView2 installed.
    #[test]
    #[ignore]
    fn real_webview2_is_detected() {
        let p = std::env::var("ZS_TEST_PREFIX").expect("set ZS_TEST_PREFIX");
        assert!(component_installed("webview2", Path::new(&p)));
    }
}
