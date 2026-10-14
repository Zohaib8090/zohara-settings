//! Java for games (Minecraft and its launchers): which Java is installed, which Java a Minecraft version needs, and a
//! way to install or switch the system Java.
//!
//! Two kinds of Java exist on a computer: the **system** ones (`archlinux-java status`, packages `jre8-openjdk` …) that
//! launchers installed as normal programs use, and the ones a **launcher brings itself** (Modrinth's Zulu, Prism's and the
//! official launcher's runtimes) that it picks per game. A launcher installed as a Flatpak cannot see the system Java at
//! all: it uses its own.

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemJava {
    /// `java-21-openjdk`
    pub name: String,
    pub major: u32,
    pub is_default: bool,
}

/// The major version in a name like `java-21-openjdk`, `java-8-openjdk` or `java-8-openjdk/jre`.
pub fn major_of_name(name: &str) -> Option<u32> {
    name.split('/').next()?.split('-').nth(1)?.parse().ok()
}

/// `archlinux-java status` output -> the installed system Javas.
pub fn parse_status(text: &str) -> Vec<SystemJava> {
    text.lines()
        .filter(|l| l.starts_with(' ') && !l.trim().is_empty())
        .filter_map(|l| {
            let t = l.trim();
            let name = t.split_whitespace().next()?.to_string();
            Some(SystemJava { major: major_of_name(&name)?, is_default: t.contains("(default)"), name })
        })
        .collect()
}

pub fn system_javas() -> Vec<SystemJava> {
    Command::new("archlinux-java").arg("status").output().map(|o| parse_status(&String::from_utf8_lossy(&o.stdout))).unwrap_or_default()
}

/// A Java a launcher keeps for itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LauncherJava {
    pub launcher: &'static str,
    pub major: u32,
    pub path: PathBuf,
}

/// The major version from a Java folder's `release` file (`JAVA_VERSION="25.0.4.1"`, or `"1.8.0_402"` for Java 8).
pub fn major_from_release(text: &str) -> Option<u32> {
    let v = text.lines().find_map(|l| l.strip_prefix("JAVA_VERSION="))?.trim().trim_matches('"');
    let mut parts = v.split(['.', '_']);
    let first: u32 = parts.next()?.parse().ok()?;
    if first == 1 { parts.next()?.parse().ok() } else { Some(first) }
}

fn runtimes_in(launcher: &'static str, dir: &Path, depth: u32, out: &mut Vec<LauncherJava>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if !p.is_dir() {
            continue;
        }
        if let Some(major) = std::fs::read_to_string(p.join("release")).ok().and_then(|t| major_from_release(&t)) {
            out.push(LauncherJava { launcher, major, path: p });
        } else if depth > 0 {
            runtimes_in(launcher, &p, depth - 1, out);
        }
    }
}

/// The Javas that Minecraft launchers brought, under `home`.
pub fn launcher_javas_in(home: &Path) -> Vec<LauncherJava> {
    let mut out = Vec::new();
    let spots: [(&'static str, &str); 6] = [
        ("Modrinth App (Flatpak)", ".var/app/com.modrinth.ModrinthApp/data/ModrinthApp/meta/java_versions"),
        ("Modrinth App", ".local/share/ModrinthApp/meta/java_versions"),
        ("Prism Launcher", ".local/share/PrismLauncher/java"),
        ("Prism Launcher (Flatpak)", ".var/app/org.prismlauncher.PrismLauncher/data/PrismLauncher/java"),
        ("Minecraft launcher", ".minecraft/runtime"),
        ("Minecraft launcher (Flatpak)", ".var/app/com.mojang.Minecraft/data/minecraft/runtime"),
    ];
    for (name, rel) in spots {
        runtimes_in(name, &home.join(rel), 3, &mut out);
    }
    out.sort_by(|a, b| (a.launcher, a.major).cmp(&(b.launcher, b.major)));
    out
}

pub fn launcher_javas() -> Vec<LauncherJava> {
    launcher_javas_in(&PathBuf::from(std::env::var("HOME").unwrap_or_default()))
}

/// The Java a Minecraft version needs at least, as far as Mojang has published it:
/// up to 1.16 Java 8, 1.17 Java 16, 1.18 to 1.20.4 Java 17, 1.20.5 to 1.21.x Java 21; the newer year-numbered
/// versions (26.x) Java 25. None = not a version number.
pub fn java_for_minecraft(version: &str) -> Option<u32> {
    let mut it = version.trim().split(['.', '-']).map(|p| p.parse::<u32>().ok());
    let major = it.next()??;
    let minor = it.next().flatten().unwrap_or(0);
    let patch = it.next().flatten().unwrap_or(0);
    if major >= 20 {
        return Some(25); // the year-numbered releases (26.1, 26.3 …)
    }
    if major != 1 {
        return None;
    }
    Some(match (minor, patch) {
        (..=16, _) => 8,
        (17, _) => 16,
        (18..=19, _) => 17,
        (20, 0..=4) => 17,
        _ => 21,
    })
}

/// The system Java packages Zohara offers, by major version.
pub const PACKAGES: [(u32, &str); 4] = [(8, "jre8-openjdk"), (17, "jre17-openjdk"), (21, "jre21-openjdk"), (25, "jre25-openjdk")];

pub fn package_for(major: u32) -> Option<&'static str> {
    PACKAGES.iter().find(|(m, _)| *m == major).map(|(_, p)| *p)
}

/// The script (run as administrator) that installs a system Java.
pub fn install_script(major: u32) -> Result<String, String> {
    let pkg = package_for(major).ok_or("That Java version is not offered.")?;
    Ok(format!("pacman -S --needed --noconfirm {pkg}"))
}

/// The script that makes `name` (one of the installed system Javas) the default.
pub fn default_script(name: &str, installed: &[SystemJava]) -> Result<String, String> {
    if !installed.iter().any(|j| j.name == name) {
        return Err("That Java is not installed.".into());
    }
    Ok(format!("archlinux-java set {name}"))
}

pub fn run_admin(script: &str) -> Result<(), String> {
    match Command::new("pkexec").args(["sh", "-c", script]).status() {
        Ok(s) if s.success() => Ok(()),
        Ok(s) if s.code() == Some(126) || s.code() == Some(127) => Err("The password was not given.".into()),
        Ok(_) => Err("It did not finish. Check the internet and try again.".into()),
        Err(_) => Err("Couldn't ask for the password.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archlinux_java_status_is_read() {
        let t = "Available Java environments:\n  java-17-openjdk\n  java-21-openjdk (default)\n  java-8-openjdk/jre\n";
        let v = parse_status(t);
        assert_eq!(v.iter().map(|j| (j.major, j.is_default)).collect::<Vec<_>>(), vec![(17, false), (21, true), (8, false)]);
        assert!(parse_status("").is_empty());
        assert!(parse_status("No Java environment is set as default anymore\n").is_empty());
    }

    #[test]
    fn release_files_give_the_major_version() {
        assert_eq!(major_from_release("JAVA_VERSION=\"25.0.4.1\"\n"), Some(25));
        assert_eq!(major_from_release("X=1\nJAVA_VERSION=\"1.8.0_402\"\n"), Some(8));
        assert_eq!(major_from_release("JAVA_VERSION=\"17.0.9\"\n"), Some(17));
        assert_eq!(major_from_release("nothing"), None);
    }

    #[test]
    fn minecraft_versions_map_to_java() {
        for (v, j) in [("1.8.9", 8), ("1.12.2", 8), ("1.16.5", 8), ("1.17.1", 16), ("1.18.2", 17), ("1.19.4", 17), ("1.20.1", 17), ("1.20.4", 17), ("1.20.5", 21), ("1.20.6", 21), ("1.21", 21), ("1.21.4", 21), ("26.1", 25), ("26.3", 25)] {
            assert_eq!(java_for_minecraft(v), Some(j), "{v}");
        }
        assert_eq!(java_for_minecraft("hello"), None);
        assert_eq!(java_for_minecraft(""), None);
    }

    #[test]
    fn launcher_javas_are_found_under_home() {
        let home = std::env::temp_dir().join(format!("zs-java-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let z = home.join(".var/app/com.modrinth.ModrinthApp/data/ModrinthApp/meta/java_versions/zulu25-linux_x64");
        std::fs::create_dir_all(&z).unwrap();
        std::fs::write(z.join("release"), "JAVA_VERSION=\"25.0.4.1\"\n").unwrap();
        let rt = home.join(".minecraft/runtime/java-runtime-delta/linux/java-runtime-delta");
        std::fs::create_dir_all(&rt).unwrap();
        std::fs::write(rt.join("release"), "JAVA_VERSION=\"21.0.5\"\n").unwrap();
        let v = launcher_javas_in(&home);
        assert_eq!(v.iter().map(|j| (j.launcher, j.major)).collect::<Vec<_>>(), vec![("Minecraft launcher", 21), ("Modrinth App (Flatpak)", 25)]);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn only_listed_packages_reach_the_script() {
        assert_eq!(install_script(21).unwrap(), "pacman -S --needed --noconfirm jre21-openjdk");
        assert!(install_script(11).is_err());
        let have = vec![SystemJava { name: "java-21-openjdk".into(), major: 21, is_default: false }];
        assert_eq!(default_script("java-21-openjdk", &have).unwrap(), "archlinux-java set java-21-openjdk");
        assert!(default_script("java-21-openjdk; rm -rf /", &have).is_err());
    }
}

#[cfg(test)]
mod real {
    /// `cargo test real_launcher_javas -- --ignored --nocapture`: what is found under the real home.
    #[test]
    #[ignore]
    fn real_launcher_javas() {
        for j in super::launcher_javas() {
            println!("FOUND Java {} in {} at {}", j.major, j.launcher, j.path.display());
        }
    }
}
