//! Accessibility settings backed by Plasma: high-contrast colour scheme,
//! cursor size, reduced animations, text size (via the Fonts dialog), and
//! visual bell, speech and voice typing.

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

// ── Speech ─────────────────────────────────────────────────────────────────

fn speechd_user_conf() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    let base = std::env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| format!("{home}/.config"));
    std::path::PathBuf::from(base).join("speech-dispatcher").join("speechd.conf")
}

fn speech_rate() -> i32 {
    std::fs::read_to_string(speechd_user_conf())
        .ok()
        .and_then(|s| {
            s.lines().find_map(|l| l.trim().strip_prefix("DefaultRate").and_then(|v| v.trim().parse().ok()))
        })
        .unwrap_or(0)
}

/// speech-dispatcher replaces its whole system config with a user config if
/// one exists, so start from a copy of the system file and change one line.
fn set_speech_rate(rate: i32) {
    let path = speechd_user_conf();
    let base = std::fs::read_to_string(&path)
        .or_else(|_| std::fs::read_to_string("/etc/speech-dispatcher/speechd.conf"))
        .unwrap_or_default();
    let mut lines: Vec<String> = base
        .lines()
        .filter(|l| !l.trim_start().starts_with("DefaultRate") && !l.trim_start().starts_with("# DefaultRate"))
        .map(str::to_string)
        .collect();
    lines.push(format!("DefaultRate {rate}"));
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, lines.join("\n") + "\n");
    // The daemon is started on demand; stopping it makes the next use read the new config.
    let _ = Command::new("pkill").args(["-x", "speech-dispatcher"]).status();
}

fn screen_reader_on() -> bool {
    kconfig::read("kaccessrc", &["ScreenReader"], "Enabled").as_deref() == Some("true")
}

fn speech_group() -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("Speech");

    let has_orca = std::path::Path::new("/usr/bin/orca").exists();
    let reader = adw::SwitchRow::new();
    reader.set_title("Screen reader");
    reader.set_subtitle(if has_orca {
        "Read out what's on screen with Orca. Starts automatically when you sign in"
    } else {
        "The Orca screen reader is not installed"
    });
    reader.add_prefix(&gtk4::Image::from_icon_name("preferences-desktop-accessibility-symbolic"));
    reader.set_active(has_orca && screen_reader_on());
    reader.set_sensitive(has_orca);
    reader.connect_active_notify(|r| {
        let on = r.is_active();
        kconfig::spawn(move || {
            kconfig::write("kaccessrc", &["ScreenReader"], "Enabled", if on { "true" } else { "false" });
            if on {
                let _ = Command::new("orca").arg("--replace").spawn();
            } else {
                let _ = Command::new("pkill").args(["-f", "/usr/bin/orca"]).status();
            }
        });
    });
    g.add(&reader);

    let has_tts = std::path::Path::new("/usr/bin/spd-say").exists();
    let rate = adw::ActionRow::new();
    rate.set_title("Speaking rate");
    rate.add_prefix(&gtk4::Image::from_icon_name("audio-speakers-symbolic"));
    rate.set_activatable(false);
    if !has_tts {
        rate.set_subtitle("Text-to-speech is not installed");
        g.add(&rate);
        return g;
    }
    rate.set_subtitle("How fast the screen reader and other apps speak");
    let scale = gtk4::Scale::with_range(gtk4::Orientation::Horizontal, -100.0, 100.0, 10.0);
    scale.set_value(speech_rate() as f64);
    scale.add_mark(0.0, gtk4::PositionType::Bottom, Some("Normal"));
    scale.set_size_request(220, -1);
    scale.set_valign(gtk4::Align::Center);
    scale.connect_value_changed(|s| {
        let v = s.value().round() as i32;
        kconfig::spawn(move || set_speech_rate(v));
    });
    rate.add_suffix(&scale);
    g.add(&rate);

    let test = adw::ActionRow::new();
    test.set_title("Test voice");
    let play = gtk4::Button::from_icon_name("media-playback-start-symbolic");
    play.set_valign(gtk4::Align::Center);
    play.add_css_class("flat");
    play.set_tooltip_text(Some("Speak a sample sentence"));
    let scale2 = scale.clone();
    play.connect_clicked(move |_| {
        let r = (scale2.value().round() as i32).to_string();
        std::thread::spawn(move || {
            let _ = Command::new("spd-say")
                .args(["-w", "-r", &r, "This is how Zohara sounds when it reads to you."])
                .status();
        });
    });
    test.add_suffix(&play);
    test.set_activatable_widget(Some(&play));
    g.add(&test);
    g
}

// ── Voice typing ───────────────────────────────────────────────────────────

fn voice_typing_group() -> adw::PreferencesGroup {
    use crate::backend::dictation;
    let g = adw::PreferencesGroup::new();
    g.set_title("Voice typing");
    g.set_description(Some(
        "Press the shortcut, speak, and your words are typed wherever the cursor is.          Speech is recognised on this computer and never sent anywhere.",
    ));
    let s = dictation::settings();
    let missing = dictation::missing();

    let on = adw::SwitchRow::new();
    on.set_title("Voice typing");
    on.add_prefix(&gtk4::Image::from_icon_name("audio-input-microphone-symbolic"));
    let subtitle = match &missing {
        Some(why) => why.clone(),
        None => format!("Press {} to start or finish", dictation::shortcut_text()),
    };
    on.set_subtitle(&subtitle);
    on.set_active(missing.is_none() && s.enabled);
    on.set_sensitive(missing.is_none());
    on.connect_active_notify(|r| {
        let v = if r.is_active() { "true" } else { "false" };
        kconfig::spawn(move || dictation::set("Enabled", v));
    });
    g.add(&on);
    if missing.is_some() {
        return g;
    }

    let lang = adw::ComboRow::new();
    lang.set_title("Language");
    lang.set_subtitle("The language you'll speak");
    let labels: Vec<&str> = dictation::LANGUAGES.iter().map(|(l, _)| *l).collect();
    lang.set_model(Some(&gtk4::StringList::new(&labels)));
    lang.set_selected(dictation::LANGUAGES.iter().position(|(_, c)| *c == s.language).unwrap_or(0) as u32);
    lang.connect_selected_notify(|r| {
        if let Some((_, code)) = dictation::LANGUAGES.get(r.selected() as usize) {
            kconfig::spawn(move || dictation::set("Language", code));
        }
    });
    g.add(&lang);

    let auto = adw::SwitchRow::new();
    auto.set_title("Stop when I stop talking");
    auto.set_subtitle("Otherwise, press the shortcut again to finish");
    auto.set_active(s.auto_stop);
    auto.connect_active_notify(|r| {
        let v = if r.is_active() { "true" } else { "false" };
        kconfig::spawn(move || dictation::set("AutoStop", v));
    });
    g.add(&auto);

    let key = adw::ActionRow::new();
    key.set_title("Shortcut");
    key.set_subtitle(&dictation::shortcut_text());
    let change = gtk4::Button::with_label("Change");
    change.set_valign(gtk4::Align::Center);
    change.connect_clicked(|b| crate::pages::shortcuts::open(b.upcast_ref()));
    key.add_suffix(&change);
    g.add(&key);

    if !dictation::can_type() {
        let note = adw::ActionRow::new();
        note.set_title("Typing into apps isn't set up");
        note.set_subtitle("What you say is copied to the clipboard instead. Paste it with Ctrl+V");
        note.add_prefix(&gtk4::Image::from_icon_name("dialog-information-symbolic"));
        g.add(&note);
    }

    let tryit = adw::EntryRow::new();
    tryit.set_title("Try it: click here, then press the shortcut and speak");
    g.add(&tryit);

    for w in [lang.upcast_ref::<gtk4::Widget>(), auto.upcast_ref(), key.upcast_ref(), tryit.upcast_ref()] {
        on.bind_property("active", w, "sensitive").sync_create().build();
    }
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
        root.append(&speech_group());
        root.append(&voice_typing_group());
    }

    scroll.set_child(Some(&root));
    scroll.upcast()
}
