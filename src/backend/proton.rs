//! Windows games: the 32-bit support Steam needs, Proton-GE, and ProtonUp-Qt (Gaming page).
//!
//! * **Support** (`support_script`, one administrator prompt): turns on Arch's `[multilib]` repository, then installs
//!   Steam, Lutris, winetricks, the 32-bit graphics libraries for this computer's graphics chip, and ProtonUp-Qt. It
//!   refuses while updates are waiting, because syncing a new repository on a computer that is behind would mix old
//!   and new libraries.
//! * **Proton-GE** (`GE-ProtonNN-N`): Valve's Proton plus extra fixes for more Windows games. No repository carries it,
//!   so it is downloaded from the project's own releases, checked against the published SHA-512 and unpacked into
//!   Steam's `compatibilitytools.d` folder in the person's home (no administrator needed). Steam lists it under
//!   "Compatibility" the next time it starts.
//!
//! Scripts are fixed text plus values that were checked first (a release tag is only letters, digits, `-` and `.`).

use std::path::{Path, PathBuf};
use std::process::Command;

pub const MULTILIB_HEADER: &str = "[multilib]";
pub const EXIT_UPDATES_WAITING: i32 = 6;
const RELEASES_API: &str = "https://api.github.com/repos/GloriousEggroll/proton-ge-custom/releases/latest";

/// Whether `/etc/pacman.conf` text has the multilib repository switched on.
pub fn multilib_enabled(conf: &str) -> bool {
    conf.lines().any(|l| l.trim() == MULTILIB_HEADER)
}

pub fn multilib_on() -> bool {
    std::fs::read_to_string("/etc/pacman.conf").map(|c| multilib_enabled(&c)).unwrap_or(false)
}

fn pkg_installed(pkg: &str) -> bool {
    Command::new("pacman").args(["-Qq", pkg]).output().map(|o| o.status.success()).unwrap_or(false)
}

/// Steam from the repositories (not the Flatpak) is installed, which is what makes this computer ready.
pub fn support_ready() -> bool {
    multilib_on() && pkg_installed("steam")
}

/// Script for the one-click setup. Exit codes: 6 = updates are waiting (see `explain_exit`).
pub fn support_script() -> String {
    format!(
        r#"set -e
if ! grep -q '^\[multilib\]' /etc/pacman.conf; then
  if grep -q '^#\[multilib\]' /etc/pacman.conf; then
    sed -i '/^#\[multilib\]/{{s/^#//;n;s/^#//}}' /etc/pacman.conf
  else
    printf '\n[multilib]\nInclude = /etc/pacman.d/mirrorlist\n' >> /etc/pacman.conf
  fi
fi
pacman -Sy --noconfirm
if [ -n "$(pacman -Qu 2>/dev/null)" ]; then exit {EXIT_UPDATES_WAITING}; fi
GPU=$(lspci 2>/dev/null | grep -iE 'vga|3d|display' || true)
LIBS="lib32-mesa"
if echo "$GPU" | grep -qi nvidia; then LIBS="$LIBS lib32-nvidia-utils"; fi
if echo "$GPU" | grep -qiE 'amd|ati|radeon'; then LIBS="$LIBS lib32-vulkan-radeon"; fi
if echo "$GPU" | grep -qi intel; then LIBS="$LIBS lib32-vulkan-intel"; fi
if [ "$LIBS" = "lib32-mesa" ]; then LIBS="$LIBS lib32-vulkan-radeon lib32-vulkan-intel"; fi
pacman -S --needed --noconfirm steam lutris winetricks lib32-gamemode lib32-mangohud $LIBS
pacman -S --needed --noconfirm protonup-qt || true
"#
    )
}

pub fn explain_exit(code: Option<i32>) -> Option<String> {
    match code {
        Some(0) => None,
        Some(EXIT_UPDATES_WAITING) => Some("Updates are waiting. Update the computer first (Zohara Update), then try again.".into()),
        Some(126) | Some(127) => Some("The password prompt was cancelled.".into()),
        Some(n) => Some(format!("Setup did not finish (error {n}). Check the internet connection and try again.")),
        None => Some("Setup was stopped.".into()),
    }
}

pub fn run_support() -> Option<String> {
    match Command::new("pkexec").args(["sh", "-c", &support_script()]).status() {
        Ok(s) => explain_exit(s.code()),
        Err(_) => Some("Couldn't ask for the password.".into()),
    }
}

// ── Proton-GE ─────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub tag: String,
    pub tar_url: String,
    pub sha_url: String,
    pub size: u64,
}

/// A release tag as Proton-GE names them: `GE-Proton11-9`. Anything else is refused before it reaches a script.
pub fn valid_tag(tag: &str) -> bool {
    tag.starts_with("GE-Proton") && tag.len() < 40 && tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
}

/// The latest release from the GitHub API's JSON: the x86_64 archive and its checksum file.
pub fn parse_release(json: &str) -> Option<Release> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let tag = v["tag_name"].as_str()?.to_string();
    if !valid_tag(&tag) {
        return None;
    }
    let mut tar = None;
    let mut sha = None;
    for a in v["assets"].as_array()? {
        let name = a["name"].as_str().unwrap_or("");
        let url = a["browser_download_url"].as_str().unwrap_or("").to_string();
        if !url.starts_with("https://github.com/GloriousEggroll/proton-ge-custom/") {
            continue;
        }
        if name == format!("{tag}-x86_64.tar.gz") || name == format!("{tag}.tar.gz") {
            tar = Some((url, a["size"].as_u64().unwrap_or(0)));
        } else if name == format!("{tag}-x86_64.sha512sum") || name == format!("{tag}.sha512sum") {
            sha = Some(url);
        }
    }
    let (tar_url, size) = tar?;
    Some(Release { tag, tar_url, sha_url: sha?, size })
}

pub fn latest() -> Result<Release, String> {
    let o = Command::new("curl")
        .args(["-fsSL", "--max-time", "20", "-H", "Accept: application/vnd.github+json", RELEASES_API])
        .output()
        .map_err(|_| "curl is missing".to_string())?;
    if !o.status.success() {
        return Err("Couldn't reach GitHub to look for the newest Proton-GE.".into());
    }
    parse_release(&String::from_utf8_lossy(&o.stdout)).ok_or_else(|| "GitHub's answer didn't look like a Proton-GE release.".into())
}

/// First word of a `.sha512sum` file: the hash.
pub fn hash_of(sumfile: &str) -> Option<String> {
    let h = sumfile.split_whitespace().next()?;
    (h.len() == 128 && h.chars().all(|c| c.is_ascii_hexdigit())).then(|| h.to_lowercase())
}

/// Version numbers of a tag for ordering: `GE-Proton11-9` → [11, 9].
pub fn version_key(tag: &str) -> Vec<u64> {
    tag.trim_start_matches("GE-Proton").split(|c: char| !c.is_ascii_digit()).filter_map(|p| p.parse().ok()).collect()
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
}

/// Folders Steam reads extra Proton versions from (the repository Steam and the Flatpak Steam).
pub fn tool_dirs_in(home: &Path) -> Vec<PathBuf> {
    let native = home.join(".local/share/Steam");
    let flat = home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam");
    let mut out: Vec<PathBuf> = [native.clone(), flat].into_iter().filter(|r| r.exists()).map(|r| r.join("compatibilitytools.d")).collect();
    if out.is_empty() {
        out.push(native.join("compatibilitytools.d")); // Steam creates the rest when it first starts
    }
    out
}

pub fn tool_dirs() -> Vec<PathBuf> {
    tool_dirs_in(&home())
}

/// Installed Proton-GE versions in `dirs`, newest first, each once.
pub fn installed_in(dirs: &[PathBuf]) -> Vec<String> {
    let mut v: Vec<String> = dirs
        .iter()
        .filter_map(|d| std::fs::read_dir(d).ok())
        .flatten()
        .flatten()
        .filter(|e| e.path().join("proton").exists() || e.path().join("compatibilitytool.vdf").exists())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| valid_tag(n))
        .collect();
    v.sort_by(|a, b| version_key(b).cmp(&version_key(a)));
    v.dedup();
    v
}

pub fn installed() -> Vec<String> {
    installed_in(&tool_dirs())
}

/// Where the download goes while it is running (the page reads its size for progress).
pub fn download_path(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("zohara-{tag}.tar.gz"))
}

/// Downloads, checks and unpacks `rel` into every Steam folder. Err = what to tell the person.
pub fn install(rel: &Release) -> Result<(), String> {
    if !valid_tag(&rel.tag) {
        return Err("Unexpected version name.".into());
    }
    let dirs = tool_dirs();
    let file = download_path(&rel.tag);
    let result = install_inner(rel, &dirs, &file);
    let _ = std::fs::remove_file(&file);
    result
}

fn install_inner(rel: &Release, dirs: &[PathBuf], file: &Path) -> Result<(), String> {
    let sums = Command::new("curl").args(["-fsSL", "--max-time", "30", &rel.sha_url]).output().map_err(|_| "curl is missing".to_string())?;
    let want = hash_of(&String::from_utf8_lossy(&sums.stdout)).ok_or("Couldn't read the checksum Proton-GE publishes.")?;
    let dl = Command::new("curl")
        .args(["-fL", "--retry", "3", "--connect-timeout", "20", "-o"])
        .arg(file)
        .arg(&rel.tar_url)
        .output()
        .map_err(|_| "curl is missing".to_string())?;
    if !dl.status.success() {
        return Err("The download didn't finish. Check the internet connection and try again.".into());
    }
    let got = Command::new("sha512sum").arg(file).output().map_err(|_| "sha512sum is missing".to_string())?;
    if hash_of(&String::from_utf8_lossy(&got.stdout)).as_deref() != Some(want.as_str()) {
        return Err("The download didn't match Proton-GE's checksum, so it was not installed.".into());
    }
    for d in dirs {
        std::fs::create_dir_all(d).map_err(|e| format!("Couldn't make {}: {e}", d.display()))?;
        // Unpack beside the final place, then rename, so Steam never sees half a version.
        let tmp = d.join(format!(".{}.part", rel.tag));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
        let st = Command::new("tar").arg("-xzf").arg(file).arg("-C").arg(&tmp).status().map_err(|_| "tar is missing".to_string())?;
        // The archive's one top folder is named like the release, sometimes with "-x86_64" added.
        let from = std::fs::read_dir(&tmp).ok().and_then(|mut it| it.find_map(|e| e.ok().filter(|e| e.path().is_dir()).map(|e| e.path())));
        let Some(from) = from.filter(|f| st.success() && f.join("proton").exists()) else {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err("The download could not be unpacked (is the disk full?).".into());
        };
        let to = d.join(&rel.tag);
        let _ = std::fs::remove_dir_all(&to);
        std::fs::rename(&from, &to).map_err(|e| e.to_string())?;
        let _ = std::fs::remove_dir_all(&tmp);
    }
    Ok(())
}

pub fn remove(tag: &str) -> Result<(), String> {
    if !valid_tag(tag) {
        return Err("Unexpected version name.".into());
    }
    for d in tool_dirs() {
        let p = d.join(tag);
        if p.exists() {
            std::fs::remove_dir_all(&p).map_err(|e| format!("Couldn't remove {}: {e}", p.display()))?;
        }
    }
    Ok(())
}

pub fn protonup_qt_present() -> bool {
    Command::new("sh").args(["-c", "command -v protonup-qt"]).output().map(|o| o.status.success()).unwrap_or(false)
        || Command::new("flatpak").args(["info", "net.davidotek.pupgui2"]).output().map(|o| o.status.success()).unwrap_or(false)
}

/// How to start ProtonUp-Qt: the program if it's installed, else the Flatpak.
pub fn protonup_qt_command() -> Vec<String> {
    if Command::new("sh").args(["-c", "command -v protonup-qt"]).output().map(|o| o.status.success()).unwrap_or(false) {
        vec!["protonup-qt".into()]
    } else {
        vec!["flatpak".into(), "run".into(), "net.davidotek.pupgui2".into()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RELEASE: &str = r#"{"tag_name":"GE-Proton11-9","assets":[
      {"name":"GE-Proton11-9-aarch64.tar.gz","browser_download_url":"https://github.com/GloriousEggroll/proton-ge-custom/releases/download/GE-Proton11-9/GE-Proton11-9-aarch64.tar.gz","size":647938895},
      {"name":"GE-Proton11-9-x86_64.sha512sum","browser_download_url":"https://github.com/GloriousEggroll/proton-ge-custom/releases/download/GE-Proton11-9/GE-Proton11-9-x86_64.sha512sum","size":158},
      {"name":"GE-Proton11-9-x86_64.tar.gz","browser_download_url":"https://github.com/GloriousEggroll/proton-ge-custom/releases/download/GE-Proton11-9/GE-Proton11-9-x86_64.tar.gz","size":564697280}]}"#;

    #[test]
    fn the_x86_64_archive_and_its_checksum_are_picked() {
        let r = parse_release(RELEASE).unwrap();
        assert_eq!(r.tag, "GE-Proton11-9");
        assert!(r.tar_url.ends_with("GE-Proton11-9-x86_64.tar.gz"));
        assert!(r.sha_url.ends_with("GE-Proton11-9-x86_64.sha512sum"));
        assert_eq!(r.size, 564697280);
    }

    #[test]
    fn odd_releases_are_refused() {
        assert!(parse_release("{}").is_none());
        assert!(parse_release("not json").is_none());
        // a tag with shell characters never gets near a script or a path
        assert!(parse_release(&RELEASE.replace("GE-Proton11-9\",\"assets", "GE-Proton1;rm -rf ~\",\"assets")).is_none());
        // a download from somewhere else is ignored, so there is no archive to install
        assert!(parse_release(&RELEASE.replace("github.com/GloriousEggroll", "evil.example/x")).is_none());
        assert!(!valid_tag("../etc"));
        assert!(!valid_tag("Proton-9"));
        assert!(valid_tag("GE-Proton9-27"));
    }

    #[test]
    fn checksum_file_is_read() {
        let h = "a".repeat(128);
        assert_eq!(hash_of(&format!("{h}  GE-Proton11-9-x86_64.tar.gz\n")).as_deref(), Some(h.as_str()));
        assert!(hash_of("short  x").is_none());
        assert!(hash_of("").is_none());
    }

    #[test]
    fn newest_version_sorts_first_and_tens_beat_nines() {
        assert!(version_key("GE-Proton11-9") > version_key("GE-Proton9-27"));
        assert!(version_key("GE-Proton9-27") > version_key("GE-Proton9-4"));
    }

    #[test]
    fn multilib_is_found_only_when_switched_on() {
        assert!(multilib_enabled("[core]\nInclude=x\n[multilib]\nInclude=x\n"));
        assert!(!multilib_enabled("#[multilib]\n#Include=x\n"));
        assert!(!multilib_enabled(""));
    }

    #[test]
    fn installed_versions_are_read_from_the_steam_folders() {
        let home = std::env::temp_dir().join(format!("zs-proton-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let native = home.join(".local/share/Steam/compatibilitytools.d");
        let flat = home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam/compatibilitytools.d");
        for (d, t) in [(&native, "GE-Proton9-4"), (&native, "GE-Proton11-9"), (&flat, "GE-Proton11-9")] {
            std::fs::create_dir_all(d.join(t)).unwrap();
            std::fs::write(d.join(t).join("proton"), "x").unwrap();
        }
        std::fs::create_dir_all(native.join("NotProton")).unwrap(); // other tools are left out
        std::fs::create_dir_all(native.join("GE-Proton1-1")).unwrap(); // half-unpacked: no proton file
        let dirs = tool_dirs_in(&home);
        assert_eq!(dirs.len(), 2);
        assert_eq!(installed_in(&dirs), vec!["GE-Proton11-9", "GE-Proton9-4"]);
        // with no Steam at all, the repository Steam's folder is where it goes
        let bare = std::env::temp_dir().join(format!("zs-proton-bare-{}", std::process::id()));
        assert_eq!(tool_dirs_in(&bare), vec![bare.join(".local/share/Steam/compatibilitytools.d")]);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn the_setup_script_is_valid_and_safe() {
        let s = support_script();
        assert!(Command::new("sh").args(["-n", "-c", &s]).status().unwrap().success());
        assert!(s.contains("pacman -S --needed --noconfirm steam"));
        assert!(s.contains(&format!("exit {EXIT_UPDATES_WAITING}")));
        assert!(explain_exit(Some(EXIT_UPDATES_WAITING)).unwrap().contains("Update the computer"));
        assert!(explain_exit(Some(0)).is_none());
    }

    #[test]
    fn multilib_edit_uncomments_exactly_the_two_lines() {
        let dir = std::env::temp_dir().join(format!("zs-ml-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let conf = dir.join("pacman.conf");
        std::fs::write(&conf, "[extra]\nInclude = /m\n\n#[multilib]\n#Include = /m\n\n#[custom]\n#Include = /c\n").unwrap();
        let sed = r"/^#\[multilib\]/{s/^#//;n;s/^#//}";
        assert!(Command::new("sed").args(["-i", sed]).arg(&conf).status().unwrap().success());
        let out = std::fs::read_to_string(&conf).unwrap();
        assert!(out.contains("\n[multilib]\nInclude = /m\n"));
        assert!(out.contains("#[custom]\n#Include = /c"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_checked_archive_is_unpacked_and_a_bad_one_is_refused() {
        let base = std::env::temp_dir().join(format!("zs-proton-fake-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let src = base.join("GE-Proton9-9-x86_64");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("proton"), "#!/bin/sh\n").unwrap();
        let tgz = base.join("a.tar.gz");
        assert!(Command::new("tar").arg("-czf").arg(&tgz).arg("-C").arg(&base).arg("GE-Proton9-9-x86_64").status().unwrap().success());
        let sum = Command::new("sha512sum").arg(&tgz).output().unwrap();
        let sumfile = base.join("a.sha512sum");
        std::fs::write(&sumfile, &sum.stdout).unwrap();
        let rel = Release { tag: "GE-Proton9-9".into(), tar_url: format!("file://{}", tgz.display()), sha_url: format!("file://{}", sumfile.display()), size: 1 };
        let dirs = vec![base.join("home/.local/share/Steam/compatibilitytools.d")];
        let file = base.join("dl.tar.gz");
        install_inner(&rel, &dirs, &file).unwrap();
        assert_eq!(installed_in(&dirs), vec!["GE-Proton9-9"]);
        assert!(dirs[0].join("GE-Proton9-9/proton").exists());
        assert!(!dirs[0].join(".GE-Proton9-9.part").exists());
        // a wrong checksum installs nothing
        let _ = std::fs::remove_dir_all(&dirs[0]);
        std::fs::write(&sumfile, format!("{}  x\n", "0".repeat(128))).unwrap();
        let err = install_inner(&rel, &dirs, &file).unwrap_err();
        assert!(err.contains("checksum"), "{err}");
        assert!(installed_in(&dirs).is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Downloads the real newest Proton-GE (about 560 MB) into a throwaway home. Run by hand:
    /// `cargo test live_proton_ge -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_proton_ge_downloads_checks_and_unpacks() {
        let home = std::env::temp_dir().join(format!("zs-proton-live-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        std::env::set_var("HOME", &home);
        let rel = latest().expect("latest release");
        println!("newest: {} ({} bytes)", rel.tag, rel.size);
        install(&rel).expect("install");
        assert_eq!(installed(), vec![rel.tag.clone()]);
        assert!(!download_path(&rel.tag).exists(), "download file is cleaned up");
        remove(&rel.tag).unwrap();
        assert!(installed().is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }
}
