//! Keyboard settings -- key repeat, layouts, and global shortcuts.
//!
//! Reads and writes Plasma's own config (kcminputrc, kxkbrc) with
//! kreadconfig6/kwriteconfig6, then tells Plasma's keyboard daemon and KWin
//! to reload, which is what System Settings itself does.

use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use std::cell::RefCell;
use std::process::Command;
use std::rc::Rc;

fn kread(file: &str, group: &str, key: &str) -> Option<String> {
    let o = Command::new("kreadconfig6")
        .args(["--file", file, "--group", group, "--key", key])
        .output()
        .ok()?;
    let v = String::from_utf8_lossy(&o.stdout).trim().to_string();
    (o.status.success() && !v.is_empty()).then_some(v)
}

fn kwrite(file: &str, group: &str, key: &str, value: &str) {
    let _ = Command::new("kwriteconfig6")
        .args(["--file", file, "--group", group, "--key", key, value])
        .status();
}

/// Ask Plasma's keyboard daemon (X11) and KWin (Wayland) to pick up changes.
fn reload_keyboard() {
    let _ = Command::new("dbus-send")
        .args(["--session", "--type=signal", "/Layouts", "org.kde.keyboard.reloadConfig"])
        .status();
    let _ = Command::new("dbus-send")
        .args(["--session", "--type=method_call", "--dest=org.kde.KWin", "/KWin", "org.kde.KWin.reconfigure"])
        .status();
}

fn write_and_reload(writes: Vec<(&'static str, &'static str, &'static str, String)>) {
    std::thread::spawn(move || {
        for (file, group, key, value) in &writes {
            kwrite(file, group, key, value);
        }
        reload_keyboard();
    });
}

/// (code, description) from the XKB rules, falling back to bare codes.
fn available_layouts() -> Vec<(String, String)> {
    if let Ok(text) = std::fs::read_to_string("/usr/share/X11/xkb/rules/evdev.lst") {
        let mut in_layouts = false;
        let mut out = Vec::new();
        for line in text.lines() {
            if let Some(section) = line.strip_prefix("! ") {
                in_layouts = section.trim() == "layout";
                continue;
            }
            if in_layouts {
                let t = line.trim();
                if let Some((code, desc)) = t.split_once(char::is_whitespace) {
                    out.push((code.to_string(), desc.trim().to_string()));
                }
            }
        }
        if !out.is_empty() {
            out.sort_by(|a, b| a.1.cmp(&b.1));
            return out;
        }
    }
    Command::new("localectl")
        .arg("list-x11-keymap-layouts")
        .output()
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|l| (l.trim().to_string(), l.trim().to_string()))
                .filter(|(c, _)| !c.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn split_list(v: Option<String>) -> Vec<String> {
    v.map(|s| s.split(',').map(|x| x.trim().to_string()).collect()).unwrap_or_default()
}

struct Layouts {
    codes: Vec<String>,
    variants: Vec<String>,
}

impl Layouts {
    fn load() -> Self {
        let codes: Vec<String> =
            split_list(kread("kxkbrc", "Layout", "LayoutList")).into_iter().filter(|c| !c.is_empty()).collect();
        let mut variants = split_list(kread("kxkbrc", "Layout", "VariantList"));
        variants.resize(codes.len(), String::new());
        Layouts { codes, variants }
    }

    fn save(&self) {
        write_and_reload(vec![
            ("kxkbrc", "Layout", "Use", "true".into()),
            ("kxkbrc", "Layout", "LayoutList", self.codes.join(",")),
            ("kxkbrc", "Layout", "VariantList", self.variants.join(",")),
        ]);
    }
}

fn rebuild_layout_rows(
    group: &adw::PreferencesGroup,
    rows: &Rc<RefCell<Vec<adw::ActionRow>>>,
    state: &Rc<RefCell<Layouts>>,
    names: &Rc<Vec<(String, String)>>,
) {
    for r in rows.borrow_mut().drain(..) {
        group.remove(&r);
    }
    let codes = state.borrow().codes.clone();
    if codes.is_empty() {
        let row = adw::ActionRow::new();
        row.set_title("System default layout");
        row.set_subtitle("Add a layout to choose it explicitly and switch between several");
        group.add(&row);
        rows.borrow_mut().push(row);
        return;
    }
    for (i, code) in codes.iter().enumerate() {
        let desc = names.iter().find(|(c, _)| c == code).map(|(_, d)| d.clone()).unwrap_or_else(|| code.clone());
        let row = adw::ActionRow::new();
        row.set_title(&glib::markup_escape_text(&desc));
        row.set_subtitle(if i == 0 { "Default" } else { code.as_str() });
        if codes.len() > 1 {
            let remove = gtk4::Button::from_icon_name("user-trash-symbolic");
            remove.set_css_classes(&["flat"]);
            remove.set_valign(gtk4::Align::Center);
            remove.set_tooltip_text(Some("Remove layout"));
            let (group2, rows2, state2, names2) = (group.clone(), rows.clone(), state.clone(), names.clone());
            remove.connect_clicked(move |_| {
                {
                    let mut st = state2.borrow_mut();
                    st.codes.remove(i);
                    st.variants.remove(i);
                    st.save();
                }
                rebuild_layout_rows(&group2, &rows2, &state2, &names2);
            });
            row.add_suffix(&remove);
        }
        group.add(&row);
        rows.borrow_mut().push(row);
    }
}

fn build_layouts_group() -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title("Keyboard layouts");
    group.set_description(Some("Switch between layouts with Meta+Alt+K"));

    let names = Rc::new(available_layouts());
    let state = Rc::new(RefCell::new(Layouts::load()));
    let rows = Rc::new(RefCell::new(Vec::new()));

    let add_model: Vec<String> = names.iter().map(|(c, d)| format!("{d} ({c})")).collect();
    let add_refs: Vec<&str> = add_model.iter().map(String::as_str).collect();
    let picker = gtk4::DropDown::from_strings(&add_refs);
    picker.set_enable_search(true);
    picker.set_valign(gtk4::Align::Center);
    let add_btn = gtk4::Button::with_label("Add");
    add_btn.set_valign(gtk4::Align::Center);

    let add_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    add_box.append(&picker);
    add_box.append(&add_btn);
    group.set_header_suffix(Some(&add_box));

    if names.is_empty() {
        add_box.set_sensitive(false);
    }

    {
        let (group, rows, state, names) = (group.clone(), rows.clone(), state.clone(), names.clone());
        add_btn.connect_clicked(move |_| {
            let Some((code, _)) = names.get(picker.selected() as usize) else { return };
            {
                let mut st = state.borrow_mut();
                if st.codes.contains(code) {
                    return;
                }
                st.codes.push(code.clone());
                st.variants.push(String::new());
                st.save();
            }
            rebuild_layout_rows(&group, &rows, &state, &names);
        });
    }

    rebuild_layout_rows(&group, &rows, &state, &names);
    group
}

fn build_typing_group() -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title("Typing");

    let repeat = adw::SwitchRow::new();
    repeat.set_title("Key repeat");
    repeat.set_subtitle("Repeat a character while its key is held down");
    repeat.set_active(kread("kcminputrc", "Keyboard", "KeyRepeat").as_deref() != Some("nothing"));

    let delay = adw::SpinRow::with_range(100.0, 2000.0, 50.0);
    delay.set_title("Repeat delay");
    delay.set_subtitle("Milliseconds before a held key starts repeating");
    delay.set_value(kread("kcminputrc", "Keyboard", "RepeatDelay").and_then(|v| v.parse().ok()).unwrap_or(600.0));

    let rate = adw::SpinRow::with_range(1.0, 100.0, 1.0);
    rate.set_title("Repeat rate");
    rate.set_subtitle("Characters per second while repeating");
    rate.set_digits(0);
    rate.set_value(kread("kcminputrc", "Keyboard", "RepeatRate").and_then(|v| v.parse().ok()).unwrap_or(25.0));

    delay.set_sensitive(repeat.is_active());
    rate.set_sensitive(repeat.is_active());

    {
        let (delay, rate) = (delay.clone(), rate.clone());
        repeat.connect_active_notify(move |r| {
            let on = r.is_active();
            delay.set_sensitive(on);
            rate.set_sensitive(on);
            write_and_reload(vec![("kcminputrc", "Keyboard", "KeyRepeat", if on { "repeat" } else { "nothing" }.into())]);
        });
    }
    delay.connect_value_notify(|s| {
        write_and_reload(vec![("kcminputrc", "Keyboard", "RepeatDelay", (s.value() as i64).to_string())]);
    });
    rate.connect_value_notify(|s| {
        write_and_reload(vec![("kcminputrc", "Keyboard", "RepeatRate", (s.value() as i64).to_string())]);
    });

    group.add(&repeat);
    group.add(&delay);
    group.add(&rate);

    let test = gtk4::Entry::builder().placeholder_text("Type here to test").build();
    let test_row = adw::ActionRow::new();
    test_row.set_title("Test area");
    test.set_valign(gtk4::Align::Center);
    test_row.add_suffix(&test);
    group.add(&test_row);
    group
}

fn build_shortcuts_group() -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::new();
    group.set_title("Shortcuts");
    let row = adw::ActionRow::new();
    row.set_title("Keyboard shortcuts");
    row.set_subtitle("View and change global shortcuts for the desktop and apps");
    row.add_prefix(&gtk4::Image::from_icon_name("preferences-desktop-keyboard-shortcuts-symbolic"));
    row.add_suffix(&gtk4::Image::from_icon_name("go-next-symbolic"));
    row.set_activatable(true);
    row.connect_activated(|r| super::shortcuts::open(r.upcast_ref()));
    group.add(&row);
    group
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
            .label("Keyboard")
            .halign(gtk4::Align::Start)
            .css_classes(vec!["win11-page-title".to_string()])
            .build(),
    );

    root.append(&build_shortcuts_group());
    root.append(&build_typing_group());
    root.append(&build_layouts_group());

    scroll.set_child(Some(&root));
    scroll.upcast()
}
