//! Programs a Windows installer put into the shared Windows folders (Proton's and plain Wine's), so they can be offered
//! as games: the installer itself makes no menu entry on Linux.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// The program's folder name, for example "Hades".
    pub name: String,
    pub exe: PathBuf,
}

/// Folders every Windows folder has from the start: not something a person installed.
const SKIP_FOLDERS: [&str; 12] = [
    "common files", "internet explorer", "windows media player", "windows nt", "windows mail", "windows photo viewer",
    "windows defender", "windows sidebar", "modifiableloader", "microsoft.net", "reference assemblies", "msbuild",
];

/// Programs that belong to an installed program but are not the program itself.
fn is_helper(file: &str) -> bool {
    let f = file.to_lowercase();
    ["unins", "uninst", "vc_redist", "vcredist", "dxsetup", "dotnet", "crash", "updater", "update", "setup", "helper", "webview", "cef", "installer", "redist", "report"]
        .iter()
        .any(|bad| f.contains(bad))
}

/// The biggest program file in `dir` (looking two folders deep) that is not a helper.
fn main_program(dir: &Path, depth: u32) -> Option<(u64, PathBuf)> {
    let mut best: Option<(u64, PathBuf)> = None;
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        let Ok(meta) = e.metadata() else { continue };
        if meta.is_dir() && depth > 0 {
            if let Some(c) = main_program(&p, depth - 1) {
                if best.as_ref().map(|b| c.0 > b.0).unwrap_or(true) {
                    best = Some(c);
                }
            }
        } else if meta.is_file() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.to_lowercase().ends_with(".exe") && !is_helper(&name) && best.as_ref().map(|b| meta.len() > b.0).unwrap_or(true) {
                best = Some((meta.len(), p));
            }
        }
    }
    best
}

fn plain(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_lowercase()
}

/// A program named like its folder ("Hades/Hades.exe") is the program itself, whatever the sizes.
fn named_like(dir: &Path, folder: &str, depth: u32) -> Option<PathBuf> {
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        if p.is_dir() && depth > 0 {
            if let Some(f) = named_like(&p, folder, depth - 1) {
                return Some(f);
            }
        } else if p.extension().map(|x| x.eq_ignore_ascii_case("exe")).unwrap_or(false) && p.file_stem().map(|s| plain(&s.to_string_lossy()) == plain(folder)).unwrap_or(false) {
            return Some(p);
        }
    }
    None
}

/// What was installed under one C: drive (`.../drive_c`).
pub fn in_drive(drive_c: &Path) -> Vec<Found> {
    let mut out = Vec::new();
    for pf in ["Program Files", "Program Files (x86)"] {
        let Ok(rd) = std::fs::read_dir(drive_c.join(pf)) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !e.path().is_dir() || SKIP_FOLDERS.contains(&name.to_lowercase().as_str()) {
                continue;
            }
            let pick = named_like(&e.path(), &name, 2).or_else(|| main_program(&e.path(), 2).map(|(_, p)| p));
            if let Some(exe) = pick {
                out.push(Found { name, exe });
            }
        }
    }
    out
}

/// Everything installed in the shared Windows folders, without what is already in `known` (program files).
pub fn find(data_home: &Path, known: &[String]) -> Vec<Found> {
    let mut all = in_drive(&data_home.join("zohara/windows/pfx/drive_c"));
    all.extend(in_drive(&data_home.join("zohara/windows-wine/drive_c")));
    all.retain(|f| !known.iter().any(|k| Path::new(k) == f.exe));
    all.sort_by_key(|f| f.name.to_lowercase());
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(p: &Path, size: usize) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, vec![0u8; size]).unwrap();
    }

    #[test]
    fn the_game_is_found_not_its_helpers_or_windows_own_folders() {
        let base = std::env::temp_dir().join(format!("zs-found-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let c = base.join("zohara/windows/pfx/drive_c");
        file(&c.join("Program Files (x86)/Hades/Hades.exe"), 5000);
        file(&c.join("Program Files (x86)/Hades/unins000.exe"), 9000);
        file(&c.join("Program Files (x86)/Hades/bin/crashpad_handler.exe"), 9000);
        file(&c.join("Program Files (x86)/Hades/bin/Engine.exe"), 7000);
        file(&c.join("Program Files/Internet Explorer/iexplore.exe"), 9000);
        file(&c.join("Program Files/Common Files/x.exe"), 9000);
        file(&c.join("Program Files/Notes/notes.exe"), 100);
        let wine = base.join("zohara/windows-wine/drive_c");
        file(&wine.join("Program Files/Other/other.exe"), 100);

        let found = find(&base, &[]);
        let names: Vec<_> = found.iter().map(|f| (f.name.as_str(), f.exe.file_name().unwrap().to_str().unwrap())).collect();
        assert_eq!(names, vec![("Hades", "Hades.exe"), ("Notes", "notes.exe"), ("Other", "other.exe")]);

        // something already in My games is not offered again
        let known = vec![c.join("Program Files/Notes/notes.exe").to_string_lossy().to_string()];
        assert_eq!(find(&base, &known).len(), 2);
        let _ = std::fs::remove_dir_all(&base);
    }
}
