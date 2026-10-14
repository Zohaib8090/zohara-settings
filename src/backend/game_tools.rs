//! Tools for one Windows game (its gear window): add Windows components with winetricks, reset its Windows folder, and
//! pick which Proton-GE it uses.

use super::game_launch::Runner;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Components games commonly ask for: (winetricks name, what the person sees).
pub const COMPONENTS: [(&str, &str); 6] = [
    ("vcrun2022", "Visual C++ runtime (most newer games)"),
    ("vcrun2019", "Visual C++ 2015-2019 runtime"),
    ("d3dx9", "DirectX 9 files (older games)"),
    ("corefonts", "Windows fonts (text shows as boxes without them)"),
    ("dotnet48", ".NET Framework 4.8 (slow; some launchers and mods)"),
    ("xact", "DirectX sound files (older games with no sound)"),
];

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
    if !valid_component(component) {
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

/// Whether `prefix_dir` belongs to this game alone (so resetting it hurts nobody else).
pub fn is_private_prefix(prefix_dir: &Path, data_home: &Path) -> bool {
    prefix_dir.starts_with(data_home.join("zohara/prefixes")) && prefix_dir != data_home.join("zohara/prefixes")
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
        assert!(reset_prefix(Path::new("/d/zohara/windows"), d).is_err());
        assert_eq!(wine_prefix_of(Path::new("/p"), Path::new("/w"), Runner::Proton), PathBuf::from("/p/pfx"));
        assert_eq!(wine_prefix_of(Path::new("/p"), Path::new("/w"), Runner::Wine), PathBuf::from("/w"));
    }
}
