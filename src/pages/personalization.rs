use gtk4::prelude::*;
use gtk4::gio;
use libadwaita as adw;
use adw::prelude::*;
use std::process::Command;

use crate::theme;

pub fn build() -> gtk4::Widget {
    let current = std::rc::Rc::new(std::cell::RefCell::new(theme::load()));
    let scroll = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .build();

    let root_box = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
    root_box.set_margin_start(28);
    root_box.set_margin_end(28);
    root_box.set_margin_top(20);
    root_box.set_margin_bottom(32);

    // ── Page Title ────────────────────────────────────────────────────────────
    let title_lbl = gtk4::Label::builder()
        .label("Personalization")
        .halign(gtk4::Align::Start)
        .css_classes(vec!["win11-page-title".to_string()])
        .build();
    root_box.append(&title_lbl);

    // ── Hero Preview Banner (Mockup Preview on Left + 6 Themes on Right) ──────
    let hero_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 20);
    hero_box.set_margin_bottom(8);

    // Left: Big Desktop Window Preview Mockup
    let preview_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    preview_box.set_size_request(240, 140);
    preview_box.set_css_classes(&["win11-theme-preview-large"]);
    hero_box.append(&preview_box);

    // Right: 6-Theme Selection Grid
    let themes_container = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    let select_lbl = gtk4::Label::builder()
        .label("Select a theme to apply")
        .halign(gtk4::Align::Start)
        .css_classes(vec!["win11-card-title".to_string()])
        .build();
    themes_container.append(&select_lbl);

    let themes_grid = gtk4::Grid::builder()
        .column_spacing(8)
        .row_spacing(8)
        .column_homogeneous(true)
        .build();

    let theme_presets = [
        ("Zohara Dark Bloom", "#1a162b", "#7c3aed"),
        ("Windows 11 Light", "#e0e7ff", "#0078d4"),
        ("Deep Nebula", "#0a0e17", "#0ea5e9"),
        ("Nordic Night", "#2e3440", "#88c0d0"),
        ("Sunset Amber", "#26130d", "#f97316"),
        ("Emerald Forest", "#062016", "#10b981"),
    ];

    for (i, (name, bg, accent)) in theme_presets.iter().enumerate() {
        let btn = gtk4::Button::builder()
            .css_classes(vec!["win11-theme-thumb".to_string()])
            .tooltip_text(*name)
            .build();
        let thumb = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        thumb.set_size_request(76, 48);

        // Flat preview swatch: solid background with a thin accent-colored
        // bottom stripe, instead of a gradient blend — the thumbnail should
        // preview what the flat theme actually looks like.
        let custom_css = format!(
            "box {{ background: {bg}; border-radius: 8px; border: 2px solid transparent; \
             border-bottom: 4px solid {accent}; }}",
            bg = bg,
            accent = accent,
        );
        let provider = gtk4::CssProvider::new();
        provider.load_from_string(&custom_css);
        thumb.style_context().add_provider(&provider, gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION);

        btn.set_child(Some(&thumb));

        let bg_owned = bg.to_string();
        let accent_owned = accent.to_string();
        let current_clone = current.clone();
        btn.connect_clicked(move |_| {
            let mut cfg = current_clone.borrow_mut();
            cfg.background = bg_owned.clone();
            cfg.accent = accent_owned.clone();
            theme::set_and_apply(&cfg);
        });

        let col = (i % 3) as i32;
        let row = (i / 3) as i32;
        themes_grid.attach(&btn, col, row, 1, 1);
    }
    themes_container.append(&themes_grid);
    hero_box.append(&themes_container);
    root_box.append(&hero_box);

    // ── Grouped Rows (100% In-App Standalone) ─────────────────────────────────
    let rows_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    rows_box.set_css_classes(&["win11-card-group"]);

    // 1. Background (with In-App Wallpapers)
    let bg_exp = adw::ExpanderRow::new();
    bg_exp.set_title("Background");
    bg_exp.set_subtitle("Background image, color, slideshow");
    bg_exp.add_prefix(&gtk4::Image::from_icon_name("preferences-desktop-wallpaper-symbolic"));
    bg_exp.set_css_classes(&["win11-expander-row"]);

    let choose_file_row = adw::ActionRow::new();
    choose_file_row.set_title("Choose a photo");
    let browse_btn = gtk4::Button::builder()
        .label("Browse photos")
        .css_classes(vec!["win11-secondary-btn".to_string()])
        .build();
    let choose_file_clone = choose_file_row.clone();
    browse_btn.connect_clicked(move |btn| {
        let parent = btn.root().and_downcast::<gtk4::Window>();
        let file_dialog = gtk4::FileDialog::new();
        file_dialog.set_title("Select Wallpaper Image");
        let choose_clone = choose_file_clone.clone();
        file_dialog.open(parent.as_ref(), None::<&gio::Cancellable>, move |res| {
            if let Ok(file) = res {
                if let Some(path) = file.path() {
                    let path_str = path.to_string_lossy().to_string();
                    let _ = Command::new("plasma-apply-wallpaperimage").arg(&path_str).spawn();
                    choose_clone.set_subtitle(&format!("Applied: {}", path.file_name().unwrap_or_default().to_string_lossy()));
                }
            }
        });
    });
    choose_file_row.add_suffix(&browse_btn);
    bg_exp.add_row(&choose_file_row);
    rows_box.append(&bg_exp);

    // 2. Colors (with In-App Accent Colors)
    let colors_exp = adw::ExpanderRow::new();
    colors_exp.set_title("Colors");
    colors_exp.set_subtitle("Accent color, transparency effects, color theme");
    colors_exp.add_prefix(&gtk4::Image::from_icon_name("preferences-desktop-color-symbolic"));
    colors_exp.set_css_classes(&["win11-expander-row"]);

    let mode_row = adw::ComboRow::new();
    mode_row.set_title("Choose your mode");
    let mode_list = gtk4::StringList::new(&["Dark (Recommended)", "Light", "Custom"]);
    mode_row.set_model(Some(&mode_list));
    mode_row.set_selected(match current.borrow().mode.as_str() {
        "light" => 1,
        "custom" => 2,
        _ => 0,
    });
    {
        let current_clone = current.clone();
        mode_row.connect_selected_notify(move |row| {
            let mut cfg = current_clone.borrow_mut();
            cfg.mode = match row.selected() {
                1 => "light",
                2 => "custom",
                _ => "dark",
            }
            .to_string();
            theme::set_and_apply(&cfg);
        });
    }
    colors_exp.add_row(&mode_row);

    // Off by default: a translucent window without real compositor
    // blur-behind just looks washed out, not premium. See theme.rs for the
    // KWin blur-behind integration this would need to do properly.
    let trans_row = adw::SwitchRow::new();
    trans_row.set_title("Transparency effects");
    trans_row.set_subtitle("Windows and surfaces appear translucent");
    trans_row.set_active(current.borrow().transparency);
    {
        let current_clone = current.clone();
        trans_row.connect_active_notify(move |row| {
            let mut cfg = current_clone.borrow_mut();
            cfg.transparency = row.is_active();
            theme::set_and_apply(&cfg);
        });
    }
    colors_exp.add_row(&trans_row);

    rows_box.append(&colors_exp);

    // 3. Themes
    let themes_row = build_action_row("Themes", "Install, create, and manage desktop themes", "applications-graphics-symbolic");
    themes_row.connect_activated(open_themes_window);
    rows_box.append(&themes_row);

    // 4. Dynamic Lighting
    let lighting_row = build_action_row("Dynamic Lighting", "Connected RGB devices, effects, app settings", "weather-clear-symbolic");
    lighting_row.connect_activated(open_dynamic_lighting_window);
    rows_box.append(&lighting_row);

    // 5. Lock Screen
    let lock_row = build_action_row("Lock screen", "Lock screen images, apps, timeout status", "system-lock-screen-symbolic");
    lock_row.connect_activated(open_lock_screen_window);
    rows_box.append(&lock_row);

    // 6. Text Input
    let text_row = build_action_row("Text input", "Touch keyboard, voice typing, emoji and more", "input-keyboard-symbolic");
    text_row.connect_activated(open_text_input_window);
    rows_box.append(&text_row);

    // 7. Start
    let start_row = build_action_row("Start", "Recent apps and items, folders, start menu layout", "view-app-grid-symbolic");
    start_row.connect_activated(open_start_window);
    rows_box.append(&start_row);

    // 8. Taskbar (with In-App Alignment Toggle)
    let taskbar_exp = adw::ExpanderRow::new();
    taskbar_exp.set_title("Taskbar");
    taskbar_exp.set_subtitle("Taskbar behaviors, system pins, alignment");
    taskbar_exp.add_prefix(&gtk4::Image::from_icon_name("view-paged-symbolic"));
    taskbar_exp.set_css_classes(&["win11-expander-row"]);

    let align_row = adw::ComboRow::new();
    align_row.set_title("Taskbar alignment");
    let align_list = gtk4::StringList::new(&["Center (Windows 11 style)", "Left (Classic style)"]);
    align_row.set_model(Some(&align_list));
    align_row.set_selected(0);
    taskbar_exp.add_row(&align_row);

    rows_box.append(&taskbar_exp);

    // 9. Fonts
    let fonts_row = build_action_row("Fonts", "Font family, font sizes, ClearType text", "preferences-desktop-font-symbolic");
    fonts_row.connect_activated(open_fonts_window);
    rows_box.append(&fonts_row);

    root_box.append(&rows_box);
    scroll.set_child(Some(&root_box));
    scroll.upcast()
}

fn build_action_row(title: &str, subtitle: &str, icon_name: &str) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(title);
    row.set_subtitle(subtitle);
    row.add_prefix(&gtk4::Image::from_icon_name(icon_name));
    row.add_suffix(&gtk4::Image::from_icon_name("go-next-symbolic"));
    row.set_css_classes(&["win11-expander-row"]);
    row.set_activatable(true);
    row
}

/// Opens `content` as a small modal window over whatever window `row`
/// currently lives in. Used by the six rows below, which used to be pure
/// navigation chevrons with `set_activatable(true)` and no handler at all
/// — clicking them did nothing.
fn open_settings_window(row: &adw::ActionRow, title: &str, content: &gtk4::Box) {
    let Some(parent) = row.root().and_downcast::<gtk4::Window>() else {
        return;
    };
    let win = gtk4::Window::builder()
        .title(title)
        .transient_for(&parent)
        .modal(true)
        .default_width(420)
        .default_height(340)
        .build();
    let scroll = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .build();
    scroll.set_child(Some(content));
    win.set_child(Some(&scroll));
    win.present();
}

fn dialog_content_box() -> gtk4::Box {
    let b = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    b.set_margin_top(16);
    b.set_margin_bottom(16);
    b.set_margin_start(16);
    b.set_margin_end(16);
    b
}

/// Lists installed Plasma look-and-feel packages (`plasma-apply-lookandfeel
/// --list`) with an Apply button each. Real listing/apply, not a mockup.
fn open_themes_window(row: &adw::ActionRow) {
    let content = dialog_content_box();

    let names: Vec<String> = Command::new("plasma-apply-lookandfeel")
        .arg("--list")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_default();

    if names.is_empty() {
        let msg = gtk4::Label::new(Some(
            "plasma-apply-lookandfeel isn't available, or no look-and-feel \
             packages are installed.",
        ));
        msg.set_wrap(true);
        content.append(&msg);
    } else {
        for name in names {
            let theme_row = adw::ActionRow::new();
            theme_row.set_title(&name);
            let apply_btn = gtk4::Button::builder()
                .label("Apply")
                .css_classes(vec!["win11-secondary-btn".to_string()])
                .build();
            let name_clone = name.clone();
            apply_btn.connect_clicked(move |_| {
                let _ = Command::new("plasma-apply-lookandfeel")
                    .args(["-a", &name_clone])
                    .spawn();
            });
            theme_row.add_suffix(&apply_btn);
            content.append(&theme_row);
        }
    }

    open_settings_window(row, "Themes", &content);
}

/// Lists RGB devices via `openrgb --list-devices` with a few preset colors
/// each. Real device control when OpenRGB is installed; an honest message
/// (not a hardcoded fake list) when it isn't.
fn open_dynamic_lighting_window(row: &adw::ActionRow) {
    let content = dialog_content_box();

    let has_openrgb = Command::new("sh")
        .args(["-c", "command -v openrgb"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !has_openrgb {
        let msg = gtk4::Label::new(Some(
            "OpenRGB isn't installed, so no RGB devices can be controlled \
             here. Install the `openrgb` package to enable this.",
        ));
        msg.set_wrap(true);
        content.append(&msg);
    } else {
        let devices: Vec<(usize, String)> = Command::new("openrgb")
            .arg("--list-devices")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .filter_map(|l| {
                        let (idx, name) = l.trim().split_once(':')?;
                        let idx: usize = idx.trim().parse().ok()?;
                        Some((idx, name.trim().to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default();

        if devices.is_empty() {
            content.append(&gtk4::Label::new(Some("No RGB devices detected.")));
        }
        for (idx, name) in devices {
            let dev_row = adw::ActionRow::new();
            dev_row.set_title(&name);
            for (label, hex) in [("Off", "000000"), ("White", "FFFFFF"), ("Red", "FF0000")] {
                let btn = gtk4::Button::with_label(label);
                btn.connect_clicked(move |_| {
                    let _ = Command::new("openrgb")
                        .args(["-d", &idx.to_string(), "-c", hex])
                        .spawn();
                });
                dev_row.add_suffix(&btn);
            }
            content.append(&dev_row);
        }
    }

    open_settings_window(row, "Dynamic Lighting", &content);
}

/// Lock screen wallpaper + auto-lock timeout, written to kscreenlockerrc
/// (KDE's own config file for this, via kwriteconfig6) rather than a
/// separate Zohara-only mechanism — this is what actually controls the
/// lock screen on Zohara OS's KDE Plasma session.
fn open_lock_screen_window(row: &adw::ActionRow) {
    let content = dialog_content_box();

    let wallpaper_row = adw::ActionRow::new();
    wallpaper_row.set_title("Lock screen background");
    let browse_btn = gtk4::Button::builder()
        .label("Choose photo")
        .css_classes(vec!["win11-secondary-btn".to_string()])
        .build();
    let wallpaper_row_clone = wallpaper_row.clone();
    browse_btn.connect_clicked(move |btn| {
        let parent = btn.root().and_downcast::<gtk4::Window>();
        let file_dialog = gtk4::FileDialog::new();
        file_dialog.set_title("Select Lock Screen Wallpaper");
        let wallpaper_row_clone2 = wallpaper_row_clone.clone();
        file_dialog.open(parent.as_ref(), None::<&gio::Cancellable>, move |res| {
            if let Ok(file) = res {
                if let Some(path) = file.path() {
                    let path_str = path.to_string_lossy().to_string();
                    let _ = Command::new("kwriteconfig6")
                        .args([
                            "--file", "kscreenlockerrc",
                            "--group", "Greeter", "--group", "Wallpaper",
                            "--group", "org.kde.image", "--group", "General",
                            "--key", "Image", &path_str,
                        ])
                        .spawn();
                    wallpaper_row_clone2.set_subtitle(&format!(
                        "Set: {}",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    ));
                }
            }
        });
    });
    wallpaper_row.add_suffix(&browse_btn);
    content.append(&wallpaper_row);

    let timeout_row = adw::ComboRow::new();
    timeout_row.set_title("Lock after inactivity");
    let timeout_list = gtk4::StringList::new(&["1 minute", "5 minutes", "10 minutes", "30 minutes", "Never"]);
    timeout_row.set_model(Some(&timeout_list));
    timeout_row.set_selected(1);
    timeout_row.connect_selected_notify(|r| {
        let minutes: Option<u32> = match r.selected() {
            0 => Some(1),
            1 => Some(5),
            2 => Some(10),
            3 => Some(30),
            _ => None, // "Never"
        };
        match minutes {
            Some(m) => {
                let _ = Command::new("kwriteconfig6")
                    .args(["--file", "kscreenlockerrc", "--group", "Daemon", "--key", "Autolock", "true"])
                    .spawn();
                let _ = Command::new("kwriteconfig6")
                    .args(["--file", "kscreenlockerrc", "--group", "Daemon", "--key", "Timeout", &m.to_string()])
                    .spawn();
            }
            None => {
                let _ = Command::new("kwriteconfig6")
                    .args(["--file", "kscreenlockerrc", "--group", "Daemon", "--key", "Autolock", "false"])
                    .spawn();
            }
        }
    });
    content.append(&timeout_row);

    open_settings_window(row, "Lock Screen", &content);
}

/// Launches whichever on-screen keyboard is actually installed
/// (wvkbd/onboard/florence, checked in that order) instead of assuming
/// one exists.
fn open_text_input_window(row: &adw::ActionRow) {
    let content = dialog_content_box();

    let info = gtk4::Label::new(Some(
        "Touch keyboard and voice typing depend on what's installed on this system.",
    ));
    info.set_wrap(true);
    info.set_halign(gtk4::Align::Start);
    content.append(&info);

    let open_btn = gtk4::Button::builder()
        .label("Open on-screen keyboard")
        .css_classes(vec!["win11-secondary-btn".to_string()])
        .build();
    open_btn.connect_clicked(|_| {
        for candidate in ["wvkbd-mobintl", "onboard", "florence"] {
            let found = Command::new("sh")
                .args(["-c", &format!("command -v {candidate}")])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            if found {
                let _ = Command::new(candidate).spawn();
                return;
            }
        }
        log::warn!("No on-screen keyboard (wvkbd/onboard/florence) is installed");
    });
    content.append(&open_btn);

    open_settings_window(row, "Text Input", &content);
}

/// The Start menu is a Plasma panel applet (Kickoff or similar), not
/// something this app owns — its layout/recent-items settings live in the
/// applet's own config, addressed by an instance ID that varies per
/// install. Rather than guess at that and risk corrupting the panel
/// config, this points at the real, correct way to reach those settings.
fn open_start_window(row: &adw::ActionRow) {
    let content = dialog_content_box();
    let info = gtk4::Label::new(Some(
        "Start menu layout, recent items, and pinned folders are configured \
         by right-clicking the Start button on the taskbar and choosing \
         \u{201c}Configure Application Launcher\u{201d} \u{2014} that's the \
         launcher applet's own settings, not something duplicated here.",
    ));
    info.set_wrap(true);
    info.set_halign(gtk4::Align::Start);
    content.append(&info);
    open_settings_window(row, "Start", &content);
}

/// Sets the GTK font (so this app and every other GTK4 app picks it up
/// immediately via gtk-4.0/settings.ini) and best-effort sets the Qt/Plasma
/// font too (kdeglobals), so both toolkits agree.
fn open_fonts_window(row: &adw::ActionRow) {
    let content = dialog_content_box();

    const FAMILIES: &[&str] = &["Segoe UI Variable", "Cantarell", "Noto Sans", "Inter", "Ubuntu"];
    const SIZES: &[&str] = &["9", "10", "11", "12", "14"];

    let family_row = adw::ComboRow::new();
    family_row.set_title("System font");
    family_row.set_model(Some(&gtk4::StringList::new(FAMILIES)));
    family_row.set_selected(0);
    content.append(&family_row);

    let size_row = adw::ComboRow::new();
    size_row.set_title("Font size");
    size_row.set_model(Some(&gtk4::StringList::new(SIZES)));
    size_row.set_selected(2);
    content.append(&size_row);

    let apply_btn = gtk4::Button::builder()
        .label("Apply")
        .halign(gtk4::Align::End)
        .css_classes(vec!["win11-primary-btn".to_string()])
        .build();
    let family_row_clone = family_row.clone();
    let size_row_clone = size_row.clone();
    apply_btn.connect_clicked(move |_| {
        let family = FAMILIES
            .get(family_row_clone.selected() as usize)
            .copied()
            .unwrap_or("Cantarell");
        let size = SIZES.get(size_row_clone.selected() as usize).copied().unwrap_or("11");

        let gtk_ini_path = std::env::var("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
                std::path::PathBuf::from(home).join(".config")
            })
            .join("gtk-4.0")
            .join("settings.ini");
        if let Some(parent_dir) = gtk_ini_path.parent() {
            let _ = std::fs::create_dir_all(parent_dir);
        }
        let existing = std::fs::read_to_string(&gtk_ini_path).unwrap_or_else(|_| "[Settings]".to_string());
        let mut lines: Vec<String> = existing
            .lines()
            .filter(|l| !l.trim_start().starts_with("gtk-font-name"))
            .map(|l| l.to_string())
            .collect();
        if !lines.iter().any(|l| l.trim() == "[Settings]") {
            lines.insert(0, "[Settings]".to_string());
        }
        lines.push(format!("gtk-font-name={family} {size}"));
        let _ = std::fs::write(&gtk_ini_path, lines.join("\n") + "\n");

        // Best-effort: Qt/Plasma apps read kdeglobals, not gtk-4.0/settings.ini.
        let _ = Command::new("kwriteconfig6")
            .args([
                "--file", "kdeglobals", "--group", "General", "--key", "font",
                &format!("{family},{size},-1,5,50,0,0,0,0,0"),
            ])
            .spawn();
    });
    content.append(&apply_btn);

    open_settings_window(row, "Fonts", &content);
}
