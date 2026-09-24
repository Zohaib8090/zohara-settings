//! Accessibility settings backed by Plasma: high-contrast colour scheme,
//! cursor size, reduced animations, text size (via the Fonts dialog), and
//! visual bell.

use crate::backend::kconfig;
use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use std::process::Command;

const HIGH_CONTRAST: &str = "BreezeHighContrast";

fn state_file() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    let base = std::env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| format!("{home}/.config"));
    std::path::PathBuf::from(base).join("zohara").join("previous-colorscheme")
}

fn current_scheme() -> String {
    kconfig::read("kdeglobals", &["General"], "ColorScheme").unwrap_or_default()
}

fn apply_scheme(name: String) {
    kconfig::spawn(move || {
        let _ = Command::new("plasma-apply-colorscheme").arg(&name).status();
    });
}

fn vision_group() -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("Vision");

    let hc = adw::SwitchRow::new();
    hc.set_title("High contrast");
    hc.set_subtitle("Use Breeze High Contrast colours for the desktop and apps");
    hc.add_prefix(&gtk4::Image::from_icon_name("preferences-desktop-color-symbolic"));
    hc.set_active(current_scheme() == HIGH_CONTRAST);
    hc.connect_active_notify(|r| {
        if r.is_active() {
            let prev = current_scheme();
            if prev != HIGH_CONTRAST && !prev.is_empty() {
                let path = state_file();
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::write(path, prev);
            }
            apply_scheme(HIGH_CONTRAST.to_string());
        } else {
            let prev = std::fs::read_to_string(state_file())
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty() && s != HIGH_CONTRAST)
                .unwrap_or_else(|| "BreezeDark".to_string());
            apply_scheme(prev);
        }
    });
    g.add(&hc);

    let cursor = adw::ComboRow::new();
    cursor.set_title("Cursor size");
    cursor.set_subtitle("Applies to newly opened apps; log out and in for everything");
    cursor.add_prefix(&gtk4::Image::from_icon_name("input-mouse-symbolic"));
    const SIZES: [u32; 6] = [24, 32, 48, 64, 96, 128];
    let labels: Vec<String> = SIZES.iter().map(|s| format!("{s} px")).collect();
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    cursor.set_model(Some(&gtk4::StringList::new(&refs)));
    let cur: u32 = kconfig::read("kcminputrc", &["Mouse"], "cursorSize").and_then(|v| v.parse().ok()).unwrap_or(24);
    cursor.set_selected(SIZES.iter().position(|s| *s == cur).unwrap_or(0) as u32);
    cursor.connect_selected_notify(|c| {
        let Some(size) = SIZES.get(c.selected() as usize).copied() else { return };
        kconfig::spawn(move || {
            let theme = kconfig::read("kcminputrc", &["Mouse"], "cursorTheme").unwrap_or_else(|| "breeze_cursors".into());
            let _ = Command::new("plasma-apply-cursortheme").args(["--size", &size.to_string(), &theme]).status();
        });
    });
    g.add(&cursor);

    let text = adw::ActionRow::new();
    text.set_title("Text size");
    text.set_subtitle("Change the size of text across the desktop in Fonts");
    text.add_prefix(&gtk4::Image::from_icon_name("preferences-desktop-font-symbolic"));
    text.add_suffix(&gtk4::Image::from_icon_name("go-next-symbolic"));
    text.set_activatable(true);
    text.connect_activated(super::personalization::open_fonts_window);
    g.add(&text);

    let anim = adw::SwitchRow::new();
    anim.set_title("Reduce animations");
    anim.set_subtitle("Turn off window and desktop animation effects");
    anim.add_prefix(&gtk4::Image::from_icon_name("media-playback-pause-symbolic"));
    anim.set_active(
        kconfig::read("kdeglobals", &["KDE"], "AnimationDurationFactor")
            .and_then(|v| v.parse::<f64>().ok())
            .map(|f| f == 0.0)
            .unwrap_or(false),
    );
    anim.connect_active_notify(|r| {
        let on = r.is_active();
        kconfig::spawn(move || {
            if on {
                kconfig::write("kdeglobals", &["KDE"], "AnimationDurationFactor", "0");
            } else {
                kconfig::delete("kdeglobals", &["KDE"], "AnimationDurationFactor");
            }
            kconfig::kwin_reconfigure();
        });
    });
    g.add(&anim);
    g
}

fn hearing_group() -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("Hearing");

    let bell = adw::SwitchRow::new();
    bell.set_title("Visual bell");
    bell.set_subtitle("Flash the screen instead of playing the system bell. Takes effect after you log in again");
    bell.add_prefix(&gtk4::Image::from_icon_name("audio-volume-high-symbolic"));
    bell.set_active(kconfig::read("kaccessrc", &["Bell"], "VisibleBell").as_deref() == Some("true"));
    bell.connect_active_notify(|r| {
        let v = if r.is_active() { "true" } else { "false" };
        kconfig::spawn(move || kconfig::write("kaccessrc", &["Bell"], "VisibleBell", v));
    });
    g.add(&bell);
    g
}

pub fn build() -> gtk4::Widget {
    let scroll = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .build();

    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 24);
    root.set_margin_start(28);
    root.set_margin_end(28);
    root.set_margin_top(20);
    root.set_margin_bottom(32);

    root.append(
        &gtk4::Label::builder()
            .label("Accessibility")
            .halign(gtk4::Align::Start)
            .css_classes(vec!["win11-page-title".to_string()])
            .build(),
    );

    if !kconfig::available() {
        root.append(
            &adw::StatusPage::builder()
                .icon_name("dialog-information-symbolic")
                .title("Accessibility settings unavailable")
                .description("These settings need Plasma's configuration tools (kwriteconfig6).")
                .build(),
        );
    } else {
        root.append(&vision_group());
        root.append(&hearing_group());
    }

    scroll.set_child(Some(&root));
    scroll.upcast()
}
