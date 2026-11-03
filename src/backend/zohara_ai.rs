//! Zohara AI (ZO-AI, repo `ZO-AI`): installing, opening and removing the app from Settings.
//!
//! Settings only manages the package (`zohara-ai`) and starts the app; the AI itself, its keys and its chats live in the app.
//! Everything here blocks (pacman, a polkit prompt), so callers run it with `worker::in_background`.

use std::process::{Command, Stdio};

pub const PACKAGE: &str = "zohara-ai";

/// Is the app installed on this computer?
pub fn installed() -> bool {
    std::path::Path::new("/usr/bin/zo-ai").exists()
}

/// The installed package version (`pacman -Q`), if the package is there.
pub fn version() -> Option<String> {
    let out = Command::new("pacman").args(["-Q", PACKAGE]).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    if !out.status.success() {
        return None;
    }
    parse_version(&String::from_utf8_lossy(&out.stdout))
}

/// `zohara-ai 0.1.0-1` -> `0.1.0-1`.
fn parse_version(line: &str) -> Option<String> {
    let mut it = line.split_whitespace();
    if it.next()? != PACKAGE {
        return None;
    }
    it.next().map(str::to_string)
}

fn run(cmd: &mut Command) -> Result<(), String> {
    let out = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).output().map_err(|e| e.to_string())?;
    match out.status.code() {
        Some(0) => Ok(()),
        Some(126) | Some(127) => Err("Administrator approval was not given.".into()),
        _ => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
    }
}

/// Turns a pacman failure into a sentence the person can act on.
fn explain(err: String) -> String {
    if err.contains("target not found") {
        "Zohara AI is not in your update channel yet. Update the system first (Settings > Zohara Update), or switch to the alpha channel there.".into()
    } else if err.is_empty() {
        "Something went wrong. Try again.".into()
    } else {
        err
    }
}

/// Installs the package (administrator prompt). Uses the package lists already on the computer: a bare `pacman -Sy` is never
/// safe on Arch, so a missing package says to update first.
pub fn install() -> Result<(), String> {
    run(Command::new("pkexec").args(["pacman", "-S", "--needed", "--noconfirm", PACKAGE])).map_err(explain)
}

/// Removes the package (administrator prompt). Chats, projects and settings in the home folder stay.
pub fn remove() -> Result<(), String> {
    run(Command::new("pkexec").args(["pacman", "-R", "--noconfirm", PACKAGE])).map_err(explain)
}

/// Starts the app, detached from Settings.
pub fn open() -> Result<(), String> {
    Command::new("zo-ai")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|_| "The app could not be started.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_read_from_pacman_output() {
        assert_eq!(parse_version("zohara-ai 0.1.0-1\n").as_deref(), Some("0.1.0-1"));
        assert_eq!(parse_version("other-pkg 1.0-1\n"), None);
        assert_eq!(parse_version(""), None);
    }

    #[test]
    fn a_missing_package_points_to_updates() {
        assert!(explain("error: target not found: zohara-ai".into()).contains("Zohara Update"));
        assert_eq!(explain("boom".into()), "boom");
        assert!(!explain(String::new()).is_empty());
    }
}
