//! Personalization > Text input: on-screen keyboard, emoji picker, Num Lock at start-up and what the Caps Lock key does.
//! Key repeat and layouts live on the Keyboard page.

use crate::backend::{kconfig, worker::in_background};
use gtk4::prelude::*;
use libadwaita as adw;
use adw::prelude::*;
use std::process::Command;
use std::rc::Rc;

/// What the Caps Lock key can do: label and the xkb option that makes it so (None = normal).
pub const CAPS_CHOICES: [(&str, Option<&str>); 4] = [
    ("Caps Lock (normal)", None),
    ("Escape", Some("caps:escape")),
    ("Control", Some("ctrl:nocaps")),
    ("Nothing", Some("caps:none")),
];

/// Removes every Caps Lock option from a comma separated xkb option list and adds the chosen one.
pub fn with_caps_option(options: &str, choice: Option<&str>) -> String {
    let mut v: Vec<&str> = options.split(',').map(str::trim).filter(|o| !o.is_empty() && !o.starts_with("caps:") && *o != "ctrl:nocaps").collect();
    if let Some(c) = choice {
        v.push(c);
    }
    v.join(",")
}

/// Which Caps Lock choice a stored option list means.
pub fn caps_choice_of(options: &str) -> u32 {
    let has = |o: &str| options.split(',').any(|x| x.trim() == o);
    CAPS_CHOICES.iter().position(|(_, o)| o.map_or(false, |o| has(o))).unwrap_or(0) as u32
}

/// Num Lock at start-up: label and the value kcminputrc stores (0 on, 1 off, 2 leave as it was).
pub const NUMLOCK_CHOICES: [(&str, &str); 3] = [("Turn on", "0"), ("Turn off", "1"), ("Leave as it was", "2")];

/// First installed on-screen keyboard, if any.
fn find_keyboard() -> Option<&'static str> {
    ["maliit-keyboard", "wvkbd-mobintl", "onboard", "florence"].into_iter().find(|c| which(c))
}

fn which(cmd: &str) -> bool {
    Command::new("sh").args(["-c", &format!("command -v {cmd}")]).output().map(|o| o.status.success()).unwrap_or(false)
}

fn reload_keyboard() {
    let _ = Command::new("dbus-send").args(["--session", "--type=signal", "/Layouts", "org.kde.keyboard.reloadConfig"]).status();
    let _ = Command::new("dbus-send").args(["--session", "--type=method_call", "--dest=org.kde.KWin", "/KWin", "org.kde.KWin.reconfigure"]).status();
}

pub fn open(parent_row: &adw::ActionRow) {
    let content = super::personalization::dialog_content_box();
    let status = gtk4::Label::new(Some("Changes are saved as you make them."));
    status.set_halign(gtk4::Align::Start);
    status.set_wrap(true);
    status.add_css_class("dim-label");

    let tools = adw::PreferencesGroup::new();
    tools.set_title("Typing tools");

    let osk = adw::ActionRow::new();
    osk.set_title("On-screen keyboard");
    match find_keyboard() {
        Some(k) => {
            osk.set_subtitle("Type by tapping or clicking keys on the screen");
            let b = gtk4::Button::with_label("Open");
            b.set_valign(gtk4::Align::Center);
            b.connect_clicked(move |_| {
                let _ = Command::new(k).spawn();
            });
            osk.add_suffix(&b);
        }
        None => osk.set_subtitle("No on-screen keyboard is installed on this computer"),
    }
    tools.add(&osk);

    let emoji = adw::ActionRow::new();
    emoji.set_title("Emoji picker");
    if which("plasma-emojier") {
        emoji.set_subtitle("Press the Windows key and the full stop (Meta + .) in any app, or open it here");
        let b = gtk4::Button::with_label("Open");
        b.set_valign(gtk4::Align::Center);
        b.connect_clicked(|_| {
            let _ = Command::new("plasma-emojier").spawn();
        });
        emoji.add_suffix(&b);
    } else {
        emoji.set_subtitle("The emoji picker isn't installed on this computer");
    }
    tools.add(&emoji);

    let voice = adw::ActionRow::new();
    voice.set_title("Voice typing");
    voice.set_subtitle("Set up in Accessibility, under Speech");
    tools.add(&voice);
    content.append(&tools);

    let keys = adw::PreferencesGroup::new();
    keys.set_title("Keys");

    let caps = adw::ComboRow::new();
    caps.set_title("Caps Lock key");
    caps.set_subtitle("What the Caps Lock key does");
    caps.set_model(Some(&gtk4::StringList::new(&CAPS_CHOICES.map(|c| c.0))));
    let opts = kconfig::read("kxkbrc", &["Layout"], "Options").unwrap_or_default();
    caps.set_selected(caps_choice_of(&opts));

    let num = adw::ComboRow::new();
    num.set_title("Num Lock at start-up");
    num.set_subtitle("Applies the next time you sign in");
    num.set_model(Some(&gtk4::StringList::new(&NUMLOCK_CHOICES.map(|c| c.0))));
    let cur = kconfig::read("kcminputrc", &["Keyboard"], "NumLock").unwrap_or_else(|| "2".into());
    num.set_selected(NUMLOCK_CHOICES.iter().position(|(_, v)| *v == cur).unwrap_or(2) as u32);

    let ready = Rc::new(std::cell::Cell::new(false));
    {
        let (status, ready) = (status.clone(), ready.clone());
        caps.connect_selected_notify(move |r| {
            if !ready.get() {
                return;
            }
            let (label, opt) = CAPS_CHOICES[(r.selected() as usize).min(CAPS_CHOICES.len() - 1)];
            let status = status.clone();
            in_background(
                move || {
                    let now = kconfig::read("kxkbrc", &["Layout"], "Options").unwrap_or_default();
                    let new = with_caps_option(&now, opt);
                    kconfig::write_typed("kxkbrc", &["Layout"], "ResetOldOptions", "bool", "true");
                    kconfig::write_typed("kxkbrc", &["Layout"], "Use", "bool", "true");
                    kconfig::write("kxkbrc", &["Layout"], "Options", &new);
                    reload_keyboard();
                    kconfig::read("kxkbrc", &["Layout"], "Options").unwrap_or_default() == new
                },
                move |ok| status.set_text(&if ok { format!("Saved: Caps Lock key is {}", label.to_lowercase()) } else { "Couldn't save this setting".into() }),
            );
        });
    }
    {
        let (status, ready) = (status.clone(), ready.clone());
        num.connect_selected_notify(move |r| {
            if !ready.get() {
                return;
            }
            let (label, v) = NUMLOCK_CHOICES[(r.selected() as usize).min(NUMLOCK_CHOICES.len() - 1)];
            let status = status.clone();
            in_background(
                move || {
                    kconfig::write("kcminputrc", &["Keyboard"], "NumLock", v);
                    kconfig::read("kcminputrc", &["Keyboard"], "NumLock").as_deref() == Some(v)
                },
                move |ok| status.set_text(&if ok { format!("Saved: Num Lock at start-up: {}", label.to_lowercase()) } else { "Couldn't save this setting".into() }),
            );
        });
    }
    ready.set(true);
    keys.add(&caps);
    keys.add(&num);
    content.append(&keys);
    content.append(&status);

    super::personalization::open_settings_window_sized(parent_row, "Text Input", &content, 560, 560);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_option_replaces_the_old_one_and_keeps_others() {
        assert_eq!(with_caps_option("", Some("caps:escape")), "caps:escape");
        assert_eq!(with_caps_option("compose:ralt,caps:escape", Some("ctrl:nocaps")), "compose:ralt,ctrl:nocaps");
        assert_eq!(with_caps_option("compose:ralt,caps:none", None), "compose:ralt");
    }

    #[test]
    fn stored_options_map_back_to_a_choice() {
        assert_eq!(caps_choice_of(""), 0);
        assert_eq!(caps_choice_of("caps:escape"), 1);
        assert_eq!(caps_choice_of("compose:ralt, ctrl:nocaps"), 2);
        assert_eq!(caps_choice_of("caps:none"), 3);
    }
}
