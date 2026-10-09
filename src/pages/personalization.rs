use gtk4::prelude::*;
use gtk4::gio;
use libadwaita as adw;
use crate::backend::kconfig;
use crate::backend::desktop_style;
use crate::backend::effects;
use crate::backend::looks;
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
    let mode_list = gtk4::StringList::new(&["Match the system (Recommended)", "Dark", "Light", "Custom"]);
    mode_row.set_model(Some(&mode_list));
    mode_row.set_selected(match current.borrow().mode.as_str() {
        "dark" => 1,
        "light" => 2,
        "custom" => 3,
        _ => 0,
    });
    {
        let current_clone = current.clone();
        mode_row.connect_selected_notify(move |row| {
            let mut cfg = current_clone.borrow_mut();
            cfg.mode = match row.selected() {
                1 => "dark",
                2 => "light",
                3 => "custom",
                _ => "system",
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
    themes_row.connect_activated(super::themes::open);
    rows_box.append(&super::in_list(&themes_row));

    // 4. Dynamic Lighting
    let lighting_row = build_action_row("Dynamic Lighting", "Connected RGB devices, effects, app settings", "weather-clear-symbolic");
    lighting_row.connect_activated(super::lighting::open);
    rows_box.append(&super::in_list(&lighting_row));

    // 5. Lock Screen
    let lock_row = build_action_row("Lock screen", "Lock screen images, apps, timeout status", "system-lock-screen-symbolic");
    lock_row.connect_activated(super::lockscreen::open);
    rows_box.append(&super::in_list(&lock_row));

    // 6. Text Input
    let text_row = build_action_row("Text input", "Touch keyboard, voice typing, emoji and more", "input-keyboard-symbolic");
    text_row.connect_activated(super::text_input::open);
    rows_box.append(&super::in_list(&text_row));

    // 7. Start
    let start_row = build_action_row("Start", "Recent apps and items, folders, start menu layout", "view-app-grid-symbolic");
    start_row.connect_activated(super::start_menu::open);
    rows_box.append(&super::in_list(&start_row));

    // 8. Taskbar (with In-App Alignment Toggle)
    let taskbar_exp = adw::ExpanderRow::new();
    taskbar_exp.set_title("Taskbar");
    taskbar_exp.set_subtitle("Taskbar behaviors, system pins, alignment");
    taskbar_exp.add_prefix(&gtk4::Image::from_icon_name("view-paged-symbolic"));
    taskbar_exp.set_css_classes(&["win11-expander-row"]);

    let (cur_align, cur_pos) = taskbar_state();
    let pos_row = adw::ComboRow::new();
    pos_row.set_title("Taskbar position");
    pos_row.set_subtitle("Which edge of the screen the taskbar sits on");
    pos_row.set_model(Some(&gtk4::StringList::new(&POSITIONS.map(|p| p.0))));
    pos_row.set_selected(cur_pos);

    let align_row = adw::ComboRow::new();
    align_row.set_title("Taskbar alignment");
    align_row.set_subtitle("Where the icons sit on the taskbar");
    let labels_for = |pos: u32| if pos >= 2 { ALIGN_LABELS_VERTICAL } else { ALIGN_LABELS_HORIZONTAL };
    align_row.set_model(Some(&gtk4::StringList::new(&labels_for(cur_pos))));
    align_row.set_selected(cur_align);

    let status = adw::ActionRow::new();
    status.set_title("Changes apply right away");
    status.set_activatable(false);
    {
        let ready = std::rc::Rc::new(std::cell::Cell::new(false));
        let apply = {
            let (pos_row, align_row, status, ready) = (pos_row.clone(), align_row.clone(), status.clone(), ready.clone());
            std::rc::Rc::new(move |move_bar: bool| {
                if !ready.get() {
                    return;
                }
                let align = align_row.selected().min(2);
                let location = POSITIONS[(pos_row.selected() as usize).min(POSITIONS.len() - 1)].1;
                let status = status.clone();
                status.set_title("Applying…");
                crate::backend::worker::in_background(
                    move || run_taskbar_script(align, move_bar.then_some(location)),
                    move |ok| {
                        status.set_title(if ok { "Saved" } else { "Taskbar not changed" });
                        status.set_subtitle(if ok { "" } else { "Plasma didn't accept the change. The taskbar can only be changed from inside a Zohara desktop session." });
                    },
                );
            })
        };
        {
            let (align_row, apply) = (align_row.clone(), apply.clone());
            pos_row.connect_selected_notify(move |r| {
                // A side bar lines icons up top to bottom, so the names change with it.
                let keep = align_row.selected();
                align_row.set_model(Some(&gtk4::StringList::new(&labels_for(r.selected()))));
                align_row.set_selected(keep);
                apply(true);
            });
        }
        let apply2 = apply.clone();
        align_row.connect_selected_notify(move |_| apply2(false));
        ready.set(true);
    }
    taskbar_exp.add_row(&pos_row);
    taskbar_exp.add_row(&align_row);
    taskbar_exp.add_row(&status);
    for r in bar_style_rows() {
        taskbar_exp.add_row(&r);
    }

    rows_box.append(&taskbar_exp);

    // 8a. Quick looks: a whole feel in one click
    let looks_exp = adw::ExpanderRow::new();
    looks_exp.set_title("Quick looks");
    looks_exp.set_subtitle("Window buttons, taskbar and effects set together");
    looks_exp.add_prefix(&gtk4::Image::from_icon_name("preferences-desktop-theme-symbolic"));
    looks_exp.set_css_classes(&["win11-expander-row"]);
    let looks_status = adw::ActionRow::new();
    looks_status.set_title("Pick one to try it");
    looks_status.set_subtitle("Your colors, icons and wallpaper stay as they are. You can still change every part below.");
    looks_status.set_activatable(false);
    for (i, preset) in looks::PRESETS.iter().enumerate() {
        let row = adw::ActionRow::new();
        row.set_title(preset.name);
        row.set_subtitle(preset.about);
        let btn = gtk4::Button::with_label("Use");
        btn.set_valign(gtk4::Align::Center);
        let status = looks_status.clone();
        btn.connect_clicked(move |b| {
            b.set_sensitive(false);
            status.set_title("Applying…");
            let (b, status) = (b.clone(), status.clone());
            crate::backend::worker::in_background(move || looks::apply(&(looks::PRESETS[i].look)()), move |ok| {
                b.set_sensitive(true);
                status.set_title(if ok { "Done" } else { "Some parts were not accepted" });
                status.set_subtitle("Open Personalization again to see every switch below in its new place.");
            });
        });
        row.add_suffix(&btn);
        looks_exp.add_row(&row);
    }
    looks_exp.add_row(&looks_status);
    rows_box.append(&looks_exp);

    // 8b. Window buttons (close / minimize / maximize on the left like a Mac, or on the right)
    let buttons_row = adw::ComboRow::new();
    buttons_row.set_title("Window buttons");
    buttons_row.set_subtitle("Which side of a window the close, minimize and maximize buttons are on");
    buttons_row.add_prefix(&gtk4::Image::from_icon_name("window-close-symbolic"));
    buttons_row.set_model(Some(&gtk4::StringList::new(&["Right (like Windows)", "Left (like a Mac)"])));
    buttons_row.set_selected(match desktop_style::button_side() {
        desktop_style::ButtonSide::Right => 0,
        desktop_style::ButtonSide::Left => 1,
    });
    {
        let ready = std::rc::Rc::new(std::cell::Cell::new(false));
        let r2 = ready.clone();
        buttons_row.connect_selected_notify(move |r| {
            if !r2.get() {
                return;
            }
            let side = if r.selected() == 1 { desktop_style::ButtonSide::Left } else { desktop_style::ButtonSide::Right };
            crate::backend::worker::in_background(move || desktop_style::set_button_side(side), |_| {});
        });
        ready.set(true);
    }
    rows_box.append(&super::in_list(&buttons_row));

    // 8c. Desktop effects (jelly windows, minimize style, open/close style, dimming, mouse finders)
    let fx_exp = adw::ExpanderRow::new();
    fx_exp.set_title("Desktop effects");
    fx_exp.set_subtitle("Jelly windows, minimize and open animations, blur, dimming");
    fx_exp.add_prefix(&gtk4::Image::from_icon_name("preferences-desktop-effects-symbolic"));
    fx_exp.set_css_classes(&["win11-expander-row"]);
    for r in effect_rows() {
        fx_exp.add_row(&r);
    }
    rows_box.append(&fx_exp);

    // 9. Fonts
    let fonts_exp = adw::ExpanderRow::new();
    fonts_exp.set_title("Fonts");
    fonts_exp.set_subtitle("System font, font size, fixed-width font");
    fonts_exp.add_prefix(&gtk4::Image::from_icon_name("preferences-desktop-font-symbolic"));
    fonts_exp.set_css_classes(&["win11-expander-row"]);
    for r in font_rows() {
        fonts_exp.add_row(&r);
    }
    rows_box.append(&fonts_exp);

    root_box.append(&rows_box);
    scroll.set_child(Some(&root_box));
    scroll.upcast()
}

/// Where the taskbar sits on the screen: label and the name Plasma's scripting uses.
pub const POSITIONS: [(&str, &str); 4] = [("Bottom", "bottom"), ("Top", "top"), ("Left side", "left"), ("Right side", "right")];

/// How the icons are lined up on the bar: 0 start (left, or top on a side bar), 1 centre, 2 end (right, or bottom).
pub const ALIGN_LABELS_HORIZONTAL: [&str; 3] = ["Left", "Center", "Right"];
pub const ALIGN_LABELS_VERTICAL: [&str; 3] = ["Top", "Center", "Bottom"];

/// Plasma script that lines up the taskbar icons and, when `location` is given, moves the bar. Plasma has no "align
/// the icons" switch: the start button and task manager are pushed by flexible spacers (none for the start, one before
/// them for the end, one on each side for the centre). Only the panel that holds the task manager is touched.
fn taskbar_script(align: u32, location: Option<&str>) -> String {
    let loc = location.map(|l| format!("\"{l}\"")).unwrap_or_else(|| "null".to_string());
    format!(
        r#"var ALIGN = {align}; var LOC = {loc};
panels().forEach(function (p) {{
  var ws = p.widgets(), first = -1, last = -1;
  for (var i = 0; i < ws.length; i++) {{
    var t = ws[i].type;
    if (t.indexOf("org.kde.plasma.kickoff") == 0 || t.indexOf("org.kde.plasma.kicker") == 0 ||
        t == "org.kde.plasma.icontasks" || t == "org.kde.plasma.taskmanager") {{ if (first < 0) first = i; last = i; }}
  }}
  if (first < 0) return;
  if (LOC) p.location = LOC;
  p.widgets().forEach(function (w) {{ if (w.type == "org.kde.plasma.panelspacer") w.remove(); }});
  if (ALIGN == 0) return;
  ws = p.widgets(); first = -1; last = -1;
  for (var j = 0; j < ws.length; j++) {{
    var u = ws[j].type;
    if (u.indexOf("org.kde.plasma.kickoff") == 0 || u.indexOf("org.kde.plasma.kicker") == 0 ||
        u == "org.kde.plasma.icontasks" || u == "org.kde.plasma.taskmanager") {{ if (first < 0) first = j; last = j; }}
  }}
  var a = p.addWidget("org.kde.plasma.panelspacer"); a.index = first;
  if (ALIGN == 1) {{ var b = p.addWidget("org.kde.plasma.panelspacer"); b.index = last + 2; }}
}});"#
    )
}

pub(super) fn run_taskbar_script(align: u32, location: Option<&str>) -> bool {
    Command::new("dbus-send")
        .args(["--session", "--type=method_call", "--dest=org.kde.plasmashell", "/PlasmaShell", "org.kde.PlasmaShell.evaluateScript"])
        .arg(format!("string:{}", taskbar_script(align, location)))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// What the saved panel layout says: (alignment 0/1/2, position index into `POSITIONS`).
/// Spacers: none is the start, one the end, two the centre. The position is the panel's `location=` number.
pub fn parse_taskbar_state(appletsrc: &str) -> (u32, u32) {
    let spacers = appletsrc.lines().filter(|l| l.trim() == "plugin=org.kde.plasma.panelspacer").count();
    let align = match spacers {
        0 => 0,
        1 => 2,
        _ => 1,
    };
    let (mut in_top, mut location, mut found) = (false, None::<u32>, None::<u32>);
    for l in appletsrc.lines().map(str::trim).chain(std::iter::once("[end]")) {
        if l.starts_with('[') {
            in_top = l.matches('[').count() == 2 && l.starts_with("[Containments]");
            location = None;
            continue;
        }
        if !in_top {
            continue;
        }
        if let Some(v) = l.strip_prefix("location=") {
            location = v.parse().ok();
        } else if l == "plugin=org.kde.panel" && found.is_none() {
            // `location=` comes before `plugin=` in the file, so it is already known here.
            found = location;
        }
    }
    let pos = match found {
        Some(3) => 1,
        Some(5) => 2,
        Some(6) => 3,
        _ => 0,
    };
    (align, pos)
}

fn taskbar_state() -> (u32, u32) {
    let home = std::env::var("HOME").unwrap_or_default();
    std::fs::read_to_string(format!("{home}/.config/plasma-org.kde.plasma.desktop-appletsrc"))
        .map(|t| parse_taskbar_state(&t))
        .unwrap_or((1, 0)) // the Zohara default look: centred, at the bottom
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
    open_settings_window_sized(row, title, content, 560, 600);
}

/// Same, with a size. Rows put straight into `content` would ignore clicks (they need a list around them), so any
/// that are there get one first.
pub(super) fn open_settings_window_sized(row: &adw::ActionRow, title: &str, content: &gtk4::Box, width: i32, height: i32) {
    let Some(parent) = row.root().and_downcast::<gtk4::Window>() else {
        return;
    };
    super::adopt_orphan_rows(content.upcast_ref());
    let win = gtk4::Window::builder()
        .title(title)
        .transient_for(&parent)
        .modal(true)
        .default_width(width)
        .default_height(height)
        .build();
    let scroll = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .build();
    scroll.set_child(Some(content));
    win.set_child(Some(&scroll));
    win.present();
}

pub(super) fn dialog_content_box() -> gtk4::Box {
    let b = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    b.set_margin_top(16);
    b.set_margin_bottom(16);
    b.set_margin_start(16);
    b.set_margin_end(16);
    b
}

/// Every font family installed (`fc-list`), each once, sorted, with `current` first when it is not in the list.
fn all_font_families(current: &str) -> Vec<String> {
    let out = Command::new("fc-list").args([":", "family"]).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
    parse_families(&out, current)
}

pub(crate) fn parse_families(fc_list: &str, current: &str) -> Vec<String> {
    let mut v: Vec<String> = fc_list
        .lines()
        // "Noto Sans,Noto Sans Bold" lists the family and its style names: the first is the family.
        .filter_map(|l| l.split(',').next())
        .map(|f| f.trim().to_string())
        .filter(|f| !f.is_empty() && !f.starts_with('.'))
        .collect();
    v.sort_by_key(|f| f.to_lowercase());
    v.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    if !current.is_empty() && !v.iter().any(|f| f.eq_ignore_ascii_case(current)) {
        v.insert(0, current.to_string());
    }
    v
}

/// "Noto Sans,10,-1,5,400,..." (KDE's font string) -> ("Noto Sans", 10).
fn parse_qt_font(v: &str) -> Option<(String, u32)> {
    let mut it = v.split(',');
    let family = it.next()?.trim().to_string();
    let size = it.next()?.trim().parse::<f32>().ok()?.round() as u32;
    (!family.is_empty() && size > 0).then_some((family, size))
}

fn qt_font_value(family: &str, size: u32, fixed: bool) -> String {
    // The ninth field marks a fixed-width font for Qt.
    if fixed {
        format!("{family},{size},-1,5,400,0,0,0,1,0,0,0,0,0,0,1")
    } else {
        format!("{family},{size},-1,5,400,0,0,0,0,0,0,0,0,0,0,1")
    }
}

fn gtk_config_dir() -> std::path::PathBuf {
    std::env::var("XDG_CONFIG_HOME").map(std::path::PathBuf::from).unwrap_or_else(|_| {
        std::path::PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string())).join(".config")
    })
}

/// Sets `key=value` in the `[Settings]` part of the GTK 3 and 4 settings files.
fn set_gtk_ini(key: &str, value: &str) {
    let base = gtk_config_dir();
    for dir in ["gtk-4.0", "gtk-3.0"] {
        let path = base.join(dir).join("settings.ini");
        let _ = std::fs::create_dir_all(path.parent().unwrap_or(&base));
        let existing = std::fs::read_to_string(&path).unwrap_or_else(|_| "[Settings]".to_string());
        let mut lines: Vec<String> = existing.lines().filter(|l| !l.trim_start().starts_with(&format!("{key}="))).map(str::to_string).collect();
        if !lines.iter().any(|l| l.trim() == "[Settings]") {
            lines.insert(0, "[Settings]".to_string());
        }
        lines.push(format!("{key}={value}"));
        let _ = std::fs::write(&path, lines.join("\n") + "\n");
    }
}

/// Writes the system font where every kind of app looks: GTK 3 and 4 settings files, the GNOME setting that GTK apps
/// read on Wayland, and Plasma's `kdeglobals`. Returns false when the Plasma config tool is missing.
fn apply_font(family: &str, size: u32) -> bool {
    set_gtk_ini("gtk-font-name", &format!("{family} {size}"));
    let _ = Command::new("gsettings").args(["set", "org.gnome.desktop.interface", "font-name", &format!("{family} {size}")]).status();
    if !kconfig::available() {
        return false;
    }
    let v = qt_font_value(family, size, false);
    for key in ["font", "menuFont", "toolBarFont"] {
        kconfig::write("kdeglobals", &["General"], key, &v);
    }
    notify_fonts_changed();
    true
}

/// The fixed-width font (terminals, code).
fn apply_fixed_font(family: &str, size: u32) -> bool {
    let _ = Command::new("gsettings").args(["set", "org.gnome.desktop.interface", "monospace-font-name", &format!("{family} {size}")]).status();
    if !kconfig::available() {
        return false;
    }
    kconfig::write("kdeglobals", &["General"], "fixed", &qt_font_value(family, size, true));
    notify_fonts_changed();
    true
}

/// Tell running Qt apps the fonts changed (KDE's own settings-changed signal, "FontChanged").
fn notify_fonts_changed() {
    let _ = Command::new("dbus-send")
        .args(["--session", "--type=signal", "/KGlobalSettings", "org.kde.KGlobalSettings.notifyChange", "int32:2", "int32:0"])
        .status();
}

const FONT_SIZES: [u32; 11] = [8, 9, 10, 11, 12, 13, 14, 16, 18, 20, 24];

fn size_list(cur: u32) -> Vec<u32> {
    let mut sizes: Vec<u32> = FONT_SIZES.to_vec();
    if !sizes.contains(&cur) {
        sizes.push(cur);
        sizes.sort_unstable();
    }
    sizes
}

/// A searchable family dropdown: typing in it narrows the list.
fn family_row(title: &str, subtitle: &str, families: &[String], current: &str) -> adw::ComboRow {
    let row = adw::ComboRow::new();
    row.set_title(title);
    row.set_subtitle(subtitle);
    row.set_model(Some(&gtk4::StringList::new(&families.iter().map(String::as_str).collect::<Vec<_>>())));
    row.set_expression(Some(gtk4::PropertyExpression::new(gtk4::StringObject::static_type(), None::<gtk4::Expression>, "string")));
    row.set_enable_search(true);
    row.set_selected(families.iter().position(|f| f.eq_ignore_ascii_case(current)).unwrap_or(0) as u32);
    row
}

fn size_row(title: &str, sizes: &[u32], cur: u32) -> adw::ComboRow {
    let row = adw::ComboRow::new();
    row.set_title(title);
    row.set_model(Some(&gtk4::StringList::new(&sizes.iter().map(|s| s.to_string()).collect::<Vec<_>>().iter().map(String::as_str).collect::<Vec<_>>())));
    row.set_selected(sizes.iter().position(|s| *s == cur).unwrap_or(0) as u32);
    row
}

/// The font rows: system font, its size, fixed-width font and size, and a line saying what happened.
/// Changes apply at once; this window's own app changes first so the result is visible immediately.
fn font_rows() -> Vec<gtk4::Widget> {
    let (cur_family, cur_size) = kconfig::read("kdeglobals", &["General"], "font").and_then(|v| parse_qt_font(&v)).unwrap_or_else(|| ("Noto Sans".to_string(), 10));
    let (cur_fixed, cur_fixed_size) = kconfig::read("kdeglobals", &["General"], "fixed").and_then(|v| parse_qt_font(&v)).unwrap_or_else(|| ("Monospace".to_string(), 10));
    let mut families = all_font_families(&cur_family);
    for extra in [&cur_fixed] {
        if !families.iter().any(|f| f.eq_ignore_ascii_case(extra)) {
            families.insert(0, extra.clone());
        }
    }
    let sizes = size_list(cur_size);
    let fixed_sizes = size_list(cur_fixed_size);

    let fam = family_row("System font", "The font of menus, buttons and windows. Type to search.", &families, &cur_family);
    let sz = size_row("Font size", &sizes, cur_size);
    let fixed = family_row("Fixed-width font", "Used in terminals and code. Type to search.", &families, &cur_fixed);
    let fixed_sz = size_row("Fixed-width font size", &fixed_sizes, cur_fixed_size);

    let status = adw::ActionRow::new();
    status.set_title("Changes apply right away");
    status.set_subtitle("Open apps pick them up when they are reopened.");
    status.set_activatable(false);

    let ready = std::rc::Rc::new(std::cell::Cell::new(false));
    let apply_system = {
        let (fam, sz, status, ready) = (fam.clone(), sz.clone(), status.clone(), ready.clone());
        let (families, sizes) = (families.clone(), sizes.clone());
        move || {
            if !ready.get() {
                return;
            }
            let family = families.get(fam.selected() as usize).cloned().unwrap_or_else(|| "Noto Sans".to_string());
            let size = sizes.get(sz.selected() as usize).copied().unwrap_or(10);
            if let Some(settings) = gtk4::Settings::default() {
                settings.set_gtk_font_name(Some(&format!("{family} {size}")));
            }
            status.set_title("Applying…");
            let status = status.clone();
            let label = format!("{family} {size}");
            crate::backend::worker::in_background(move || apply_font(&family, size), move |ok| {
                status.set_title(&if ok { format!("Applied: {label}") } else { "Applied for GTK apps only".to_string() });
                status.set_subtitle(if ok { "Open apps pick it up when they are reopened." } else { "Plasma's settings tool is missing, so KDE apps keep their font." });
            });
        }
    };
    let apply_fixed = {
        let (fixed, fixed_sz, status, ready) = (fixed.clone(), fixed_sz.clone(), status.clone(), ready.clone());
        let (families, sizes) = (families.clone(), fixed_sizes.clone());
        move || {
            if !ready.get() {
                return;
            }
            let family = families.get(fixed.selected() as usize).cloned().unwrap_or_else(|| "Monospace".to_string());
            let size = sizes.get(fixed_sz.selected() as usize).copied().unwrap_or(10);
            status.set_title("Applying…");
            let status = status.clone();
            let label = format!("{family} {size}");
            crate::backend::worker::in_background(move || apply_fixed_font(&family, size), move |ok| {
                status.set_title(&if ok { format!("Applied fixed-width font: {label}") } else { "Applied for GTK apps only".to_string() });
                status.set_subtitle(if ok { "Terminals pick it up when they are reopened." } else { "Plasma's settings tool is missing, so KDE apps keep their font." });
            });
        }
    };
    {
        let a = apply_system.clone();
        fam.connect_selected_notify(move |_| a());
        let a = apply_system.clone();
        sz.connect_selected_notify(move |_| a());
        let a = apply_fixed.clone();
        fixed.connect_selected_notify(move |_| a());
        let a = apply_fixed.clone();
        fixed_sz.connect_selected_notify(move |_| a());
    }
    ready.set(true);
    vec![fam.upcast(), sz.upcast(), fixed.upcast(), fixed_sz.upcast(), status.upcast()]
}

/// The Fonts window (also opened from Accessibility > Text size).
pub(crate) fn open_fonts_window(row: &adw::ActionRow) {
    let content = dialog_content_box();
    let group = adw::PreferencesGroup::new();
    for r in font_rows() {
        group.add(&r);
    }
    content.append(&group);
    open_settings_window_sized(row, "Fonts", &content, 560, 560);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qt_font_strings_roundtrip() {
        assert_eq!(parse_qt_font("Noto Sans,10,-1,5,400,0,0,0,0,0,0,0,0,0,0,1"), Some(("Noto Sans".into(), 10)));
        assert_eq!(parse_qt_font("Inter,11.5,-1,5,50,0"), Some(("Inter".into(), 12)));
        assert_eq!(parse_qt_font(""), None);
        assert_eq!(parse_qt_font("Inter,big"), None);
        assert!(qt_font_value("Inter", 12, false).starts_with("Inter,12,"));
        assert_eq!(qt_font_value("Mono", 10, true).split(',').nth(8), Some("1"));
    }

    #[test]
    fn taskbar_script_adds_spacers_by_alignment() {
        assert!(taskbar_script(1, None).contains("var ALIGN = 1; var LOC = null;"));
        assert!(taskbar_script(2, Some("top")).contains("var LOC = \"top\";"));
        assert!(taskbar_script(0, None).contains("panelspacer"));
    }

    #[test]
    fn taskbar_state_is_read_from_the_panel_layout() {
        let centred = "[Containments][2]\nlocation=4\nplugin=org.kde.panel\n\n[Containments][2][Applets][3]\nplugin=org.kde.plasma.panelspacer\n\n[Containments][2][Applets][4]\nplugin=org.kde.plasma.panelspacer\n";
        assert_eq!(parse_taskbar_state(centred), (1, 0));
        let top_right = "[Containments][1]\nlocation=0\nplugin=org.kde.desktopcontainment\n\n[Containments][2]\nlocation=3\nplugin=org.kde.panel\n\n[Containments][2][Applets][3]\nplugin=org.kde.plasma.panelspacer\n";
        assert_eq!(parse_taskbar_state(top_right), (2, 1));
        assert_eq!(parse_taskbar_state("[Containments][2]\nlocation=5\nplugin=org.kde.panel\n"), (0, 2));
    }

    #[test]
    fn families_are_listed_once_and_sorted() {
        let v = parse_families("Noto Sans,Noto Sans Bold\nInter\nnoto sans\n.hidden\nDejaVu Sans\n", "Segoe UI");
        assert_eq!(v, ["Segoe UI", "DejaVu Sans", "Inter", "Noto Sans"]);
        assert_eq!(parse_families("Inter\n", "inter"), ["Inter"]);
    }
}

/// Taskbar look: when it hides, floating, see-through, thickness. Each change is sent to the running shell at once.
fn bar_style_rows() -> Vec<gtk4::Widget> {
    use std::{cell::{Cell, RefCell}, rc::Rc};
    let start = desktop_style::bar();
    let state = Rc::new(RefCell::new(start));
    let ready = Rc::new(Cell::new(false));
    let push = {
        let (state, ready) = (state.clone(), ready.clone());
        Rc::new(move || {
            if !ready.get() {
                return;
            }
            let bar = *state.borrow();
            crate::backend::worker::in_background(move || desktop_style::apply_bar(bar), |_| {});
        })
    };

    let hide = adw::ComboRow::new();
    hide.set_title("Hide the taskbar");
    hide.set_subtitle("Slides away and comes back when the pointer reaches the screen edge");
    hide.set_model(Some(&gtk4::StringList::new(&desktop_style::HIDING.map(|h| h.0))));
    hide.set_selected(desktop_style::HIDING.iter().position(|h| h.1 == start.hiding).unwrap_or(0) as u32);
    {
        let (state, push) = (state.clone(), push.clone());
        hide.connect_selected_notify(move |r| {
            state.borrow_mut().hiding = desktop_style::HIDING[(r.selected() as usize).min(2)].1;
            push();
        });
    }

    let floating = adw::SwitchRow::new();
    floating.set_title("Floating taskbar");
    floating.set_subtitle("A gap around the taskbar, like a dock");
    floating.set_active(start.floating);
    {
        let (state, push) = (state.clone(), push.clone());
        floating.connect_active_notify(move |r| {
            state.borrow_mut().floating = r.is_active();
            push();
        });
    }

    let look = adw::ComboRow::new();
    look.set_title("Taskbar background");
    look.set_subtitle("Solid, see-through, or decided by Plasma");
    look.set_model(Some(&gtk4::StringList::new(&desktop_style::OPACITY.map(|o| o.0))));
    look.set_selected(desktop_style::OPACITY.iter().position(|o| o.1 == start.opacity).unwrap_or(0) as u32);
    {
        let (state, push) = (state.clone(), push.clone());
        look.connect_selected_notify(move |r| {
            state.borrow_mut().opacity = desktop_style::OPACITY[(r.selected() as usize).min(2)].1;
            push();
        });
    }

    let size = adw::SpinRow::with_range(desktop_style::MIN_THICKNESS as f64, desktop_style::MAX_THICKNESS as f64, 2.0);
    size.set_title("Taskbar size");
    size.set_subtitle("Height in pixels (width on a side bar)");
    size.set_value(start.thickness as f64);
    {
        // A spin row fires on every step: wait until the typing stops before touching the shell.
        let (state, push) = (state.clone(), push.clone());
        let pending: Rc<RefCell<Option<gtk4::glib::SourceId>>> = Rc::new(RefCell::new(None));
        size.connect_value_notify(move |r| {
            state.borrow_mut().thickness = r.value() as u32;
            if let Some(id) = pending.borrow_mut().take() {
                id.remove();
            }
            let (push, pending2) = (push.clone(), pending.clone());
            let id = gtk4::glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
                pending2.borrow_mut().take();
                push();
            });
            *pending.borrow_mut() = Some(id);
        });
    }

    ready.set(true);
    vec![hide.upcast(), floating.upcast(), look.upcast(), size.upcast()]
}

/// One row per effect or choice. The running state is read from KWin when the page is built (off the UI thread), and a
/// change is sent at once. A row that cannot reach KWin says so instead of pretending.
fn effect_rows() -> Vec<gtk4::Widget> {
    use std::{cell::Cell, rc::Rc};
    let mut rows: Vec<gtk4::Widget> = Vec::new();
    let note = adw::ActionRow::new();
    note.set_title("Changes apply right away");
    note.set_activatable(false);

    let mut switches: Vec<(&'static str, adw::SwitchRow)> = Vec::new();
    for t in &effects::TOGGLES {
        let row = adw::SwitchRow::new();
        row.set_title(t.title);
        row.set_subtitle(t.subtitle);
        if !effects::available(t) {
            // not installed: say where to get it instead of a switch that does nothing
            row.set_sensitive(false);
            row.set_subtitle("Not installed yet. Zohara Update > New for your computer offers it (Desktop cube and extra widgets).");
            switches.push((t.id, row.clone()));
            rows.push(row.upcast());
            continue;
        }
        let (id, row2, note2) = (t.id, row.clone(), note.clone());
        let ready = Rc::new(Cell::new(false));
        let r = ready.clone();
        row.connect_active_notify(move |sw| {
            if !r.get() {
                return;
            }
            let (on, note3) = (sw.is_active(), note2.clone());
            crate::backend::worker::in_background(move || effects::set(id, on), move |ok| {
                note3.set_title(if ok { "Saved" } else { "KWin did not accept the change" });
            });
        });
        // the real state comes from KWin
        let ready2 = ready.clone();
        crate::backend::worker::in_background(move || effects::is_on(id), move |on| {
            row2.set_active(on);
            ready2.set(true);
        });
        switches.push((t.id, row.clone()));
        rows.push(row.upcast());
    }

    // jelly strength follows the jelly switch
    let strength = adw::ComboRow::new();
    strength.set_title("Jelly strength");
    strength.set_subtitle("How much windows wobble");
    strength.set_model(Some(&gtk4::StringList::new(&effects::JELLY.map(|j| j.0))));
    strength.set_selected(effects::jelly_index());
    if let Some((_, jelly)) = switches.iter().find(|(id, _)| *id == "wobblywindows") {
        jelly.bind_property("active", &strength, "sensitive").sync_create().build();
    }
    strength.connect_selected_notify(|r| {
        let i = r.selected();
        crate::backend::worker::in_background(move || effects::set_jelly(i), |_| {});
    });
    rows.insert(1, strength.upcast());

    for (n, g) in effects::GROUPS.iter().enumerate() {
        let row = adw::ComboRow::new();
        row.set_title(g.title);
        row.set_subtitle(g.subtitle);
        row.set_model(Some(&gtk4::StringList::new(&g.options.iter().map(|o| o.0).collect::<Vec<_>>())));
        let ready = Rc::new(Cell::new(false));
        let (r, row2, ready2) = (ready.clone(), row.clone(), ready.clone());
        row.connect_selected_notify(move |c| {
            if !r.get() {
                return;
            }
            let id = effects::GROUPS[n].options[(c.selected() as usize).min(effects::GROUPS[n].options.len() - 1)].1;
            crate::backend::worker::in_background(move || effects::choose(&effects::GROUPS[n], id), |_| {});
        });
        crate::backend::worker::in_background(move || effects::chosen_index(&effects::GROUPS[n], effects::is_on), move |i| {
            row2.set_selected(i);
            ready2.set(true);
        });
        rows.push(row.upcast());
    }
    rows.push(note.upcast());
    rows
}
