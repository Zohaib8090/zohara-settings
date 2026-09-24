//! Accounts: the signed-in user's real name, picture and password; sign-in
//! and lock-screen options; and managing other users.
//!
//! User data comes from AccountsService (org.freedesktop.Accounts on the
//! system bus). Changes to other users, passwords and automatic sign-in run
//! through pkexec, so Plasma's normal admin prompt appears. Passwords are
//! passed to chpasswd on stdin, never on a command line.

use crate::backend::kconfig;
use crate::backend::worker::{block_on, in_background};
use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use std::io::Write;
use std::process::{Command, Stdio};

const ACCOUNTS: &str = "org.freedesktop.Accounts";
const USER_IFACE: &str = "org.freedesktop.Accounts.User";
const AUTOLOGIN_CONF: &str = "/etc/sddm.conf.d/zohara-autologin.conf";

#[derive(Clone, Default)]
struct User {
    path: String,
    name: String,
    real_name: String,
    icon: String,
    admin: bool,
    uid: u64,
}

async fn read_user(conn: &zbus::Connection, path: String) -> zbus::Result<User> {
    let p = zbus::Proxy::new(conn, ACCOUNTS, path.clone(), USER_IFACE).await?;
    Ok(User {
        name: p.get_property("UserName").await?,
        real_name: p.get_property("RealName").await.unwrap_or_default(),
        icon: p.get_property("IconFile").await.unwrap_or_default(),
        admin: p.get_property::<i32>("AccountType").await.unwrap_or(0) == 1,
        uid: p.get_property("Uid").await.unwrap_or(0),
        path,
    })
}

fn load_users() -> Result<Vec<User>, String> {
    block_on(async {
        let conn = zbus::Connection::system().await.map_err(|e| e.to_string())?;
        let reply = conn
            .call_method(Some(ACCOUNTS), "/org/freedesktop/Accounts", Some(ACCOUNTS), "ListCachedUsers", &())
            .await
            .map_err(|e| e.to_string())?;
        let paths: Vec<zbus::zvariant::OwnedObjectPath> = reply.body().deserialize().map_err(|e| e.to_string())?;
        let mut users = Vec::new();
        for p in paths {
            if let Ok(u) = read_user(&conn, p.as_str().to_string()).await {
                users.push(u);
            }
        }
        // ListCachedUsers only returns users who have signed in before; make sure we're included.
        let me = std::env::var("USER").unwrap_or_default();
        if !users.iter().any(|u| u.name == me) {
            if let Ok(r) = conn
                .call_method(Some(ACCOUNTS), "/org/freedesktop/Accounts", Some(ACCOUNTS), "FindUserByName", &me)
                .await
            {
                if let Ok(p) = r.body().deserialize::<zbus::zvariant::OwnedObjectPath>() {
                    if let Ok(u) = read_user(&conn, p.as_str().to_string()).await {
                        users.push(u);
                    }
                }
            }
        }
        users.sort_by_key(|u| u.uid);
        Ok(users)
    })
}

fn call_user(path: &str, method: &str, arg: &str) -> Result<(), String> {
    block_on(async {
        let conn = zbus::Connection::system().await.map_err(|e| e.to_string())?;
        conn.call_method(Some(ACCOUNTS), path, Some(USER_IFACE), method, &arg)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    })
}

/// Runs `script` as root through pkexec with positional args and optional stdin.
fn pkexec_sh(script: &str, args: &[&str], stdin: Option<String>) -> Result<(), String> {
    let mut cmd = Command::new("pkexec");
    cmd.args(["sh", "-c", script, "sh"]).args(args);
    cmd.stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() });
    cmd.stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    if let (Some(data), Some(mut sin)) = (stdin, child.stdin.take()) {
        let _ = sin.write_all(data.as_bytes());
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    match out.status.code() {
        Some(0) => Ok(()),
        // pkexec: 126 = dismissed, 127 = not authorized
        Some(126) | Some(127) => Err("Administrator approval was not given.".into()),
        _ => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
    }
}

fn message(w: &impl IsA<gtk4::Widget>, title: &str, body: &str) {
    let d = adw::AlertDialog::new(Some(title), Some(body));
    d.add_response("ok", "OK");
    d.present(Some(w));
}

fn avatar(u: &User, size: i32) -> adw::Avatar {
    let a = adw::Avatar::new(size, Some(if u.real_name.is_empty() { &u.name } else { &u.real_name }), true);
    if !u.icon.is_empty() && std::path::Path::new(&u.icon).exists() {
        if let Ok(tex) = gtk4::gdk::Texture::from_filename(&u.icon) {
            a.set_custom_image(Some(&tex));
        }
    }
    a
}

fn password_entries() -> (gtk4::Box, gtk4::PasswordEntry, gtk4::PasswordEntry) {
    let b = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    let p1 = gtk4::PasswordEntry::builder().placeholder_text("New password").show_peek_icon(true).build();
    let p2 = gtk4::PasswordEntry::builder().placeholder_text("Confirm password").show_peek_icon(true).build();
    b.append(&p1);
    b.append(&p2);
    (b, p1, p2)
}

fn validate_password(p1: &str, p2: &str) -> Result<(), &'static str> {
    if p1.chars().count() < 8 {
        Err("Use at least 8 characters.")
    } else if p1 != p2 {
        Err("The passwords don't match.")
    } else {
        Ok(())
    }
}

fn change_password_dialog(w: &gtk4::Widget, user: &str) {
    let d = adw::AlertDialog::new(Some("Change password"), Some(&format!("Set a new password for {user}.")));
    let (b, p1, p2) = password_entries();
    d.set_extra_child(Some(&b));
    d.add_responses(&[("cancel", "Cancel"), ("save", "Change password")]);
    d.set_response_appearance("save", adw::ResponseAppearance::Suggested);
    d.set_close_response("cancel");
    let (user, w2) = (user.to_string(), w.clone());
    d.connect_response(None, move |_, r| {
        if r != "save" {
            return;
        }
        let (a, b) = (p1.text().to_string(), p2.text().to_string());
        if let Err(e) = validate_password(&a, &b) {
            message(&w2, "Password not changed", e);
            return;
        }
        let (user, w3) = (user.clone(), w2.clone());
        in_background(
            move || pkexec_sh("chpasswd", &[], Some(format!("{user}:{a}\n"))),
            move |res| match res {
                Ok(()) => message(&w3, "Password changed", "Use your new password the next time you sign in."),
                Err(e) => message(&w3, "Password not changed", &e),
            },
        );
    });
    d.present(Some(w));
}

// ── Your account ───────────────────────────────────────────────────────────

fn me_group(me: &User, page: &gtk4::Box) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();

    let header = adw::ActionRow::new();
    header.set_title(&glib::markup_escape_text(if me.real_name.is_empty() { &me.name } else { &me.real_name }));
    header.set_subtitle(&format!("{} · {}", me.name, if me.admin { "Administrator" } else { "Standard account" }));
    header.add_prefix(&avatar(me, 64));
    header.set_activatable(false);
    g.add(&header);

    let name = adw::EntryRow::new();
    name.set_title("Full name");
    name.set_text(&me.real_name);
    name.set_show_apply_button(true);
    let path = me.path.clone();
    let page2 = page.clone();
    name.connect_apply(move |e| {
        let (text, path, page3, header) = (e.text().to_string(), path.clone(), page2.clone(), header.clone());
        in_background(
            move || call_user(&path, "SetRealName", &text).map(|_| text),
            move |res| match res {
                Ok(t) => header.set_title(&glib::markup_escape_text(&t)),
                Err(e) => message(&page3, "Name not changed", &e),
            },
        );
    });
    g.add(&name);

    let pic = adw::ActionRow::new();
    pic.set_title("Account picture");
    pic.set_subtitle("Shown on the sign-in and lock screens");
    let choose = gtk4::Button::with_label("Choose…");
    choose.set_valign(gtk4::Align::Center);
    let path = me.path.clone();
    let page2 = page.clone();
    choose.connect_clicked(move |b| {
        let dialog = gtk4::FileDialog::new();
        dialog.set_title("Choose an account picture");
        let filter = gtk4::FileFilter::new();
        filter.add_pixbuf_formats();
        filter.set_name(Some("Images"));
        let filters = gtk4::gio::ListStore::new::<gtk4::FileFilter>();
        filters.append(&filter);
        dialog.set_filters(Some(&filters));
        let (path, page3) = (path.clone(), page2.clone());
        dialog.open(b.root().and_downcast_ref::<gtk4::Window>(), None::<&gtk4::gio::Cancellable>, move |res| {
            let Some(file) = res.ok().and_then(|f| f.path()) else { return };
            let (path, page4, file) = (path.clone(), page3.clone(), file.to_string_lossy().to_string());
            in_background(
                move || call_user(&path, "SetIconFile", &file),
                move |res| match res {
                    Ok(()) => message(&page4, "Picture updated", "Your new picture appears the next time you open this page."),
                    Err(e) => message(&page4, "Picture not changed", &e),
                },
            );
        });
    });
    pic.add_suffix(&choose);
    g.add(&pic);

    let pw = adw::ActionRow::new();
    pw.set_title("Password");
    let change = gtk4::Button::with_label("Change…");
    change.set_valign(gtk4::Align::Center);
    let user = me.name.clone();
    change.connect_clicked(move |b| change_password_dialog(b.upcast_ref(), &user));
    pw.add_suffix(&change);
    g.add(&pw);
    g
}

// ── Sign-in options ────────────────────────────────────────────────────────

fn autologin_user() -> Option<String> {
    let text = std::fs::read_to_string(AUTOLOGIN_CONF).ok()?;
    text.lines().find_map(|l| l.trim().strip_prefix("User=").map(str::to_string)).filter(|u| !u.is_empty())
}

fn signin_group(me: &User, page: &gtk4::Box) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("Sign-in options");

    let wake = adw::SwitchRow::new();
    wake.set_title("Require password after sleep");
    wake.set_subtitle("Lock the screen when the computer wakes up");
    wake.set_active(kconfig::read("kscreenlockerrc", &["Daemon"], "LockOnResume").as_deref() != Some("false"));
    wake.connect_active_notify(|r| {
        let v = if r.is_active() { "true" } else { "false" };
        kconfig::spawn(move || kconfig::write("kscreenlockerrc", &["Daemon"], "LockOnResume", v));
    });
    g.add(&wake);

    const MINUTES: [u32; 6] = [1, 2, 5, 10, 15, 30];
    let auto = adw::ComboRow::new();
    auto.set_title("Lock automatically after");
    let mut labels = vec!["Never".to_string()];
    labels.extend(MINUTES.iter().map(|m| if *m == 1 { "1 minute".into() } else { format!("{m} minutes") }));
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    auto.set_model(Some(&gtk4::StringList::new(&refs)));
    let enabled = kconfig::read("kscreenlockerrc", &["Daemon"], "Autolock").as_deref() != Some("false");
    let timeout: u32 = kconfig::read("kscreenlockerrc", &["Daemon"], "Timeout").and_then(|v| v.parse().ok()).unwrap_or(5);
    auto.set_selected(if enabled { MINUTES.iter().position(|m| *m == timeout).map_or(3, |i| i + 1) as u32 } else { 0 });
    auto.connect_selected_notify(|r| {
        let sel = r.selected() as usize;
        kconfig::spawn(move || {
            if sel == 0 {
                kconfig::write("kscreenlockerrc", &["Daemon"], "Autolock", "false");
            } else {
                kconfig::write("kscreenlockerrc", &["Daemon"], "Autolock", "true");
                kconfig::write("kscreenlockerrc", &["Daemon"], "Timeout", &MINUTES[sel - 1].to_string());
            }
        });
    });
    g.add(&auto);

    let al = adw::SwitchRow::new();
    al.set_title("Sign in automatically");
    al.set_subtitle("Skip the sign-in screen when the computer starts. Anyone with access to it can use your account");
    al.set_active(autologin_user().as_deref() == Some(me.name.as_str()));
    let (name, page2) = (me.name.clone(), page.clone());
    let reverting = std::rc::Rc::new(std::cell::Cell::new(false));
    al.connect_active_notify(move |r| {
        if reverting.get() {
            return;
        }
        let on = r.is_active();
        let (name, page3, r2, reverting) = (name.clone(), page2.clone(), r.clone(), reverting.clone());
        in_background(
            move || {
                if on {
                    pkexec_sh(
                        &format!("mkdir -p /etc/sddm.conf.d && printf '[Autologin]\\nUser=%s\\nSession=plasma\\n' \"$1\" > {AUTOLOGIN_CONF}"),
                        &[&name],
                        None,
                    )
                } else {
                    pkexec_sh(&format!("rm -f {AUTOLOGIN_CONF}"), &[], None)
                }
            },
            move |res| {
                if let Err(e) = res {
                    reverting.set(true);
                    r2.set_active(!on);
                    reverting.set(false);
                    message(&page3, "Setting not changed", &e);
                }
            },
        );
    });
    g.add(&al);
    g
}

// ── Other users ────────────────────────────────────────────────────────────

fn valid_username(u: &str) -> bool {
    let mut c = u.chars();
    matches!(c.next(), Some(f) if f.is_ascii_lowercase() || f == '_')
        && u.len() <= 32
        && u.chars().all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-')
}

fn suggest_username(full: &str) -> String {
    full.split_whitespace()
        .next()
        .unwrap_or("")
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        .collect()
}

fn add_user_dialog(page: &gtk4::Box, reload: std::rc::Rc<dyn Fn()>) {
    let d = adw::AlertDialog::new(Some("Add a user"), None);
    let form = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    let full = gtk4::Entry::builder().placeholder_text("Full name").build();
    let user = gtk4::Entry::builder().placeholder_text("Username").build();
    let (pw_box, p1, p2) = password_entries();
    let admin = gtk4::CheckButton::with_label("Administrator (can install apps and change system settings)");
    form.append(&full);
    form.append(&user);
    form.append(&pw_box);
    form.append(&admin);
    {
        let user = user.clone();
        let edited = std::rc::Rc::new(std::cell::Cell::new(false));
        let e2 = edited.clone();
        user.connect_changed(move |u| {
            if u.has_focus() {
                e2.set(true);
            }
        });
        full.connect_changed(move |f| {
            if !edited.get() {
                user.set_text(&suggest_username(&f.text()));
            }
        });
    }
    d.set_extra_child(Some(&form));
    d.add_responses(&[("cancel", "Cancel"), ("add", "Add user")]);
    d.set_response_appearance("add", adw::ResponseAppearance::Suggested);
    d.set_close_response("cancel");
    let page2 = page.clone();
    d.connect_response(None, move |_, r| {
        if r != "add" {
            return;
        }
        let (fname, uname) = (full.text().trim().to_string(), user.text().trim().to_string());
        let (a, b) = (p1.text().to_string(), p2.text().to_string());
        if !valid_username(&uname) {
            message(&page2, "User not added", "Usernames use lowercase letters, numbers, - and _, and start with a letter.");
            return;
        }
        if let Err(e) = validate_password(&a, &b) {
            message(&page2, "User not added", e);
            return;
        }
        let groups = if admin.is_active() { "wheel" } else { "" };
        let (page3, reload) = (page2.clone(), reload.clone());
        in_background(
            move || {
                pkexec_sh(
                    "useradd -m -c \"$1\" ${3:+-G \"$3\"} -- \"$2\" && chpasswd",
                    &[&fname, &uname, groups],
                    Some(format!("{uname}:{a}\n")),
                )
            },
            move |res| match res {
                Ok(()) => reload(),
                Err(e) => message(&page3, "User not added", &e),
            },
        );
    });
    d.present(Some(page));
}

fn user_row(u: &User, page: &gtk4::Box, reload: std::rc::Rc<dyn Fn()>) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(&glib::markup_escape_text(if u.real_name.is_empty() { &u.name } else { &u.real_name }));
    row.set_subtitle(&u.name);
    row.add_prefix(&avatar(u, 36));

    let kind = gtk4::DropDown::from_strings(&["Standard", "Administrator"]);
    kind.set_selected(u.admin as u32);
    kind.set_valign(gtk4::Align::Center);
    let (name, page2, reload2) = (u.name.clone(), page.clone(), reload.clone());
    kind.connect_selected_notify(move |k| {
        let admin = k.selected() == 1;
        let (name, page3, reload3) = (name.clone(), page2.clone(), reload2.clone());
        in_background(
            move || pkexec_sh(if admin { "gpasswd -a \"$1\" wheel" } else { "gpasswd -d \"$1\" wheel" }, &[&name], None),
            move |res| {
                if let Err(e) = res {
                    message(&page3, "Account type not changed", &e);
                }
                reload3();
            },
        );
    });
    row.add_suffix(&kind);

    let pw = gtk4::Button::from_icon_name("dialog-password-symbolic");
    pw.add_css_class("flat");
    pw.set_valign(gtk4::Align::Center);
    pw.set_tooltip_text(Some("Set password"));
    let name = u.name.clone();
    pw.connect_clicked(move |b| change_password_dialog(b.upcast_ref(), &name));
    row.add_suffix(&pw);

    let del = gtk4::Button::from_icon_name("user-trash-symbolic");
    del.add_css_class("flat");
    del.set_valign(gtk4::Align::Center);
    del.set_tooltip_text(Some("Remove user"));
    let (name, page2) = (u.name.clone(), page.clone());
    del.connect_clicked(move |_| {
        let d = adw::AlertDialog::new(
            Some(&format!("Remove {name}?")),
            Some("Their account can no longer sign in. Choose whether to also delete their files."),
        );
        d.add_responses(&[("cancel", "Cancel"), ("keep", "Remove, keep files"), ("delete", "Remove and delete files")]);
        d.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        d.set_close_response("cancel");
        let (name, page3, reload) = (name.clone(), page2.clone(), reload.clone());
        d.connect_response(None, move |_, r| {
            if r == "cancel" {
                return;
            }
            let delete_files = r == "delete";
            let (name, page4, reload) = (name.clone(), page3.clone(), reload.clone());
            in_background(
                move || pkexec_sh(if delete_files { "userdel -r -- \"$1\"" } else { "userdel -- \"$1\"" }, &[&name], None),
                move |res| {
                    if let Err(e) = res {
                        message(&page4, "User not removed", &e);
                    }
                    reload();
                },
            );
        });
        d.present(Some(&page2));
    });
    row.add_suffix(&del);
    row
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
            .label("Accounts")
            .halign(gtk4::Align::Start)
            .css_classes(vec!["win11-page-title".to_string()])
            .build(),
    );

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 24);
    root.append(&content);

    // Rebuilds the whole page from AccountsService; used after any change.
    let reload: std::rc::Rc<std::cell::RefCell<Option<std::rc::Rc<dyn Fn()>>>> = Default::default();
    let reload_fn: std::rc::Rc<dyn Fn()> = {
        let (content, root, reload) = (content.clone(), root.clone(), reload.clone());
        std::rc::Rc::new(move || {
            let (content, root, reload) = (content.clone(), root.clone(), reload.clone());
            in_background(load_users, move |res| {
                while let Some(c) = content.first_child() {
                    content.remove(&c);
                }
                let users = match res {
                    Ok(u) => u,
                    Err(e) => {
                        content.append(
                            &adw::StatusPage::builder()
                                .icon_name("dialog-warning-symbolic")
                                .title("Accounts unavailable")
                                .description(format!("The account service didn't respond: {e}"))
                                .build(),
                        );
                        return;
                    }
                };
                let again = reload.borrow().clone().expect("reload set");
                let me_name = std::env::var("USER").unwrap_or_default();
                if let Some(me) = users.iter().find(|u| u.name == me_name) {
                    content.append(&me_group(me, &root));
                    content.append(&signin_group(me, &root));
                }
                let others = adw::PreferencesGroup::new();
                others.set_title("Other users");
                let add = gtk4::Button::with_label("Add user");
                add.set_valign(gtk4::Align::Center);
                let (root2, again2) = (root.clone(), again.clone());
                add.connect_clicked(move |_| add_user_dialog(&root2, again2.clone()));
                others.set_header_suffix(Some(&add));
                let list: Vec<&User> = users.iter().filter(|u| u.name != me_name).collect();
                if list.is_empty() {
                    let r = adw::ActionRow::new();
                    r.set_title("No other users");
                    others.add(&r);
                }
                for u in list {
                    others.add(&user_row(u, &root, again.clone()));
                }
                content.append(&others);
            });
        })
    };
    *reload.borrow_mut() = Some(reload_fn.clone());
    reload_fn();

    scroll.set_child(Some(&root));
    scroll.upcast()
}
