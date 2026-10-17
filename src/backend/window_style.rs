//! The Zohara window decoration: round title-bar buttons (dash, square, cross) like Zohara's own apps draw them.
//! The decoration itself is a KDE package, `data/kwin/decorations/kwin4_decoration_qml_zohara`, installed to
//! `/usr/share/kwin/decorations`. This module only switches KWin to it and back.
//!
//! Traps found in the test VM (2026-10-17): a QML decoration is named by its plain id in `kwinrc`
//! (`theme=kwin4_decoration_qml_zohara`, with `library=org.kde.kwin.aurorae`); the `__aurorae__qml__` prefix is wrong and KWin
//! answers "Couldn't find QML Decoration". KWin keeps a decoration's QML in memory, so a changed file shows only after the next
//! sign-in (or under a new id).

use crate::backend::kconfig;
use std::path::Path;

pub const ID: &str = "kwin4_decoration_qml_zohara";
const GROUP: &[&str] = &["org.kde.kdecoration2"];
const SAVED: &[&str] = &["Zohara"];

/// The two keys to write for round buttons on or off. `previous` is what was set before the round buttons were switched on.
pub fn keys(on: bool, previous: Option<(String, String)>) -> [(&'static str, String); 2] {
    if on {
        [("library", "org.kde.kwin.aurorae".to_string()), ("theme", ID.to_string())]
    } else {
        let (library, theme) = previous.filter(|(_, t)| t != ID).unwrap_or(("org.kde.breeze".to_string(), "Breeze".to_string()));
        [("library", library), ("theme", theme)]
    }
}

pub fn installed() -> bool {
    ["/usr/share/kwin/decorations"].iter().any(|d| Path::new(d).join(ID).join("metadata.json").exists())
}

pub fn is_round() -> bool {
    kconfig::read("kwinrc", GROUP, "theme").as_deref() == Some(ID)
}

/// Switches to the round buttons, or back to what the person had (Breeze when that is not known).
pub fn set_round(on: bool) {
    let previous = if on {
        // remember the current style once, so "off" gives it back
        let library = kconfig::read("kwinrc", GROUP, "library");
        let theme = kconfig::read("kwinrc", GROUP, "theme");
        if let (Some(l), Some(t)) = (library, theme) {
            if t != ID {
                kconfig::write("kwinrc", SAVED, "PreviousDecorationLibrary", &l);
                kconfig::write("kwinrc", SAVED, "PreviousDecorationTheme", &t);
            }
        }
        None
    } else {
        match (kconfig::read("kwinrc", SAVED, "PreviousDecorationLibrary"), kconfig::read("kwinrc", SAVED, "PreviousDecorationTheme")) {
            (Some(l), Some(t)) => Some((l, t)),
            _ => None,
        }
    };
    for (key, value) in keys(on, previous) {
        kconfig::write_notify("kwinrc", GROUP, key, &value);
    }
    kconfig::kwin_reconfigure();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_uses_the_plain_id_with_the_aurorae_library() {
        let k = keys(true, None);
        assert_eq!(k[0], ("library", "org.kde.kwin.aurorae".to_string()));
        assert_eq!(k[1], ("theme", "kwin4_decoration_qml_zohara".to_string()));
        assert!(!k[1].1.contains("__aurorae__"));
    }

    #[test]
    fn off_gives_back_what_was_there_or_breeze() {
        let k = keys(false, Some(("org.kde.kwin.aurorae".into(), "__aurorae__svg__Other".into())));
        assert_eq!(k[1].1, "__aurorae__svg__Other");
        let k = keys(false, None);
        assert_eq!((k[0].1.as_str(), k[1].1.as_str()), ("org.kde.breeze", "Breeze"));
        // never "restore" our own style as the previous one
        let k = keys(false, Some(("org.kde.kwin.aurorae".into(), ID.into())));
        assert_eq!(k[1].1, "Breeze");
    }
}
