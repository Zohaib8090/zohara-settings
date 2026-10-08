//! KDE Wallet on/off. Off means: the wallet service refuses to start (no "create a wallet" window), and the
//! Chromium-family browsers are told to keep their saved passwords in their own basic store instead of the wallet
//! (`--password-store=basic` in each browser's flags file, which the Arch browser launchers read at start).

use crate::backend::kconfig;
use std::path::PathBuf;
use std::process::Command;

const RC: &str = "kwalletrc";
pub const FLAG: &str = "--password-store=basic";
/// Flags files of the browsers Zohara offers (the launchers read `$XDG_CONFIG_HOME/<name>`).
const FLAG_FILES: [&str; 4] = ["brave-origin-flags.conf", "brave-flags.conf", "chromium-flags.conf", "chrome-flags.conf"];

fn config_dir() -> PathBuf {
    std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config"))
}

/// On unless the person turned it off.
pub fn is_enabled() -> bool {
    kconfig::read(RC, &["Wallet"], "Enabled").map(|v| v != "false").unwrap_or(true)
}

/// The text of a flags file with our flag added (`on == false`) or removed (`on == true`). Other lines are kept.
pub fn flags_with(text: &str, wallet_on: bool) -> String {
    let mut lines: Vec<&str> = text.lines().filter(|l| l.trim() != FLAG).collect();
    if !wallet_on {
        lines.push(FLAG);
    }
    let mut out = lines.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

fn done_flag() -> PathBuf {
    config_dir().join("zohara/wallet-default")
}

/// A wallet file exists: the person already uses KDE Wallet, so switching it off would hide what is saved in it.
fn wallet_in_use() -> bool {
    let home = std::env::var("HOME").unwrap_or_default();
    std::fs::read_dir(PathBuf::from(home).join(".local/share/kwalletd"))
        .map(|rd| rd.flatten().any(|e| e.path().extension().map(|x| x == "kwl").unwrap_or(false)))
        .unwrap_or(false)
}

/// Autostart, once per person: KDE Wallet starts switched off (no wallet window the first time a browser runs). The
/// person can switch it on in Settings > Privacy & security. Not done when a wallet already exists.
pub fn apply_default() -> i32 {
    if done_flag().exists() {
        return 0;
    }
    if !wallet_in_use() {
        set_enabled(false);
    }
    let p = done_flag();
    let _ = std::fs::create_dir_all(p.parent().unwrap_or(&p));
    let _ = std::fs::write(p, "done\n");
    0
}

pub fn set_enabled(on: bool) -> bool {
    kconfig::write_typed(RC, &["Wallet"], "Enabled", "bool", if on { "true" } else { "false" });
    if !on {
        // First-run window off too, and stop a wallet service that is already running.
        kconfig::write_typed(RC, &["Wallet"], "First Use", "bool", "false");
        let _ = Command::new("kquitapp6").arg("kwalletd6").status();
    }
    let dir = config_dir();
    let _ = std::fs::create_dir_all(&dir);
    for name in FLAG_FILES {
        let path = dir.join(name);
        let old = std::fs::read_to_string(&path).unwrap_or_default();
        // Leave browsers that were never set up alone when the wallet is being switched back on.
        if on && !path.exists() {
            continue;
        }
        let new = flags_with(&old, on);
        if new.is_empty() {
            let _ = std::fs::remove_file(&path);
        } else if std::fs::write(&path, new).is_err() {
            return false;
        }
    }
    is_enabled() == on
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_is_added_once_and_removed_cleanly() {
        assert_eq!(flags_with("", false), "--password-store=basic\n");
        assert_eq!(flags_with("--password-store=basic\n", false), "--password-store=basic\n");
        assert_eq!(flags_with("--ozone-platform=wayland\n", false), "--ozone-platform=wayland\n--password-store=basic\n");
        assert_eq!(flags_with("--ozone-platform=wayland\n--password-store=basic\n", true), "--ozone-platform=wayland\n");
        assert_eq!(flags_with("--password-store=basic\n", true), "");
        assert_eq!(flags_with("", true), "");
    }
}
