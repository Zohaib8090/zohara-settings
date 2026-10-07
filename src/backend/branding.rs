//! The Zohara logo on the start button. Plasma's start menu takes its button icon from the icon theme (Fluent's is the
//! Windows logo), so the launcher is pointed at `zohara-start`, an icon shipped in the hicolor fallback theme that
//! every icon set can see. `zohara-settings --branding` (autostart) does it once per person; Reset does it again.

use std::path::PathBuf;

pub const ICON: &str = "zohara-start";

fn flag_path() -> PathBuf {
    std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config"))
        .join("zohara/start-icon")
}

/// JS that gives every start menu on every panel the Zohara icon.
pub fn script() -> String {
    format!(
        r#"panels().forEach(function (p) {{ p.widgets().forEach(function (w) {{
  if (w.type.indexOf("org.kde.plasma.kickoff") == 0 || w.type.indexOf("org.kde.plasma.kicker") == 0 || w.type.indexOf("org.kde.plasma.kickerdash") == 0) {{ w.currentConfigGroup = ["General"]; w.writeConfig("icon", "{ICON}"); }}
}}); }});"#
    )
}

pub fn apply() -> bool {
    crate::pages::start_menu::plasma_script(&script()).is_some()
}

/// Autostart run: once, when the icon is installed and the panel answers (it may still be starting after login).
pub fn run_once() -> i32 {
    if flag_path().exists() || !std::path::Path::new("/usr/share/icons/hicolor/scalable/apps/zohara-start.svg").exists() {
        return 0;
    }
    for _ in 0..12 {
        if apply() {
            let p = flag_path();
            let _ = std::fs::create_dir_all(p.parent().unwrap_or(&p));
            let _ = std::fs::write(p, "done\n");
            return 0;
        }
        std::thread::sleep(std::time::Duration::from_secs(5));
    }
    1
}

#[cfg(test)]
mod tests {
    #[test]
    fn script_sets_the_icon_on_start_menus() {
        let s = super::script();
        assert!(s.contains("writeConfig(\"icon\", \"zohara-start\")"));
        assert!(s.contains("org.kde.plasma.kickoff"));
    }
}
