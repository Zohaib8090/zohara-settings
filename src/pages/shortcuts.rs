//! Global keyboard shortcuts editor, backed by KDE's kglobalaccel daemon over
//! D-Bus -- the same service Plasma's own Shortcuts settings talk to, so
//! changes apply immediately and persist in kglobalshortcutsrc.
//!
//! Keys travel as Qt "combined" ints (key code | modifier bits); a shortcut is
//! a list of key sequences, of which we use the first key of each.
//!
//! Custom shortcuts (a key that runs a command) are made the way Plasma makes them:
//! a `.desktop` file in `~/.local/share/applications` marked
//! `X-KDE-GlobalAccel-CommandShortcut=true`, registered with kglobalaccel as the
//! "services" component named after that file. Removing one unregisters it and
//! deletes the file.

use crate::backend::worker::{block_on, in_background};
use adw::prelude::*;
use gtk4::glib::translate::IntoGlib;
use gtk4::prelude::*;
use libadwaita as adw;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const SERVICE: &str = "org.kde.kglobalaccel";
const PATH: &str = "/kglobalaccel";
const IFACE: &str = "org.kde.KGlobalAccel";

/// Desktop-file ids of the shortcuts made on this page start with this.
const CUSTOM_PREFIX: &str = "net.zohara.customshortcut.";
const CUSTOM_GROUP: &str = "Custom shortcuts";
const LAUNCH_ACTION: &str = "_launch";

const QT_SHIFT: i32 = 0x0200_0000;
const QT_CTRL: i32 = 0x0400_0000;
const QT_ALT: i32 = 0x0800_0000;
const QT_META: i32 = 0x1000_0000;
const QT_KEY_MASK: i32 = 0x01FF_FFFF;

/// (X11 keysym, Qt key code, display name)
const SPECIAL_KEYS: &[(u32, i32, &str)] = &[
    (0xff1b, 0x0100_0000, "Esc"),
    (0xff09, 0x0100_0001, "Tab"),
    (0xff08, 0x0100_0003, "Backspace"),
    (0xff0d, 0x0100_0004, "Return"),
    (0xff8d, 0x0100_0005, "Enter"),
    (0xff63, 0x0100_0006, "Ins"),
    (0xffff, 0x0100_0007, "Del"),
    (0xff13, 0x0100_0008, "Pause"),
    (0xff61, 0x0100_0009, "Print"),
    (0xff50, 0x0100_0010, "Home"),
    (0xff57, 0x0100_0011, "End"),
    (0xff51, 0x0100_0012, "Left"),
    (0xff52, 0x0100_0013, "Up"),
    (0xff53, 0x0100_0014, "Right"),
    (0xff54, 0x0100_0015, "Down"),
    (0xff55, 0x0100_0016, "PgUp"),
    (0xff56, 0x0100_0017, "PgDown"),
    (0x0020, 0x0000_0020, "Space"),
    (0x1008ff11, 0x0100_0070, "Volume Down"),
    (0x1008ff12, 0x0100_0071, "Volume Mute"),
    (0x1008ff13, 0x0100_0072, "Volume Up"),
    (0x1008ff14, 0x0100_0080, "Media Play"),
    (0x1008ff15, 0x0100_0081, "Media Stop"),
    (0x1008ff16, 0x0100_0082, "Media Previous"),
    (0x1008ff17, 0x0100_0083, "Media Next"),
    (0x1008ff02, 0x0100_00f2, "Brightness Up"),
    (0x1008ff03, 0x0100_00f3, "Brightness Down"),
];

const MODIFIER_KEYSYMS: &[u32] = &[
    0xffe1, 0xffe2, 0xffe3, 0xffe4, 0xffe5, 0xffe7, 0xffe8, 0xffe9, 0xffea, 0xffeb, 0xffec, 0xfe03,
];

const KEYSYM_F1: u32 = 0xffbe;
const KEYSYM_F35: u32 = 0xffe0;
const QT_F1: i32 = 0x0100_0030;

#[derive(Clone)]
struct Action {
    /// [componentUnique, actionUnique, componentFriendly, actionFriendly]
    id: Vec<String>,
    keys: Vec<i32>,
    defaults: Vec<i32>,
}

impl Action {
    fn component(&self) -> &str {
        self.id.get(2).filter(|s| !s.is_empty()).or(self.id.first()).map(String::as_str).unwrap_or("")
    }
    fn name(&self) -> &str {
        self.id.get(3).filter(|s| !s.is_empty()).or(self.id.get(1)).map(String::as_str).unwrap_or("")
    }
    fn is_custom(&self) -> bool {
        self.id.first().is_some_and(|c| c.starts_with(CUSTOM_PREFIX))
    }
    /// Heading of the group this action is listed under.
    fn group(&self) -> &str {
        if self.is_custom() {
            CUSTOM_GROUP
        } else {
            self.component()
        }
    }
}

pub fn key_text(k: i32) -> String {
    let mut s = String::new();
    for (bit, name) in [(QT_META, "Meta+"), (QT_CTRL, "Ctrl+"), (QT_ALT, "Alt+"), (QT_SHIFT, "Shift+")] {
        if k & bit != 0 {
            s.push_str(name);
        }
    }
    let code = k & QT_KEY_MASK;
    if let Some((_, _, name)) = SPECIAL_KEYS.iter().find(|(_, q, _)| *q == code) {
        s.push_str(name);
    } else if (QT_F1..QT_F1 + 35).contains(&code) {
        s.push_str(&format!("F{}", code - QT_F1 + 1));
    } else if let Some(c) = char::from_u32(code as u32).filter(|c| c.is_ascii_graphic()) {
        s.push(c);
    } else {
        s.push_str(&format!("0x{code:x}"));
    }
    s
}

fn keys_text(keys: &[i32]) -> String {
    if keys.is_empty() {
        "None".into()
    } else {
        keys.iter().map(|k| key_text(*k)).collect::<Vec<_>>().join(", ")
    }
}

/// GDK key event -> Qt combined key. None for bare modifier presses.
fn gdk_to_qt(keyval: gtk4::gdk::Key, state: gtk4::gdk::ModifierType) -> Option<i32> {
    use gtk4::gdk::ModifierType as M;
    let sym = keyval.into_glib();
    if MODIFIER_KEYSYMS.contains(&sym) {
        return None;
    }
    let code = if let Some((_, q, _)) = SPECIAL_KEYS.iter().find(|(s, _, _)| *s == sym) {
        *q
    } else if (KEYSYM_F1..=KEYSYM_F35).contains(&sym) {
        QT_F1 + (sym - KEYSYM_F1) as i32
    } else {
        let c = keyval.to_upper().to_unicode().filter(|c| c.is_ascii_graphic())?;
        c as i32
    };
    let mut mods = 0;
    if state.contains(M::SUPER_MASK) || state.contains(M::META_MASK) {
        mods |= QT_META;
    }
    if state.contains(M::CONTROL_MASK) {
        mods |= QT_CTRL;
    }
    if state.contains(M::ALT_MASK) {
        mods |= QT_ALT;
    }
    if state.contains(M::SHIFT_MASK) {
        mods |= QT_SHIFT;
    }
    Some(code | mods)
}

// ── D-Bus (runs on worker threads) ──────────────────────────────────────────

async fn call<B, R>(conn: &zbus::Connection, method: &str, body: &B) -> zbus::Result<R>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
    R: for<'d> serde::Deserialize<'d> + zbus::zvariant::Type,
{
    let msg = conn.call_method(Some(SERVICE), PATH, Some(IFACE), method, body).await?;
    msg.body().deserialize::<R>()
}

fn first_keys(seqs: Vec<(Vec<i32>,)>) -> Vec<i32> {
    seqs.into_iter().filter_map(|(s,)| s.into_iter().find(|k| *k != 0)).collect()
}

async fn read_keys(conn: &zbus::Connection, id: &Vec<String>, default: bool) -> zbus::Result<Vec<i32>> {
    let method = if default { "defaultShortcutKeys" } else { "shortcutKeys" };
    let seqs: Vec<(Vec<i32>,)> = call(conn, method, id).await?;
    Ok(first_keys(seqs))
}

fn load_actions() -> Result<Vec<Action>, String> {
    block_on(async {
        let conn = zbus::Connection::session().await.map_err(|e| e.to_string())?;
        let comps: Vec<Vec<String>> = call(&conn, "allMainComponents", &()).await.map_err(|e| e.to_string())?;
        let mut actions = Vec::new();
        for comp in comps {
            let ids: Vec<Vec<String>> = match call(&conn, "allActionsForComponent", &comp).await {
                Ok(v) => v,
                Err(_) => continue,
            };
            for id in ids {
                let keys = read_keys(&conn, &id, false).await.unwrap_or_default();
                let defaults = read_keys(&conn, &id, true).await.unwrap_or_default();
                actions.push(Action { id, keys, defaults });
            }
        }
        Ok(actions)
    })
}

/// Sets the action's shortcut and returns what kglobalaccel actually stored.
fn store_keys(id: Vec<String>, keys: Vec<i32>) -> Result<Vec<i32>, String> {
    block_on(async {
        let conn = zbus::Connection::session().await.map_err(|e| e.to_string())?;
        let seqs: Vec<(Vec<i32>,)> = keys.iter().map(|k| (vec![*k, 0, 0, 0],)).collect();
        let msg = conn
            .call_method(Some(SERVICE), PATH, Some(IFACE), "setForeignShortcutKeys", &(id.clone(), seqs))
            .await
            .map_err(|e| e.to_string())?;
        drop(msg);
        read_keys(&conn, &id, false).await.map_err(|e| e.to_string())
    })
}

fn block_global_shortcuts(block: bool) {
    std::thread::spawn(move || {
        let _ = block_on(async {
            let conn = zbus::Connection::session().await?;
            conn.call_method(Some(SERVICE), PATH, Some(IFACE), "blockGlobalShortcuts", &block).await
        });
    });
}

// ── Custom shortcuts (files + kglobalaccel) ─────────────────────────────────

fn applications_dir() -> std::path::PathBuf {
    let base = std::env::var("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share"));
    base.join("applications")
}

fn custom_file(desktop_id: &str) -> std::path::PathBuf {
    applications_dir().join(desktop_id)
}

/// A desktop-file id that is unique and safe as a file name: prefix, readable part of the name, time.
fn custom_id(name: &str, stamp: u128) -> String {
    let slug: String = name
        .chars()
        .filter_map(|c| if c.is_ascii_alphanumeric() { Some(c.to_ascii_lowercase()) } else if c == ' ' || c == '-' || c == '_' { Some('-') } else { None })
        .take(24)
        .collect();
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        format!("{CUSTOM_PREFIX}{stamp:x}.desktop")
    } else {
        format!("{CUSTOM_PREFIX}{slug}-{stamp:x}.desktop")
    }
}

/// Desktop-entry string value: backslashes doubled (newlines are refused before this).
fn entry_value(s: &str) -> String {
    s.replace('\\', "\\\\")
}

fn entry_unvalue(s: &str) -> String {
    s.replace("\\\\", "\\")
}

/// `Exec` that runs `command` through the shell. Plasma refuses shell syntax (`&&`, `|`, `~`) in a
/// bare Exec, so the command is one double-quoted argument of `sh -c`. Per the Desktop Entry spec the
/// quoted argument escapes `"` `` ` `` `$` `\` with a backslash, the string layer doubles every
/// backslash again, and a literal `%` is written `%%`.
fn exec_line(command: &str) -> String {
    let mut q = String::new();
    for c in command.chars() {
        match c {
            '"' | '`' | '$' => {
                q.push_str("\\\\");
                q.push(c);
            }
            '\\' => q.push_str("\\\\\\\\"),
            '%' => q.push_str("%%"),
            _ => q.push(c),
        }
    }
    format!("sh -c \"{q}\"")
}

fn desktop_entry(name: &str, command: &str) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName={}\nComment={}\nExec={}\nNoDisplay=true\nStartupNotify=false\nX-KDE-GlobalAccel-CommandShortcut=true\n",
        entry_value(name),
        entry_value(command),
        exec_line(command)
    )
}

/// The command a custom shortcut runs, as the user typed it (kept in the file's Comment).
fn custom_command(desktop_id: &str) -> Option<String> {
    let text = std::fs::read_to_string(custom_file(desktop_id)).ok()?;
    text.lines().find_map(|l| l.strip_prefix("Comment=")).map(entry_unvalue)
}

fn one_line(what: &str, s: &str) -> Result<String, String> {
    let s = s.trim();
    if s.is_empty() {
        Err(format!("{what} is empty."))
    } else if s.contains(['\n', '\r']) {
        Err(format!("{what} must be on one line."))
    } else {
        Ok(s.to_string())
    }
}

fn custom_stamp() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

/// Writes the desktop file and has kglobalaccel take the key for it.
fn create_custom(name: String, command: String, key: i32) -> Result<(), String> {
    let name = one_line("The name", &name)?;
    let command = one_line("The command", &command)?;
    let desktop_id = custom_id(&name, custom_stamp());
    let path = custom_file(&desktop_id);
    std::fs::create_dir_all(applications_dir()).map_err(|e| e.to_string())?;
    std::fs::write(&path, desktop_entry(&name, &command)).map_err(|e| format!("Couldn't save the shortcut: {e}"))?;

    let id = vec![desktop_id.clone(), LAUNCH_ACTION.to_string(), desktop_id.clone(), name];
    let registered = block_on(async {
        let conn = zbus::Connection::session().await.map_err(|e| e.to_string())?;
        // SetPresent | NoAutoloading: a brand-new action, take exactly this key. kglobalaccel only builds the
        // component once it can find the new desktop file, which can lag a second or two behind the write
        // (more when shortcuts were just added or removed), so register again until the key is taken.
        for attempt in 0..30 {
            conn.call_method(Some(SERVICE), PATH, Some(IFACE), "doRegister", &id).await.map_err(|e| e.to_string())?;
            let stored: Vec<i32> =
                call(&conn, "setShortcut", &(id.clone(), vec![key], 6u32)).await.map_err(|e| e.to_string())?;
            if stored.contains(&key) {
                return Ok(());
            }
            if attempt < 29 {
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        }
        Err("Plasma didn't accept that key. It may already be in use; try a different one.".to_string())
    });
    if registered.is_err() {
        let _ = std::fs::remove_file(&path);
        let _ = remove_registration(&desktop_id);
    }
    registered
}

fn remove_registration(desktop_id: &str) -> Result<(), String> {
    block_on(async {
        let conn = zbus::Connection::session().await.map_err(|e| e.to_string())?;
        conn.call_method(Some(SERVICE), PATH, Some(IFACE), "unregister", &(desktop_id, LAUNCH_ACTION))
            .await
            .map(drop)
            .map_err(|e| e.to_string())
    })
}

fn delete_custom(desktop_id: String) -> Result<(), String> {
    // Only ever our own files.
    if !desktop_id.starts_with(CUSTOM_PREFIX) || desktop_id.contains('/') {
        return Err("Not a custom shortcut.".into());
    }
    remove_registration(&desktop_id)?;
    match std::fs::remove_file(custom_file(&desktop_id)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

// ── UI ──────────────────────────────────────────────────────────────────────

enum Captured {
    Set(i32),
    Clear,
}

fn capture_shortcut(parent: &gtk4::Window, action_name: &str, on_done: impl Fn(Captured) + 'static) {
    let win = adw::Window::builder()
        .modal(true)
        .transient_for(parent)
        .default_width(420)
        .resizable(false)
        .title("Set shortcut")
        .build();

    let body = gtk4::Box::new(gtk4::Orientation::Vertical, 10);
    body.set_margin_top(28);
    body.set_margin_bottom(28);
    body.set_margin_start(24);
    body.set_margin_end(24);
    body.append(
        &gtk4::Label::builder()
            .label(format!("Press the new shortcut for “{action_name}”"))
            .wrap(true)
            .css_classes(vec!["heading".to_string()])
            .build(),
    );
    body.append(
        &gtk4::Label::builder()
            .label("Esc to cancel · Backspace to remove the shortcut")
            .css_classes(vec!["dim-label".to_string()])
            .build(),
    );
    win.set_content(Some(&body));

    // Otherwise pressing an existing global shortcut fires it instead of reaching us.
    block_global_shortcuts(true);
    win.connect_destroy(|_| block_global_shortcuts(false));

    let on_done = Rc::new(on_done);
    let keys = gtk4::EventControllerKey::new();
    {
        let win = win.clone();
        keys.connect_key_pressed(move |_, keyval, _, state| {
            use gtk4::gdk::ModifierType as M;
            let plain = (state & (M::SHIFT_MASK | M::CONTROL_MASK | M::ALT_MASK | M::SUPER_MASK | M::META_MASK))
                .is_empty();
            let sym = keyval.into_glib();
            if plain && sym == 0xff1b {
                win.close();
            } else if plain && sym == 0xff08 {
                on_done(Captured::Clear);
                win.close();
            } else if let Some(k) = gdk_to_qt(keyval, state) {
                on_done(Captured::Set(k));
                win.close();
            }
            glib::Propagation::Stop
        });
    }
    win.add_controller(keys);
    win.present();
}

struct Ui {
    actions: RefCell<Vec<Action>>,
    labels: RefCell<Vec<(gtk4::Label, gtk4::Button)>>,
    window: gtk4::Window,
    /// Search-box handler of the current list, dropped when the list is rebuilt.
    search_handler: RefCell<Option<glib::SignalHandlerId>>,
}

/// The widgets that make up the open Shortcuts window, so the list can be rebuilt after a change.
struct Page {
    ui: Rc<Ui>,
    list: gtk4::Box,
    search: gtk4::SearchEntry,
    stack: gtk4::Stack,
    status: adw::StatusPage,
    add_button: gtk4::Button,
}

fn refresh_row(ui: &Ui, i: usize) {
    let actions = ui.actions.borrow();
    let labels = ui.labels.borrow();
    if let (Some(a), Some((label, reset))) = (actions.get(i), labels.get(i)) {
        label.set_label(&keys_text(&a.keys));
        reset.set_visible(a.keys != a.defaults && !a.is_custom());
    }
}

fn apply(ui: &Rc<Ui>, i: usize, keys: Vec<i32>) {
    let id = ui.actions.borrow()[i].id.clone();
    let ui = ui.clone();
    in_background(
        move || store_keys(id, keys),
        move |res| match res {
            Ok(stored) => {
                ui.actions.borrow_mut()[i].keys = stored;
                refresh_row(&ui, i);
            }
            Err(e) => {
                let d = adw::AlertDialog::new(Some("Couldn't change the shortcut"), Some(e.as_str()));
                d.add_response("ok", "OK");
                d.present(Some(&ui.window));
            }
        },
    );
}

fn edit(ui: &Rc<Ui>, i: usize) {
    let name = ui.actions.borrow()[i].name().to_string();
    let ui2 = ui.clone();
    capture_shortcut(&ui.window, &name, move |cap| {
        let key = match cap {
            Captured::Clear => return apply(&ui2, i, Vec::new()),
            Captured::Set(k) => k,
        };
        let conflict = ui2
            .actions
            .borrow()
            .iter()
            .enumerate()
            .find(|(j, a)| *j != i && a.keys.contains(&key))
            .map(|(j, a)| (j, a.name().to_string(), a.component().to_string()));
        match conflict {
            None => apply(&ui2, i, vec![key]),
            Some((j, other, comp)) => {
                let d = adw::AlertDialog::new(
                    Some("Shortcut already in use"),
                    Some(format!("{} is used by “{other}” ({comp}). Reassign it?", key_text(key)).as_str()),
                );
                d.add_responses(&[("cancel", "Cancel"), ("reassign", "Reassign")]);
                d.set_response_appearance("reassign", adw::ResponseAppearance::Destructive);
                let ui3 = ui2.clone();
                d.connect_response(None, move |_, resp| {
                    if resp == "reassign" {
                        let remaining: Vec<i32> =
                            ui3.actions.borrow()[j].keys.iter().copied().filter(|k| *k != key).collect();
                        apply(&ui3, j, remaining);
                        apply(&ui3, i, vec![key]);
                    }
                });
                d.present(Some(&ui2.window));
            }
        }
    });
}

fn show_error(window: &gtk4::Window, title: &str, message: &str) {
    let d = adw::AlertDialog::new(Some(title), Some(message));
    d.add_response("ok", "OK");
    d.present(Some(window));
}

fn populate(page: &Rc<Page>) {
    let ui = &page.ui;
    let actions = ui.actions.borrow().clone();
    let mut order: Vec<usize> = (0..actions.len()).collect();
    // Custom shortcuts first, then the rest by program name.
    order.sort_by(|a, b| {
        let (x, y) = (&actions[*a], &actions[*b]);
        y.is_custom()
            .cmp(&x.is_custom())
            .then(x.group().to_lowercase().cmp(&y.group().to_lowercase()))
            .then(x.name().cmp(y.name()))
    });

    let mut labels = vec![(gtk4::Label::new(None), gtk4::Button::new()); actions.len()];
    let mut groups: Vec<(adw::PreferencesGroup, Vec<(adw::ActionRow, String)>)> = Vec::new();
    let mut current: Option<String> = None;

    for i in order {
        let a = &actions[i];
        if current.as_deref() != Some(a.group()) {
            current = Some(a.group().to_string());
            let g = adw::PreferencesGroup::new();
            g.set_title(&glib::markup_escape_text(a.group()));
            page.list.append(&g);
            groups.push((g, Vec::new()));
        }
        let row = adw::ActionRow::new();
        row.set_title(&glib::markup_escape_text(a.name()));
        row.set_activatable(true);

        let command = if a.is_custom() { a.id.first().and_then(|id| custom_command(id)) } else { None };
        if let Some(c) = &command {
            row.set_subtitle(&glib::markup_escape_text(c));
            row.set_subtitle_lines(1);
        }

        let label = gtk4::Label::new(Some(keys_text(&a.keys).as_str()));
        label.set_css_classes(&["dim-label"]);
        let reset = gtk4::Button::from_icon_name("edit-undo-symbolic");
        reset.set_css_classes(&["flat"]);
        reset.set_valign(gtk4::Align::Center);
        reset.set_tooltip_text(Some(format!("Reset to default ({})", keys_text(&a.defaults)).as_str()));
        reset.set_visible(a.keys != a.defaults && !a.is_custom());
        row.add_suffix(&label);
        row.add_suffix(&reset);

        if a.is_custom() {
            let del = gtk4::Button::from_icon_name("user-trash-symbolic");
            del.set_css_classes(&["flat"]);
            del.set_valign(gtk4::Align::Center);
            del.set_tooltip_text(Some("Delete this shortcut"));
            let page = page.clone();
            del.connect_clicked(move |_| confirm_delete(&page, i));
            row.add_suffix(&del);
        }

        {
            let ui = ui.clone();
            row.connect_activated(move |_| edit(&ui, i));
        }
        {
            let ui = ui.clone();
            reset.connect_clicked(move |_| {
                let defaults = ui.actions.borrow()[i].defaults.clone();
                apply(&ui, i, defaults);
            });
        }

        labels[i] = (label, reset);
        let haystack =
            format!("{} {} {} {}", a.name(), a.group(), keys_text(&a.keys), command.unwrap_or_default()).to_lowercase();
        let (g, rows) = groups.last_mut().expect("group exists");
        g.add(&row);
        rows.push((row, haystack));
    }
    *ui.labels.borrow_mut() = labels;

    let groups = Rc::new(groups);
    if let Some(old) = ui.search_handler.borrow_mut().take() {
        page.search.disconnect(old);
    }
    let handler = page.search.connect_search_changed(move |s| {
        let q = s.text().to_lowercase();
        for (g, rows) in groups.iter() {
            let mut any = false;
            for (row, hay) in rows {
                let show = q.is_empty() || hay.contains(q.as_str());
                row.set_visible(show);
                any |= show;
            }
            g.set_visible(any);
        }
    });
    *ui.search_handler.borrow_mut() = Some(handler);
}

/// (Re)reads every shortcut from kglobalaccel and rebuilds the list.
fn reload(page: &Rc<Page>) {
    page.search.set_text("");
    let page = page.clone();
    in_background(load_actions, move |res| {
        while let Some(child) = page.list.first_child() {
            page.list.remove(&child);
        }
        match res {
            Ok(actions) if !actions.is_empty() => {
                *page.ui.actions.borrow_mut() = actions;
                populate(&page);
                page.add_button.set_sensitive(true);
                page.stack.set_visible_child_name("list");
            }
            Ok(_) => {
                page.status.set_title("No shortcuts found");
                page.status.set_description(Some("No application has registered a global shortcut."));
                page.stack.set_visible_child_name("status");
            }
            Err(e) => {
                page.status.set_icon_name(Some("dialog-warning-symbolic"));
                page.status.set_title("Shortcut service unavailable");
                let msg = format!(
                    "Global shortcuts are managed by KDE's kglobalaccel, which only runs in a Plasma session.\n\n{e}"
                );
                page.status.set_description(Some(msg.as_str()));
                page.stack.set_visible_child_name("status");
            }
        }
    });
}

fn confirm_delete(page: &Rc<Page>, i: usize) {
    let (id, name, keys) = {
        let actions = page.ui.actions.borrow();
        let a = &actions[i];
        (a.id.first().cloned().unwrap_or_default(), a.name().to_string(), keys_text(&a.keys))
    };
    let d = adw::AlertDialog::new(
        Some(format!("Delete “{name}”?").as_str()),
        Some(format!("{keys} will stop doing anything. The program it starts isn't touched.").as_str()),
    );
    d.add_responses(&[("cancel", "Cancel"), ("delete", "Delete")]);
    d.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    let window = page.ui.window.clone();
    let page = page.clone();
    d.connect_response(None, move |_, resp| {
        if resp != "delete" {
            return;
        }
        let id = id.clone();
        let page = page.clone();
        in_background(move || delete_custom(id), move |res| match res {
            Ok(()) => reload(&page),
            Err(e) => show_error(&page.ui.window, "Couldn't delete the shortcut", &e),
        });
    });
    d.present(Some(&window));
}

/// "Add a custom shortcut": a name, a command, and a key to press.
fn add_custom(page: &Rc<Page>) {
    let win = adw::Window::builder()
        .modal(true)
        .transient_for(&page.ui.window)
        .default_width(460)
        .resizable(false)
        .title("Add a custom shortcut")
        .build();

    let name = adw::EntryRow::builder().title("Name").build();
    let command = adw::EntryRow::builder().title("Command").build();
    let key_label = gtk4::Label::new(Some("Not set"));
    key_label.set_css_classes(&["dim-label"]);
    let key_button = gtk4::Button::with_label("Set shortcut");
    key_button.set_valign(gtk4::Align::Center);
    let key_row = adw::ActionRow::builder().title("Shortcut").build();
    key_row.add_suffix(&key_label);
    key_row.add_suffix(&key_button);

    let group = adw::PreferencesGroup::new();
    group.add(&name);
    group.add(&command);
    group.add(&key_row);

    let hint = gtk4::Label::builder()
        .label("The command runs like it would in a terminal, for example “firefox https://example.org” or “systemctl --user restart plasma-plasmashell”.")
        .wrap(true)
        .xalign(0.0)
        .css_classes(vec!["dim-label".to_string(), "caption".to_string()])
        .build();
    let error = gtk4::Label::builder()
        .wrap(true)
        .xalign(0.0)
        .visible(false)
        .css_classes(vec!["error".to_string()])
        .build();

    let body = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    body.set_margin_top(12);
    body.set_margin_bottom(24);
    body.set_margin_start(24);
    body.set_margin_end(24);
    body.append(&group);
    body.append(&hint);
    body.append(&error);

    let cancel = gtk4::Button::with_label("Cancel");
    let add = gtk4::Button::with_label("Add");
    add.set_css_classes(&["suggested-action"]);
    add.set_sensitive(false);
    let header = adw::HeaderBar::new();
    header.pack_start(&cancel);
    header.pack_end(&add);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&body));
    win.set_content(Some(&view));

    let key: Rc<Cell<Option<i32>>> = Rc::new(Cell::new(None));
    let check = {
        let (name, command, key, add) = (name.clone(), command.clone(), key.clone(), add.clone());
        Rc::new(move || {
            add.set_sensitive(
                !name.text().trim().is_empty() && !command.text().trim().is_empty() && key.get().is_some(),
            );
        })
    };
    {
        let check = check.clone();
        name.connect_changed(move |_| check());
    }
    {
        let check = check.clone();
        command.connect_changed(move |_| check());
    }
    {
        let win = win.clone();
        cancel.connect_clicked(move |_| win.close());
    }
    {
        let (win, key, key_label, check, name) = (win.clone(), key.clone(), key_label.clone(), check.clone(), name.clone());
        key_button.connect_clicked(move |_| {
            let (key, key_label, check) = (key.clone(), key_label.clone(), check.clone());
            let what = {
                let n = name.text();
                if n.trim().is_empty() { "this shortcut".to_string() } else { n.trim().to_string() }
            };
            capture_shortcut(win.upcast_ref::<gtk4::Window>(), &what, move |cap| {
                match cap {
                    Captured::Set(k) => {
                        key.set(Some(k));
                        key_label.set_label(&key_text(k));
                    }
                    Captured::Clear => {
                        key.set(None);
                        key_label.set_label("Not set");
                    }
                }
                check();
            });
        });
    }
    {
        let (win, page, name, command, key, error, add2) =
            (win.clone(), page.clone(), name.clone(), command.clone(), key.clone(), error.clone(), add.clone());
        add.connect_clicked(move |_| {
            let Some(k) = key.get() else { return };
            let (n, c) = (name.text().to_string(), command.text().to_string());
            let taken = page
                .ui
                .actions
                .borrow()
                .iter()
                .find(|a| a.keys.contains(&k))
                .map(|a| (a.name().to_string(), a.group().to_string()));
            if let Some((other, group)) = taken {
                error.set_label(&format!("{} is already used by “{other}” ({group}). Pick another key.", key_text(k)));
                error.set_visible(true);
                return;
            }
            error.set_visible(false);
            add2.set_sensitive(false);
            let (win, page, error, add2) = (win.clone(), page.clone(), error.clone(), add2.clone());
            in_background(move || create_custom(n, c, k), move |res| match res {
                Ok(()) => {
                    win.close();
                    reload(&page);
                }
                Err(e) => {
                    error.set_label(&e);
                    error.set_visible(true);
                    add2.set_sensitive(true);
                }
            });
        });
    }
    win.present();
}

pub fn open(parent: &gtk4::Widget) {
    let win = adw::Window::builder().default_width(760).default_height(680).title("Keyboard shortcuts").build();
    if let Some(p) = parent.root().and_downcast::<gtk4::Window>() {
        win.set_transient_for(Some(&p));
        win.set_modal(true);
    }

    let search = gtk4::SearchEntry::builder().placeholder_text("Search shortcuts").hexpand(true).build();
    let add_button = gtk4::Button::from_icon_name("list-add-symbolic");
    add_button.set_tooltip_text(Some("Add a custom shortcut"));
    add_button.set_sensitive(false);
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&search));
    header.pack_start(&add_button);

    let list = gtk4::Box::new(gtk4::Orientation::Vertical, 18);
    list.set_margin_top(12);
    list.set_margin_bottom(24);
    list.set_margin_start(24);
    list.set_margin_end(24);

    let status = adw::StatusPage::builder().title("Loading shortcuts…").build();
    let stack = gtk4::Stack::new();
    stack.add_named(&status, Some("status"));
    let scroll = gtk4::ScrolledWindow::builder().hscrollbar_policy(gtk4::PolicyType::Never).vexpand(true).build();
    let clamp = adw::Clamp::builder().maximum_size(720).child(&list).build();
    scroll.set_child(Some(&clamp));
    stack.add_named(&scroll, Some("list"));

    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&stack));
    win.set_content(Some(&view));
    win.present();

    let page = Rc::new(Page {
        ui: Rc::new(Ui {
            actions: RefCell::new(Vec::new()),
            labels: RefCell::new(Vec::new()),
            window: win.clone().upcast(),
            search_handler: RefCell::new(None),
        }),
        list,
        search,
        stack,
        status,
        add_button: add_button.clone(),
    });
    {
        let page = page.clone();
        add_button.connect_clicked(move |_| add_custom(&page));
    }
    reload(&page);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_runs_the_command_through_the_shell() {
        assert_eq!(exec_line("echo hi"), r#"sh -c "echo hi""#);
        assert_eq!(exec_line("a && b | c"), r#"sh -c "a && b | c""#);
    }

    #[test]
    fn exec_escapes_quote_dollar_backtick_backslash_and_percent() {
        assert_eq!(exec_line(r#"say "hi""#), r#"sh -c "say \\"hi\\"""#);
        assert_eq!(exec_line("echo $HOME"), r#"sh -c "echo \\$HOME""#);
        assert_eq!(exec_line("echo `id`"), r#"sh -c "echo \\`id\\`""#);
        assert_eq!(exec_line(r"a\b"), r#"sh -c "a\\\\b""#);
        assert_eq!(exec_line("100%"), r#"sh -c "100%%""#);
    }

    #[test]
    fn desktop_entry_is_a_command_shortcut_and_keeps_the_typed_command() {
        let e = desktop_entry("My app", "firefox https://example.org");
        assert!(e.contains("X-KDE-GlobalAccel-CommandShortcut=true\n"));
        assert!(e.contains("Name=My app\n"));
        assert!(e.contains("Comment=firefox https://example.org\n"));
        assert!(e.contains("NoDisplay=true\n"));
        assert_eq!(entry_unvalue(&entry_value(r"a\b")), r"a\b");
    }

    #[test]
    fn ids_are_safe_file_names_with_the_custom_prefix() {
        let id = custom_id("../My Cool/App!", 0xabc);
        assert_eq!(id, format!("{CUSTOM_PREFIX}my-coolapp-abc.desktop"));
        assert!(!id.contains('/'));
        assert_eq!(custom_id("???", 1), format!("{CUSTOM_PREFIX}1.desktop"));
    }

    #[test]
    fn name_and_command_must_be_one_non_empty_line() {
        assert!(one_line("The name", "  ").is_err());
        assert!(one_line("The command", "a\nb").is_err());
        assert_eq!(one_line("The command", "  ls  ").unwrap(), "ls");
    }

    #[test]
    fn only_our_own_files_can_be_deleted() {
        assert!(delete_custom("org.kde.dolphin.desktop".into()).is_err());
        assert!(delete_custom(format!("{CUSTOM_PREFIX}../x.desktop")).is_err());
    }

    /// Needs a running Plasma session (kglobalaccel). `cargo test -- --ignored live_custom_shortcut`
    #[test]
    #[ignore]
    fn live_custom_shortcut_runs_a_shell_command_and_cleans_up() {
        let (a, b) = ("/tmp/zs-live-a", "/tmp/zs live b");
        let _ = std::fs::remove_file(a);
        let _ = std::fs::remove_file(b);
        // Ctrl+Alt+Shift+F11: unlikely to be taken.
        let key = 0x0100_003a | QT_CTRL | QT_ALT | QT_SHIFT;
        let command = format!(r#"touch {a} && touch "{b}" && echo 100% $HOME >/dev/null"#);
        if let Err(e) = create_custom("Live test".into(), command, key) { panic!("create failed: {e}"); }

        let actions = load_actions().expect("load");
        let found = actions.iter().find(|x| x.is_custom() && x.name() == "Live test").expect("listed");
        assert_eq!(found.keys, vec![key]);
        let id = found.id[0].clone();
        assert!(custom_command(&id).unwrap().contains("touch /tmp/zs-live-a"));

        let path = format!("/component/{}", id.replace(['.', '-'], "_"));
        let status = std::process::Command::new("busctl")
            .args(["--user", "call", SERVICE, &path, "org.kde.kglobalaccel.Component", "invokeShortcut", "s", LAUNCH_ACTION])
            .status()
            .unwrap();
        assert!(status.success());
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let ran = (std::path::Path::new(a).exists(), std::path::Path::new(b).exists());

        delete_custom(id.clone()).expect("delete");
        let _ = std::fs::remove_file(a);
        let _ = std::fs::remove_file(b);
        assert_eq!(ran, (true, true), "the command did not run");
        assert!(!custom_file(&id).exists());
        assert!(!load_actions().unwrap().iter().any(|x| x.id[0] == id), "still registered");
    }
}
