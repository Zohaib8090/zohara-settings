//! The finer permission controls of a Flatpak app, shown in its details screen: **which folders** it may use (each of the
//! ordinary ones as none / read only / read and write, plus any folder you add) and **every other permission** it has, in
//! plain words, each with its own switch. The big switches (internet, sound, graphics, devices, Bluetooth) stay where they
//! were in the details screen; what each button does lives in `backend::app_details`.

use crate::backend::app_details as ad;
use crate::backend::worker::in_background;
use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub struct Panel {
    pub widget: gtk4::Box,
    /// Reads the permissions again and redraws (after something else changed them).
    pub refresh: Rc<dyn Fn()>,
}

fn message(from: &impl IsA<gtk4::Widget>, title: &str, body: &str) {
    let d = adw::AlertDialog::new(Some(title), Some(body));
    d.add_response("ok", "OK");
    d.present(Some(from));
}

fn tilde(p: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    match p.strip_prefix(&home) {
        Some(rest) if !home.is_empty() => format!("~{rest}"),
        _ => p.to_string(),
    }
}

const ACCESS_LABELS: [&str; 3] = ["No access", "Read only", "Read and write"];

fn access_index(a: ad::Access) -> u32 {
    match a {
        ad::Access::None => 0,
        ad::Access::ReadOnly => 1,
        ad::Access::ReadWrite => 2,
    }
}

fn access_from(i: u32) -> ad::Access {
    match i {
        1 => ad::Access::ReadOnly,
        2 => ad::Access::ReadWrite,
        _ => ad::Access::None,
    }
}

/// Permissions the details screen already shows as big switches: not repeated in the full list.
fn shown_elsewhere(p: &ad::Perm) -> bool {
    matches!(
        (p.kind, p.token.as_str()),
        (ad::PermKind::Shared, "network") | (ad::PermKind::Socket, "pulseaudio") | (ad::PermKind::Device, "dri") | (ad::PermKind::Device, "all") | (ad::PermKind::Feature, "bluetooth")
    )
}

pub fn build(app_id: &str, dialog: &adw::Dialog) -> Panel {
    let widget = gtk4::Box::new(gtk4::Orientation::Vertical, 18);

    let folders = adw::PreferencesGroup::new();
    folders.set_title("Folders");
    folders.set_description(Some("Which of your folders this app can use, besides its own. Changes apply the next time the app is opened."));
    let all = adw::PreferencesGroup::new();
    all.set_title("All permissions");
    all.set_description(Some("Everything else this app is allowed to do, in plain words. Switch off what it doesn't need. “Back to the app's own permissions” undoes it."));
    widget.append(&folders);
    widget.append(&all);

    let folder_rows: Rc<RefCell<Vec<gtk4::Widget>>> = Rc::default();
    let perm_rows: Rc<RefCell<Vec<gtk4::Widget>>> = Rc::default();
    let generation = Rc::new(Cell::new(0u32));
    let refresh_cell: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::default();

    let refresh: Rc<dyn Fn()> = {
        let (app_id, dialog, folders, all) = (app_id.to_string(), dialog.clone(), folders.clone(), all.clone());
        let (folder_rows, perm_rows, generation, refresh_cell) = (folder_rows.clone(), perm_rows.clone(), generation.clone(), refresh_cell.clone());
        Rc::new(move || {
            let mine = generation.get() + 1;
            generation.set(mine);
            let (app_id2, dialog, folders, all) = (app_id.clone(), dialog.clone(), folders.clone(), all.clone());
            let (folder_rows, perm_rows, generation, refresh_cell) = (folder_rows.clone(), perm_rows.clone(), generation.clone(), refresh_cell.clone());
            let app_for_read = app_id.clone();
            in_background(
                move || ad::permission_texts(&app_for_read),
                move |texts| {
                    if generation.get() != mine {
                        return; // a newer read is on its way
                    }
                    for w in folder_rows.borrow_mut().drain(..) {
                        folders.remove(&w);
                    }
                    for w in perm_rows.borrow_mut().drain(..) {
                        all.remove(&w);
                    }
                    let Some((meta, over)) = texts else { return };
                    let eff = ad::parse_context(&meta).with_overrides(&ad::parse_context(&over));
                    let wide = ad::files_level(&eff) != ad::Files::Chosen;
                    let again = refresh_cell.borrow().clone();

                    // The ordinary folders.
                    for (token, label) in ad::STANDARD_FOLDERS {
                        let row = adw::ComboRow::new();
                        row.set_title(label);
                        row.set_model(Some(&gtk4::StringList::new(&ACCESS_LABELS)));
                        if wide {
                            row.set_subtitle("Already reachable: it can see your home folder");
                            row.set_selected(2);
                            row.set_sensitive(false);
                        } else {
                            row.set_selected(access_index(ad::folder_access(&eff, token)));
                            let (app, dialog, again) = (app_id2.clone(), dialog.clone(), again.clone());
                            let loading = Rc::new(Cell::new(true));
                            let l2 = loading.clone();
                            row.connect_selected_notify(move |r| {
                                if l2.get() {
                                    return;
                                }
                                let access = access_from(r.selected());
                                let (app, dialog, again) = (app.clone(), dialog.clone(), again.clone());
                                in_background(move || ad::set_folder(&app, token, access), move |res| {
                                    if let Err(e) = res {
                                        message(&dialog, "Couldn't change it", &e);
                                    }
                                    if let Some(f) = again {
                                        f();
                                    }
                                });
                            });
                            loading.set(false);
                        }
                        folders.add(&row);
                        folder_rows.borrow_mut().push(row.upcast());
                    }

                    // Folders it was given that are not one of those.
                    for (path, access) in ad::custom_folders(&eff) {
                        let row = adw::ComboRow::new();
                        row.set_title(&glib::markup_escape_text(&tilde(&path)));
                        row.set_model(Some(&gtk4::StringList::new(&ACCESS_LABELS)));
                        row.set_selected(access_index(access));
                        let remove = gtk4::Button::from_icon_name("user-trash-symbolic");
                        remove.add_css_class("flat");
                        remove.set_valign(gtk4::Align::Center);
                        remove.set_tooltip_text(Some("Take this folder away from the app"));
                        row.add_suffix(&remove);
                        let loading = Rc::new(Cell::new(true));
                        {
                            let (app, dialog, again, path, l2) = (app_id2.clone(), dialog.clone(), again.clone(), path.clone(), loading.clone());
                            row.connect_selected_notify(move |r| {
                                if l2.get() {
                                    return;
                                }
                                let access = access_from(r.selected());
                                let (app, dialog, again, path) = (app.clone(), dialog.clone(), again.clone(), path.clone());
                                in_background(move || ad::set_folder(&app, &path, access), move |res| {
                                    if let Err(e) = res {
                                        message(&dialog, "Couldn't change it", &e);
                                    }
                                    if let Some(f) = again {
                                        f();
                                    }
                                });
                            });
                        }
                        {
                            let (app, dialog, again, path) = (app_id2.clone(), dialog.clone(), again.clone(), path.clone());
                            remove.connect_clicked(move |_| {
                                let (app, dialog, again, path) = (app.clone(), dialog.clone(), again.clone(), path.clone());
                                in_background(move || ad::set_folder(&app, &path, ad::Access::None), move |res| {
                                    if let Err(e) = res {
                                        message(&dialog, "Couldn't change it", &e);
                                    }
                                    if let Some(f) = again {
                                        f();
                                    }
                                });
                            });
                        }
                        loading.set(false);
                        folders.add(&row);
                        folder_rows.borrow_mut().push(row.upcast());
                    }

                    // Adding another folder.
                    let add = adw::ActionRow::new();
                    add.set_title("Add a folder…");
                    add.set_subtitle("Let the app use one more folder");
                    add.add_prefix(&gtk4::Image::from_icon_name("folder-new-symbolic"));
                    add.set_sensitive(!wide);
                    let add_btn = gtk4::Button::with_label("Choose");
                    add_btn.set_valign(gtk4::Align::Center);
                    add.add_suffix(&add_btn);
                    {
                        let (app, dialog, again) = (app_id2.clone(), dialog.clone(), again.clone());
                        add_btn.connect_clicked(move |b| {
                            let chooser = gtk4::FileDialog::new();
                            chooser.set_title("Choose a folder for the app");
                            let (app, dialog, again) = (app.clone(), dialog.clone(), again.clone());
                            let parent = b.root().and_downcast::<gtk4::Window>();
                            chooser.select_folder(parent.as_ref(), None::<&gtk4::gio::Cancellable>, move |res| {
                                let Some(path) = res.ok().and_then(|f| f.path()) else { return };
                                let path = path.to_string_lossy().to_string();
                                let (app, dialog, again) = (app.clone(), dialog.clone(), again.clone());
                                in_background(move || ad::set_folder(&app, &path, ad::Access::ReadWrite), move |res| {
                                    if let Err(e) = res {
                                        message(&dialog, "Couldn't add the folder", &e);
                                    }
                                    if let Some(f) = again {
                                        f();
                                    }
                                });
                            });
                        });
                    }
                    folders.add(&add);
                    folder_rows.borrow_mut().push(add.upcast());

                    // Every other permission.
                    for p in ad::permission_items(&meta, &over).into_iter().filter(|p| !shown_elsewhere(p)) {
                        let row = adw::SwitchRow::new();
                        row.set_title(&glib::markup_escape_text(&p.label));
                        row.set_subtitle(&glib::markup_escape_text(&if p.broad { format!("{} · reaches a lot", p.token) } else { p.token.clone() }));
                        if p.broad {
                            let w = gtk4::Image::from_icon_name("dialog-warning-symbolic");
                            row.add_prefix(&w);
                        }
                        row.set_active(p.allowed);
                        let (app, dialog) = (app_id2.clone(), dialog.clone());
                        let (kind, token) = (p.kind, p.token.clone());
                        let loading = Rc::new(Cell::new(false));
                        row.connect_active_notify(move |r| {
                            if loading.get() {
                                return;
                            }
                            let on = r.is_active();
                            let (app, dialog, token, r2, loading) = (app.clone(), dialog.clone(), token.clone(), r.clone(), loading.clone());
                            in_background(move || ad::set_perm(&app, kind, &token, on), move |res| {
                                if let Err(e) = res {
                                    loading.set(true);
                                    r2.set_active(!on);
                                    loading.set(false);
                                    message(&dialog, "Couldn't change it", &e);
                                }
                            });
                        });
                        all.add(&row);
                        perm_rows.borrow_mut().push(row.upcast());
                    }
                },
            );
        })
    };
    *refresh_cell.borrow_mut() = Some(refresh.clone());
    refresh();
    Panel { widget, refresh }
}
