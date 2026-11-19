//! Personalization > Lock screen. Everything is written to `kscreenlockerrc`, KDE's own config file for the lock
//! screen, so these are the same settings Plasma's own page changes. Each change is read back before "Saved" is shown.

use crate::backend::{kconfig, worker::in_background};
use gtk4::prelude::*;
use gtk4::gio;
use libadwaita as adw;
use adw::prelude::*;
use std::process::Command;
use std::rc::Rc;

/// The choices of "Lock after inactivity": label and minutes (`None` = never lock).
pub const LOCK_CHOICES: [(&str, Option<u32>); 6] = [
    ("1 minute", Some(1)),
    ("5 minutes", Some(5)),
    ("10 minutes", Some(10)),
    ("30 minutes", Some(30)),
    ("1 hour", Some(60)),
    ("Never", None),
];

/// "Ask for the password after": label and seconds. KDE stores the grace time in seconds.
pub const GRACE_CHOICES: [(&str, u32); 5] = [("Right away", 0), ("5 seconds", 5), ("30 seconds", 30), ("1 minute", 60), ("5 minutes", 300)];

const RC: &str = "kscreenlockerrc";

/// ONE background for every screen that is not the desktop: the lock screen (kscreenlockerrc) and the sign-in screen that
/// shows when the computer starts, after a log out and for "switch user" (SDDM). SDDM is a separate program with its own
/// background in the theme's `theme.conf.user`; it runs as the `sddm` user, which cannot read a picture in someone's home
/// folder, so a picture is copied to a world-readable place first. This page keeps the two in step.
const LOGIN_DIR: &str = "/usr/share/zohara/login";
const LOGIN_THEME_CONF: &str = "/usr/share/sddm/themes/breeze/theme.conf.user";

/// The extension of a picture the sign-in screen can use, lower-case; `None` for anything else.
pub fn login_ext(path: &str) -> Option<&'static str> {
    match std::path::Path::new(path).extension()?.to_str()?.to_lowercase().as_str() {
        "jpg" | "jpeg" => Some("jpg"),
        "png" => Some("png"),
        "webp" => Some("webp"),
        _ => None,
    }
}

/// Shell text (run as root) that edits only the `type`, `background` and `color` lines of the theme file, keeping the rest
/// (clock, logo, fonts). `@SET@` is the lines to add under [General].
const EDIT_THEME: &str = r#"f=@CONF@; mkdir -p "$(dirname "$f")"; touch "$f"; sed -i -e '/^background=/d' -e '/^type=/d' -e '/^color=/d' "$f"; grep -q '^\[General\]' "$f" || echo '[General]' >> "$f"; @SET@"#;

fn edit_theme(set_lines: &str) -> String {
    EDIT_THEME.replace("@CONF@", LOGIN_THEME_CONF).replace("@SET@", set_lines)
}

/// Script (run as root, the picture path is `$1`): copy the picture and point the sign-in screen at it.
pub fn login_picture_script(ext: &str) -> String {
    let set = format!(r#"sed -i '/^\[General\]/a type=image\nbackground={LOGIN_DIR}/background.{ext}' "$f""#);
    format!(r#"set -e; mkdir -p {LOGIN_DIR}; rm -f {LOGIN_DIR}/background.*; install -m 644 "$1" {LOGIN_DIR}/background.{ext}; {}"#, edit_theme(&set))
}

/// Script (colour `#rrggbb` is `$1`): a plain colour on the sign-in screen.
pub fn login_colour_script() -> String {
    let set = r#"sed -i "/^\[General\]/a type=color\ncolor=$1" "$f""#;
    format!("set -e; rm -rf {LOGIN_DIR}; {}", edit_theme(set))
}

/// Script: back to the theme's own picture.
pub fn login_default_script() -> String {
    format!("set -e; rm -rf {LOGIN_DIR}; {}", edit_theme(":"))
}

/// `r,g,b` (0-255, as kscreenlockerrc stores it) as `#rrggbb`.
pub fn hex_colour(rgb: &str) -> Option<String> {
    let v: Vec<u32> = rgb.split(',').filter_map(|x| x.trim().parse().ok()).collect();
    (v.len() >= 3 && v.iter().take(3).all(|c| *c <= 255)).then(|| format!("#{:02x}{:02x}{:02x}", v[0], v[1], v[2]))
}

fn theme_value(key: &str) -> Option<String> {
    let t = std::fs::read_to_string(LOGIN_THEME_CONF).ok()?;
    t.lines().find_map(|l| l.trim().strip_prefix(&format!("{key}=")).map(str::to_string))
}

/// What the lock screen shows now, from kscreenlockerrc.
#[derive(Debug, PartialEq, Clone)]
pub enum Look {
    Picture(String),
    Colour(String),
    Default,
}

fn lock_look() -> Look {
    match kconfig::read(RC, &["Greeter"], "WallpaperPlugin").as_deref() {
        Some("org.kde.color") => kconfig::read(RC, &["Greeter", "Wallpaper", "org.kde.color", "General"], "Color").and_then(|c| hex_colour(&c)).map(Look::Colour).unwrap_or(Look::Default),
        _ => image_path().filter(|p| std::path::Path::new(p).exists()).map(Look::Picture).unwrap_or(Look::Default),
    }
}

/// Does the sign-in screen show the same as the lock screen?
fn login_in_sync(look: &Look) -> bool {
    match look {
        Look::Default => theme_value("background").map_or(true, |b| !b.starts_with(LOGIN_DIR)) && theme_value("type").map_or(true, |t| t != "color"),
        Look::Colour(hex) => theme_value("type").as_deref() == Some("color") && theme_value("color").map(|c| c.eq_ignore_ascii_case(hex)).unwrap_or(false),
        Look::Picture(path) => {
            let Some(bg) = theme_value("background").filter(|b| b.starts_with(LOGIN_DIR)) else { return false };
            matches!((std::fs::read(path), std::fs::read(&bg)), (Ok(a), Ok(b)) if a == b)
        }
    }
}

/// Makes the sign-in screen show `look` (asks for the administrator password).
fn apply_login(look: &Look) -> Result<(), String> {
    let pk = super::accounts::pkexec_sh;
    match look {
        Look::Picture(p) => {
            let ext = login_ext(p).ok_or_else(|| "The sign-in screen can only use JPEG, PNG or WebP pictures.".to_string())?;
            pk(&login_picture_script(ext), &[p], None)
        }
        Look::Colour(hex) => pk(&login_colour_script(), &[hex], None),
        Look::Default => pk(&login_default_script(), &[], None),
    }
}

/// Which choice matches what is stored: the nearest listed time at or above `minutes`, and "Never" when auto-lock is off.
pub fn lock_choice_for(autolock: bool, minutes: u32) -> u32 {
    if !autolock {
        return (LOCK_CHOICES.len() - 1) as u32;
    }
    LOCK_CHOICES.iter().position(|(_, m)| m.map_or(false, |m| m >= minutes)).unwrap_or(LOCK_CHOICES.len() - 2) as u32
}

/// The grace choice at or above `seconds`.
pub fn grace_choice_for(seconds: u32) -> u32 {
    GRACE_CHOICES.iter().position(|(_, s)| *s >= seconds).unwrap_or(GRACE_CHOICES.len() - 1) as u32
}

fn read_bool(groups: &[&str], key: &str, default: bool) -> bool {
    kconfig::read(RC, groups, key).map(|v| v != "false").unwrap_or(default)
}

fn image_path() -> Option<String> {
    kconfig::read(RC, &["Greeter", "Wallpaper", "org.kde.image", "General"], "Image").map(|v| v.trim_start_matches("file://").to_string())
}

/// Shows a short result under the controls after a change was written and read back.
fn report(status: &gtk4::Label, ok: bool, saved: String) {
    status.set_text(&if ok { format!("Saved: {saved}") } else { "Couldn't save this setting".to_string() });
}

pub fn open(parent_row: &adw::ActionRow) {
    let content = super::personalization::dialog_content_box();
    let status = gtk4::Label::new(Some("Changes are saved as you make them."));
    status.set_halign(gtk4::Align::Start);
    status.set_wrap(true);
    status.add_css_class("dim-label");

    // ── Background ──
    let bg = adw::PreferencesGroup::new();
    bg.set_title("Background");

    let photo = adw::ActionRow::new();
    photo.set_title("Photo");
    let current = image_path();
    photo.set_subtitle(&current.as_deref().and_then(|p| std::path::Path::new(p).file_name().map(|n| n.to_string_lossy().to_string())).unwrap_or_else(|| "Using the default picture".to_string()));
    if let Some(p) = current.as_deref().filter(|p| std::path::Path::new(p).exists()) {
        let pic = gtk4::Picture::for_filename(p);
        pic.set_size_request(72, 44);
        pic.set_content_fit(gtk4::ContentFit::Cover);
        pic.add_css_class("card");
        photo.add_prefix(&pic);
    }
    // The sign-in screen (computer start-up, log out, switch user) always shows the same as the lock screen.
    let sync = adw::ActionRow::new();
    sync.set_title("Sign-in screen");
    sync.set_subtitle("Checking…");
    let sync_btn = gtk4::Button::with_label("Match now");
    sync_btn.set_valign(gtk4::Align::Center);
    sync_btn.set_visible(false);
    sync.add_suffix(&sync_btn);
    const SAME: &str = "Same as the lock screen. It also shows when the computer starts, after you log out and when you switch user";
    {
        let (sync, btn) = (sync.clone(), sync_btn.clone());
        in_background(|| login_in_sync(&lock_look()), move |ok| {
            sync.set_subtitle(if ok { SAME } else { "Different from the lock screen" });
            btn.set_visible(!ok);
        });
    }
    {
        let (sync, status) = (sync.clone(), status.clone());
        sync_btn.connect_clicked(move |btn| {
            let (sync, status, btn) = (sync.clone(), status.clone(), btn.clone());
            in_background(|| apply_login(&lock_look()), move |res| match res {
                Ok(()) => {
                    sync.set_subtitle(SAME);
                    btn.set_visible(false);
                    report(&status, true, "the sign-in screen now matches the lock screen".into());
                }
                Err(e) => status.set_text(&format!("Couldn't change the sign-in screen: {e}")),
            });
        });
    }
    let choose = gtk4::Button::with_label("Choose photo…");
    choose.set_valign(gtk4::Align::Center);
    {
        let (photo, status, sync) = (photo.clone(), status.clone(), sync.clone());
        choose.connect_clicked(move |btn| {
            let parent = btn.root().and_downcast::<gtk4::Window>();
            let dialog = gtk4::FileDialog::new();
            dialog.set_title("Select a lock screen picture");
            let filter = gtk4::FileFilter::new();
            filter.add_mime_type("image/*");
            filter.set_name(Some("Pictures"));
            let filters = gio::ListStore::new::<gtk4::FileFilter>();
            filters.append(&filter);
            dialog.set_filters(Some(&filters));
            let (photo, status, sync) = (photo.clone(), status.clone(), sync.clone());
            dialog.open(parent.as_ref(), None::<&gio::Cancellable>, move |res| {
                let Some(path) = res.ok().and_then(|f| f.path()) else { return };
                let p = path.to_string_lossy().to_string();
                let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                let (photo, status) = (photo.clone(), status.clone());
                                in_background(
                    move || {
                        kconfig::write(RC, &["Greeter"], "WallpaperPlugin", "org.kde.image");
                        kconfig::write(RC, &["Greeter", "Wallpaper", "org.kde.image", "General"], "Image", &p);
                        let ok = image_path().as_deref() == Some(p.as_str());
                        let login = if ok { Some(apply_login(&Look::Picture(p.clone()))) } else { None };
                        (ok, login)
                    },
                    move |(ok, login)| {
                        if ok {
                            photo.set_subtitle(&name);
                            sync.set_subtitle(if matches!(login, Some(Ok(()))) { "Same as the lock screen" } else { "Different from the lock screen" });
                        }
                        match login {
                            Some(Err(e)) => status.set_text(&format!("Saved the lock screen picture {name}, but not for the sign-in screen: {e}")),
                            Some(Ok(())) => report(&status, ok, format!("lock screen and sign-in picture {name}")),
                            None => report(&status, ok, format!("lock screen picture {name}")),
                        }
                    },
                );
            });
        });
    }
    photo.add_suffix(&choose);
    bg.add(&photo);
    bg.add(&sync);


    let colour = adw::ActionRow::new();
    colour.set_title("Plain colour");
    colour.set_subtitle("Use one colour instead of a picture");
    let cbtn = gtk4::ColorDialogButton::new(Some(gtk4::ColorDialog::new()));
    cbtn.set_valign(gtk4::Align::Center);
    let start = kconfig::read(RC, &["Greeter", "Wallpaper", "org.kde.color", "General"], "Color")
        .and_then(|v| {
            let p: Vec<f32> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            (p.len() >= 3).then(|| gtk4::gdk::RGBA::new(p[0] / 255.0, p[1] / 255.0, p[2] / 255.0, 1.0))
        })
        .unwrap_or_else(|| gtk4::gdk::RGBA::new(0.1, 0.1, 0.12, 1.0));
    cbtn.set_rgba(&start);
    let colour_ready = Rc::new(std::cell::Cell::new(false));
    {
        let (status, ready, sync_outer) = (status.clone(), colour_ready.clone(), sync.clone());
        cbtn.connect_rgba_notify(move |b| {
            if !ready.get() {
                return;
            }
            let c = b.rgba();
            let v = format!("{},{},{}", (c.red() * 255.0).round() as u32, (c.green() * 255.0).round() as u32, (c.blue() * 255.0).round() as u32);
            let (status, sync) = (status.clone(), sync_outer.clone());
            in_background(
                move || {
                    kconfig::write(RC, &["Greeter"], "WallpaperPlugin", "org.kde.color");
                    kconfig::write(RC, &["Greeter", "Wallpaper", "org.kde.color", "General"], "Color", &v);
                    let ok = kconfig::read(RC, &["Greeter"], "WallpaperPlugin").as_deref() == Some("org.kde.color");
                    let login = if ok { hex_colour(&v).map(|h| apply_login(&Look::Colour(h))) } else { None };
                    (ok, login)
                },
                move |(ok, login)| match login {
                    Some(Err(e)) => status.set_text(&format!("Saved the colour for the lock screen, but not for the sign-in screen: {e}")),
                    Some(Ok(())) => {
                        sync.set_subtitle(SAME);
                        report(&status, ok, "plain colour on the lock and sign-in screens".into());
                    }
                    None => report(&status, ok, "plain colour background".into()),
                },
            );
        });
    }
    colour_ready.set(true);
    colour.add_suffix(&cbtn);
    bg.add(&colour);

    let reset = adw::ActionRow::new();
    reset.set_title("Use the default background");
    reset.set_subtitle("Go back to the picture that came with Zohara OS");
    let rb = gtk4::Button::with_label("Reset");
    rb.set_valign(gtk4::Align::Center);
    {
        let (photo, status, sync) = (photo.clone(), status.clone(), sync.clone());
        rb.connect_clicked(move |_| {
            let (photo, status, sync) = (photo.clone(), status.clone(), sync.clone());
            in_background(
                || {
                    kconfig::delete(RC, &["Greeter"], "WallpaperPlugin");
                    kconfig::delete(RC, &["Greeter", "Wallpaper", "org.kde.image", "General"], "Image");
                    kconfig::delete(RC, &["Greeter", "Wallpaper", "org.kde.color", "General"], "Color");
                    let ok = image_path().is_none();
                    let login = if ok { Some(apply_login(&Look::Default)) } else { None };
                    (ok, login)
                },
                move |(ok, login)| {
                    if ok {
                        photo.set_subtitle("Using the default picture");
                    }
                    match login {
                        Some(Err(e)) => status.set_text(&format!("Back to the default for the lock screen, but not for the sign-in screen: {e}")),
                        Some(Ok(())) => {
                            sync.set_subtitle(SAME);
                            report(&status, ok, "default background on the lock and sign-in screens".into());
                        }
                        None => report(&status, ok, "default background".into()),
                    }
                },
            );
        });
    }
    reset.add_suffix(&rb);
    bg.add(&reset);
    content.append(&bg);

    // ── When it locks ──
    let when = adw::PreferencesGroup::new();
    when.set_title("When the screen locks");

    let autolock = read_bool(&["Daemon"], "Autolock", true);
    let minutes = kconfig::read(RC, &["Daemon"], "Timeout").and_then(|v| v.parse().ok()).unwrap_or(5);
    let timeout_row = adw::ComboRow::new();
    timeout_row.set_title("Lock after inactivity");
    timeout_row.set_model(Some(&gtk4::StringList::new(&LOCK_CHOICES.map(|c| c.0))));
    timeout_row.set_selected(lock_choice_for(autolock, minutes));

    let resume = adw::SwitchRow::new();
    resume.set_title("Lock after waking from sleep");
    resume.set_subtitle("Ask for the password when the laptop wakes up");
    resume.set_active(read_bool(&["Daemon"], "LockOnResume", true));

    let grace_row = adw::ComboRow::new();
    grace_row.set_title("Ask for the password");
    grace_row.set_subtitle("How long after locking before the password is needed");
    grace_row.set_model(Some(&gtk4::StringList::new(&GRACE_CHOICES.map(|c| c.0))));
    let grace_now: u32 = kconfig::read(RC, &["Daemon"], "LockGrace").and_then(|v| v.parse().ok()).unwrap_or(5);
    grace_row.set_selected(grace_choice_for(grace_now));

    let ready = Rc::new(std::cell::Cell::new(false));
    {
        let (status, ready) = (status.clone(), ready.clone());
        timeout_row.connect_selected_notify(move |r| {
            if !ready.get() {
                return;
            }
            let (label, minutes) = LOCK_CHOICES[(r.selected() as usize).min(LOCK_CHOICES.len() - 1)];
            status.set_text("Saving…");
            let status = status.clone();
            in_background(
                move || {
                    if !kconfig::available() {
                        return false;
                    }
                    match minutes {
                        Some(m) => {
                            kconfig::write_typed(RC, &["Daemon"], "Autolock", "bool", "true");
                            kconfig::write(RC, &["Daemon"], "Timeout", &m.to_string());
                        }
                        None => kconfig::write_typed(RC, &["Daemon"], "Autolock", "bool", "false"),
                    }
                    let on = read_bool(&["Daemon"], "Autolock", true);
                    let t: u32 = kconfig::read(RC, &["Daemon"], "Timeout").and_then(|v| v.parse().ok()).unwrap_or(0);
                    match minutes {
                        Some(m) => on && t == m,
                        None => !on,
                    }
                },
                move |ok| report(&status, ok, format!("lock after {}", label.to_lowercase())),
            );
        });
    }
    {
        let (status, ready) = (status.clone(), ready.clone());
        resume.connect_active_notify(move |s| {
            if !ready.get() {
                return;
            }
            let on = s.is_active();
            let status = status.clone();
            in_background(
                move || {
                    kconfig::write_typed(RC, &["Daemon"], "LockOnResume", "bool", if on { "true" } else { "false" });
                    read_bool(&["Daemon"], "LockOnResume", true) == on
                },
                move |ok| report(&status, ok, format!("lock after sleep {}", if on { "on" } else { "off" })),
            );
        });
    }
    {
        let (status, ready) = (status.clone(), ready.clone());
        grace_row.connect_selected_notify(move |r| {
            if !ready.get() {
                return;
            }
            let (label, secs) = GRACE_CHOICES[(r.selected() as usize).min(GRACE_CHOICES.len() - 1)];
            let status = status.clone();
            in_background(
                move || {
                    kconfig::write(RC, &["Daemon"], "LockGrace", &secs.to_string());
                    kconfig::read(RC, &["Daemon"], "LockGrace").and_then(|v| v.parse::<u32>().ok()) == Some(secs)
                },
                move |ok| report(&status, ok, format!("password needed {}", label.to_lowercase())),
            );
        });
    }
    ready.set(true);
    when.add(&timeout_row);
    when.add(&resume);
    when.add(&grace_row);
    content.append(&when);

    // ── Now ──
    let now = adw::PreferencesGroup::new();
    now.set_title("Try it");
    let lock_now = adw::ActionRow::new();
    lock_now.set_title("Lock the screen now");
    lock_now.set_subtitle("Type your password to come back");
    let lb = gtk4::Button::with_label("Lock now");
    lb.set_valign(gtk4::Align::Center);
    lb.add_css_class("suggested-action");
    lb.connect_clicked(|_| {
        // Plasma answers to the session lock; fall back to its own D-Bus call.
        if Command::new("loginctl").arg("lock-session").status().map(|s| !s.success()).unwrap_or(true) {
            let _ = Command::new("dbus-send")
                .args(["--session", "--type=method_call", "--dest=org.freedesktop.ScreenSaver", "/ScreenSaver", "org.freedesktop.ScreenSaver.Lock"])
                .status();
        }
    });
    lock_now.add_suffix(&lb);
    now.add(&lock_now);
    content.append(&now);
    content.append(&status);

    super::personalization::open_settings_window_sized(parent_row, "Lock Screen", &content, 560, 700);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_choice_matches_what_is_stored() {
        assert_eq!(lock_choice_for(true, 1), 0);
        assert_eq!(lock_choice_for(true, 5), 1);
        assert_eq!(lock_choice_for(true, 7), 2); // between 5 and 10: the next one up
        assert_eq!(lock_choice_for(true, 30), 3);
        assert_eq!(lock_choice_for(true, 60), 4);
        assert_eq!(lock_choice_for(true, 600), 4);
        assert_eq!(lock_choice_for(false, 5), 5);
    }

    #[test]
    fn the_sign_in_screen_takes_jpeg_png_and_webp_only() {
        assert_eq!(login_ext("/home/a/Pictures/x.JPG"), Some("jpg"));
        assert_eq!(login_ext("/a/b.jpeg"), Some("jpg"));
        assert_eq!(login_ext("/a/b.png"), Some("png"));
        assert_eq!(login_ext("/a/b.webp"), Some("webp"));
        assert_eq!(login_ext("/a/b.gif"), None);
        assert_eq!(login_ext("/a/noext"), None);
    }

    #[test]
    fn colours_become_hex() {
        assert_eq!(hex_colour("26,26,31").as_deref(), Some("#1a1a1f"));
        assert_eq!(hex_colour("255, 0, 128").as_deref(), Some("#ff0080"));
        assert_eq!(hex_colour("300,0,0"), None);
        assert_eq!(hex_colour("1,2"), None);
    }

    #[test]
    fn the_sign_in_scripts_edit_only_their_lines_and_never_overwrite_the_theme_file() {
        for s in [login_picture_script("png"), login_colour_script(), login_default_script()] {
            assert!(s.contains("sed -i -e '/^background=/d' -e '/^type=/d' -e '/^color=/d'"), "{s}");
            assert!(s.contains(LOGIN_THEME_CONF) && !s.contains(" > \"$f\""), "{s}");
        }
        assert!(login_picture_script("png").contains("install -m 644 \"$1\" /usr/share/zohara/login/background.png"));
        assert!(login_picture_script("jpg").contains("type=image"));
        assert!(login_colour_script().contains("type=color") && login_colour_script().contains("color=$1"));
        assert!(login_default_script().contains("rm -rf /usr/share/zohara/login"));
    }

    #[test]
    fn grace_choice_is_in_seconds() {
        assert_eq!(grace_choice_for(0), 0);
        assert_eq!(grace_choice_for(5), 1);
        assert_eq!(grace_choice_for(20), 2);
        assert_eq!(grace_choice_for(9999), 4);
    }
}
