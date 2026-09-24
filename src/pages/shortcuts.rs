//! Global keyboard shortcuts editor, backed by KDE's kglobalaccel daemon over
//! D-Bus -- the same service Plasma's own Shortcuts settings talk to, so
//! changes apply immediately and persist in kglobalshortcutsrc.
//!
//! Keys travel as Qt "combined" ints (key code | modifier bits); a shortcut is
//! a list of key sequences, of which we use the first key of each.

use adw::prelude::*;
use gtk4::glib::translate::IntoGlib;
use gtk4::prelude::*;
use libadwaita as adw;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

const SERVICE: &str = "org.kde.kglobalaccel";
const PATH: &str = "/kglobalaccel";
const IFACE: &str = "org.kde.KGlobalAccel";

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

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
        .block_on(f)
}

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

/// Run `work` on a thread and hand its result to `done` on the GTK thread.
fn in_background<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
    done: impl FnOnce(T) + 'static,
) {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(work());
    });
    let mut done = Some(done);
    glib::timeout_add_local(Duration::from_millis(40), move || match rx.try_recv() {
        Ok(v) => {
            if let Some(d) = done.take() {
                d(v);
            }
            glib::ControlFlow::Break
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        Err(_) => glib::ControlFlow::Break,
    });
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
}

fn refresh_row(ui: &Ui, i: usize) {
    let actions = ui.actions.borrow();
    let labels = ui.labels.borrow();
    if let (Some(a), Some((label, reset))) = (actions.get(i), labels.get(i)) {
        label.set_label(&keys_text(&a.keys));
        reset.set_visible(a.keys != a.defaults);
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

fn populate(ui: &Rc<Ui>, list: &gtk4::Box, search: &gtk4::SearchEntry) {
    let actions = ui.actions.borrow().clone();
    let mut order: Vec<usize> = (0..actions.len()).collect();
    order.sort_by(|a, b| {
        let (x, y) = (&actions[*a], &actions[*b]);
        x.component().to_lowercase().cmp(&y.component().to_lowercase()).then(x.name().cmp(y.name()))
    });

    let mut labels = vec![(gtk4::Label::new(None), gtk4::Button::new()); actions.len()];
    let mut groups: Vec<(adw::PreferencesGroup, Vec<(adw::ActionRow, String)>)> = Vec::new();
    let mut current: Option<String> = None;

    for i in order {
        let a = &actions[i];
        if current.as_deref() != Some(a.component()) {
            current = Some(a.component().to_string());
            let g = adw::PreferencesGroup::new();
            g.set_title(&glib::markup_escape_text(a.component()));
            list.append(&g);
            groups.push((g, Vec::new()));
        }
        let row = adw::ActionRow::new();
        row.set_title(&glib::markup_escape_text(a.name()));
        row.set_activatable(true);

        let label = gtk4::Label::new(Some(keys_text(&a.keys).as_str()));
        label.set_css_classes(&["dim-label"]);
        let reset = gtk4::Button::from_icon_name("edit-undo-symbolic");
        reset.set_css_classes(&["flat"]);
        reset.set_valign(gtk4::Align::Center);
        reset.set_tooltip_text(Some(format!("Reset to default ({})", keys_text(&a.defaults)).as_str()));
        reset.set_visible(a.keys != a.defaults);
        row.add_suffix(&label);
        row.add_suffix(&reset);

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
        let haystack = format!("{} {} {}", a.name(), a.component(), keys_text(&a.keys)).to_lowercase();
        let (g, rows) = groups.last_mut().expect("group exists");
        g.add(&row);
        rows.push((row, haystack));
    }
    *ui.labels.borrow_mut() = labels;

    let groups = Rc::new(groups);
    search.connect_search_changed(move |s| {
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
}

pub fn open(parent: &gtk4::Widget) {
    let win = adw::Window::builder().default_width(760).default_height(680).title("Keyboard shortcuts").build();
    if let Some(p) = parent.root().and_downcast::<gtk4::Window>() {
        win.set_transient_for(Some(&p));
        win.set_modal(true);
    }

    let search = gtk4::SearchEntry::builder().placeholder_text("Search shortcuts").hexpand(true).build();
    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&search));

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

    let ui = Rc::new(Ui {
        actions: RefCell::new(Vec::new()),
        labels: RefCell::new(Vec::new()),
        window: win.clone().upcast(),
    });

    in_background(load_actions, move |res| match res {
        Ok(actions) if !actions.is_empty() => {
            *ui.actions.borrow_mut() = actions;
            populate(&ui, &list, &search);
            stack.set_visible_child_name("list");
        }
        Ok(_) => {
            status.set_title("No shortcuts found");
            status.set_description(Some("No application has registered a global shortcut."));
        }
        Err(e) => {
            status.set_icon_name(Some("dialog-warning-symbolic"));
            status.set_title("Shortcut service unavailable");
            let msg = format!(
                "Global shortcuts are managed by KDE's kglobalaccel, which only runs in a Plasma session.\n\n{e}"
            );
            status.set_description(Some(msg.as_str()));
        }
    });
}
