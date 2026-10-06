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
    let choose = gtk4::Button::with_label("Choose photo…");
    choose.set_valign(gtk4::Align::Center);
    {
        let (photo, status) = (photo.clone(), status.clone());
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
            let (photo, status) = (photo.clone(), status.clone());
            dialog.open(parent.as_ref(), None::<&gio::Cancellable>, move |res| {
                let Some(path) = res.ok().and_then(|f| f.path()) else { return };
                let p = path.to_string_lossy().to_string();
                let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                let (photo, status) = (photo.clone(), status.clone());
                in_background(
                    move || {
                        kconfig::write(RC, &["Greeter"], "WallpaperPlugin", "org.kde.image");
                        kconfig::write(RC, &["Greeter", "Wallpaper", "org.kde.image", "General"], "Image", &p);
                        image_path().as_deref() == Some(p.as_str())
                    },
                    move |ok| {
                        if ok {
                            photo.set_subtitle(&name);
                        }
                        report(&status, ok, format!("lock screen picture {name}"));
                    },
                );
            });
        });
    }
    photo.add_suffix(&choose);
    bg.add(&photo);

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
        let (status, ready) = (status.clone(), colour_ready.clone());
        cbtn.connect_rgba_notify(move |b| {
            if !ready.get() {
                return;
            }
            let c = b.rgba();
            let v = format!("{},{},{}", (c.red() * 255.0).round() as u32, (c.green() * 255.0).round() as u32, (c.blue() * 255.0).round() as u32);
            let status = status.clone();
            in_background(
                move || {
                    kconfig::write(RC, &["Greeter"], "WallpaperPlugin", "org.kde.color");
                    kconfig::write(RC, &["Greeter", "Wallpaper", "org.kde.color", "General"], "Color", &v);
                    kconfig::read(RC, &["Greeter"], "WallpaperPlugin").as_deref() == Some("org.kde.color")
                },
                move |ok| report(&status, ok, "plain colour background".into()),
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
        let (photo, status) = (photo.clone(), status.clone());
        rb.connect_clicked(move |_| {
            let (photo, status) = (photo.clone(), status.clone());
            in_background(
                || {
                    kconfig::delete(RC, &["Greeter"], "WallpaperPlugin");
                    kconfig::delete(RC, &["Greeter", "Wallpaper", "org.kde.image", "General"], "Image");
                    kconfig::delete(RC, &["Greeter", "Wallpaper", "org.kde.color", "General"], "Color");
                    image_path().is_none()
                },
                move |ok| {
                    if ok {
                        photo.set_subtitle("Using the default picture");
                    }
                    report(&status, ok, "default background".into());
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
    fn grace_choice_is_in_seconds() {
        assert_eq!(grace_choice_for(0), 0);
        assert_eq!(grace_choice_for(5), 1);
        assert_eq!(grace_choice_for(20), 2);
        assert_eq!(grace_choice_for(9999), 4);
    }
}
