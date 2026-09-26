//! Where Settings is running.
//!
//! Zohara for phones is Arch Linux ARM run by proot inside Termux. It has no
//! kernel of its own, no systemd or system bus, and no direct hardware access:
//! Android owns Wi-Fi, Bluetooth, the screen, sound devices and power. Pages
//! that control those can't work there, so they are hidden instead of showing
//! switches that do nothing. The phone system ships `/etc/zohara-proot`.

use std::path::Path;
use std::sync::OnceLock;

/// Sidebar pages that need things a phone (proot) doesn't have.
pub const PHONE_HIDDEN_PAGES: &[&str] = &[
    "Bluetooth & devices",  // BlueZ, Bluetooth hardware
    "Network & internet",   // NetworkManager
    "Accounts",             // accounts-daemon on the system bus
    "Time & language",      // timedatectl / localectl (systemd)
    "Gaming",               // GPU drivers, GameMode
    "Privacy & security",   // firewall (kernel), camera/microphone monitoring
    "Display",              // screen control (Termux:X11 owns the display)
    "Sound",                // PipeWire devices (sound goes through Termux)
    "Power & battery",      // UPower, PowerDevil
    "Mouse",                // libinput device settings
    "Touchpad",             // libinput device settings
    "Printers",             // CUPS
    "Zohara Link",          // pairs a phone with a computer; this is the phone
    "Troubleshoot",         // checks systemd services that don't exist here
];

pub fn is_phone() -> bool {
    static PHONE: OnceLock<bool> = OnceLock::new();
    *PHONE.get_or_init(|| Path::new("/etc/zohara-proot").exists() || std::env::var("ZOHARA_PROOT").as_deref() == Ok("1"))
}

pub fn page_hidden(label: &str) -> bool {
    is_phone() && PHONE_HIDDEN_PAGES.contains(&label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_pages_are_real_page_labels() {
        // A typo here would silently leave a broken page visible on phones.
        let src = include_str!("../main.rs");
        for label in PHONE_HIDDEN_PAGES {
            assert!(src.contains(&format!("label: \"{label}\"")), "no sidebar page called {label:?}");
        }
    }
}
