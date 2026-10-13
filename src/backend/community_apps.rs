//! Apps that come from outside the system packages and Flatpak: packages built from the AUR (pacman calls them "foreign")
//! and AppImages installed through Zohara Store (single files kept in the person's own folder, with a launcher of their own).
//!
//! Settings lists them next to the other apps, marked for what they are, so the person can see where each one came from.
//! The Store installs and updates them; Settings only shows them, applies the same controls and removes them.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// `pacman -Qmq` lines: the packages no repository carries.
pub fn parse_foreign(text: &str) -> HashSet<String> {
    text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect()
}

pub fn foreign_packages() -> HashSet<String> {
    Command::new("pacman")
        .arg("-Qmq")
        .stderr(Stdio::null())
        .output()
        .map(|o| parse_foreign(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
}

/// Built from the AUR: not in any repository, and not one of Zohara's own packages (those are built by Zohara's CI).
pub fn is_aur(pkg: &str, foreign: &HashSet<String>) -> bool {
    foreign.contains(pkg) && !pkg.starts_with("zohara-")
}

pub fn data_home() -> PathBuf {
    std::env::var("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share"))
}

/// An AppImage installed through the Store.
#[derive(Debug, Clone, PartialEq)]
pub struct AppImageApp {
    pub slug: String,
    pub name: String,
    pub comment: String,
    pub icon: String,
    pub launcher: PathBuf,
    pub file: PathBuf,
}

fn safe_slug(s: &str) -> bool {
    !s.is_empty() && s.len() <= 120 && !s.starts_with('.') && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn entry_value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let mut inside = false;
    for l in text.lines() {
        let l = l.trim();
        if l.starts_with('[') {
            inside = l == "[Desktop Entry]";
        } else if inside {
            if let Some(v) = l.strip_prefix(key).and_then(|r| r.strip_prefix('=')) {
                return Some(v.trim());
            }
        }
    }
    None
}

/// The Store's AppImages under `data` (the person's data folder): their launchers say `X-Zohara-AppImage=<id>`.
pub fn appimages_in(data: &Path) -> Vec<AppImageApp> {
    let launchers = data.join("applications");
    let Ok(rd) = std::fs::read_dir(&launchers) else { return Vec::new() };
    let mut out: Vec<AppImageApp> = rd
        .flatten()
        .filter_map(|e| {
            let text = std::fs::read_to_string(e.path()).ok()?;
            let slug = entry_value(&text, "X-Zohara-AppImage")?.to_string();
            if !safe_slug(&slug) {
                return None;
            }
            let file = data.join("zohara-store/appimages").join(format!("{slug}.AppImage"));
            if !file.is_file() {
                return None;
            }
            Some(AppImageApp {
                name: entry_value(&text, "Name").filter(|n| !n.is_empty()).unwrap_or(&slug).to_string(),
                comment: entry_value(&text, "Comment").unwrap_or("").to_string(),
                icon: entry_value(&text, "Icon").unwrap_or("").to_string(),
                launcher: e.path(),
                file,
                slug,
            })
        })
        .collect();
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

pub fn appimages() -> Vec<AppImageApp> {
    appimages_in(&data_home())
}

/// Removes an AppImage's file, its icon, its notes and its launcher.
pub fn remove_appimage_in(data: &Path, slug: &str) -> Result<(), String> {
    if !safe_slug(slug) {
        return Err("That is not an installed AppImage.".into());
    }
    let store = data.join("zohara-store");
    let _ = std::fs::remove_file(store.join("appimages").join(format!("{slug}.AppImage")));
    let _ = std::fs::remove_file(store.join("appimages").join(format!("{slug}.meta")));
    for ext in ["png", "svg"] {
        let _ = std::fs::remove_file(store.join("appimage-icons").join(format!("{slug}.{ext}")));
    }
    let _ = std::fs::remove_file(data.join("applications").join(format!("zohara-appimage-{slug}.desktop")));
    let _ = Command::new("update-desktop-database").arg(data.join("applications")).stderr(Stdio::null()).status();
    Ok(())
}

pub fn remove_appimage(slug: &str) -> Result<(), String> {
    remove_appimage_in(&data_home(), slug)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foreign_packages_are_read_and_zoharas_own_are_not_called_aur() {
        let f = parse_foreign("yay\nzohara-voice\n\n  ventoy \n");
        assert_eq!(f.len(), 3);
        assert!(is_aur("yay", &f) && is_aur("ventoy", &f));
        assert!(!is_aur("zohara-voice", &f), "Zohara's own packages are not from the AUR");
        assert!(!is_aur("firefox", &f));
    }

    #[test]
    fn appimages_are_found_by_their_launchers_and_removed_completely() {
        let root = std::env::temp_dir().join(format!("zohara-ca-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let apps = root.join("zohara-store/appimages");
        std::fs::create_dir_all(&apps).unwrap();
        std::fs::create_dir_all(root.join("zohara-store/appimage-icons")).unwrap();
        std::fs::create_dir_all(root.join("applications")).unwrap();
        std::fs::write(apps.join("cool-app.AppImage"), "x").unwrap();
        std::fs::write(apps.join("cool-app.meta"), "x").unwrap();
        std::fs::write(root.join("zohara-store/appimage-icons/cool-app.png"), "x").unwrap();
        std::fs::write(
            root.join("applications/zohara-appimage-cool-app.desktop"),
            "[Desktop Entry]\nName=Cool App\nComment=Does things\nIcon=/x/cool.png\nX-Zohara-AppImage=cool-app\n",
        )
        .unwrap();
        // A launcher of something else, and one whose file is gone: neither is listed.
        std::fs::write(root.join("applications/other.desktop"), "[Desktop Entry]\nName=Other\n").unwrap();
        std::fs::write(root.join("applications/zohara-appimage-gone.desktop"), "[Desktop Entry]\nName=Gone\nX-Zohara-AppImage=gone\n").unwrap();

        let found = appimages_in(&root);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].slug.as_str(), found[0].name.as_str(), found[0].comment.as_str()), ("cool-app", "Cool App", "Does things"));

        assert!(remove_appimage_in(&root, "../x").is_err());
        remove_appimage_in(&root, "cool-app").unwrap();
        assert!(appimages_in(&root).is_empty());
        assert!(!apps.join("cool-app.AppImage").exists() && !apps.join("cool-app.meta").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
