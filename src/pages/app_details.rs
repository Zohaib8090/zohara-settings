//! The details screen of one installed app: how much space it uses (and clearing its cache or saved data), whether it
//! may show notifications, what a Flatpak app may do (network, files, sound, graphics, devices, Bluetooth, camera,
//! microphone), and uninstalling it. What each button does, and its tests, live in `backend::app_details`.

use crate::backend::app_details as ad;
use crate::backend::kconfig;
use crate::backend::worker::in_background;
use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use std::cell::Cell;
use std::rc::Rc;

/// Where an app comes from: decides what can be shown and changed.
#[derive(Clone)]
pub enum Kind {
    Flatpak(String),
    Pacman(String),
    Other,
}

#[derive(Clone)]
pub struct Target {
    pub name: String,
    pub icon: Option<gtk4::gio::Icon>,
    pub version: String,
    pub kind: Kind,
    /// The launcher's file name: Plasma knows the app's notifications by it.
    pub desktop_id: String,
}

impl Target {
    fn flatpak_id(&self) -> Option<&str> {
        match &self.kind {
            Kind::Flatpak(id) => Some(id),
            _ => None,
        }
    }

    /// Names an ordinary app's folders may go by.
    fn folder_names(&self) -> Vec<String> {
        let mut v = vec![ad::notification_id(&self.desktop_id), self.name.clone(), self.name.to_lowercase()];
        if let Kind::Pacman(p) = &self.kind {
            v.push(p.clone());
        }
        v
    }
}

fn message(from: &impl IsA<gtk4::Widget>, title: &str, body: &str) {
    let d = adw::AlertDialog::new(Some(title), Some(body));
    d.add_response("ok", "OK");
    d.present(Some(from));
}

fn switch_row(title: &str, subtitle: &str) -> adw::SwitchRow {
    let r = adw::SwitchRow::new();
    r.set_title(title);
    r.set_subtitle(subtitle);
    r.set_sensitive(false); // until the real state has been read
    r
}

/// Everything slow, read off the UI thread.
struct Loaded {
    app_size: Option<String>,
    data_bytes: u64,
    cache_bytes: u64,
    permissions: Option<ad::Context>,
    camera: Option<bool>,
    microphone: Option<bool>,
    notifications_portal: Option<bool>,
    running: bool,
}

fn load(kind: Kind, names: Vec<String>) -> Loaded {
    let mut l = Loaded { app_size: None, data_bytes: 0, cache_bytes: 0, permissions: None, camera: None, microphone: None, notifications_portal: None, running: false };
    match &kind {
        Kind::Flatpak(id) => {
            l.app_size = ad::flatpak_app_size(id);
            if let Some(dir) = ad::flatpak_data_dir(id) {
                l.cache_bytes = ad::dir_size(&dir.join("cache"));
                l.data_bytes = ad::dir_size(&dir).saturating_sub(l.cache_bytes);
            }
            l.permissions = ad::effective_context(id);
            l.camera = ad::portal_state(id, ad::Portal::Camera);
            l.microphone = ad::portal_state(id, ad::Portal::Microphone);
            l.notifications_portal = ad::portal_state(id, ad::Portal::Notifications);
            l.running = ad::flatpak_running(id);
        }
        Kind::Pacman(pkg) => {
            l.app_size = ad::pacman_app_size(pkg);
            let (data, cache) = ad::native_dirs(&names);
            l.data_bytes = ad::total_size(&data);
            l.cache_bytes = ad::total_size(&cache);
        }
        Kind::Other => {
            let (data, cache) = ad::native_dirs(&names);
            l.data_bytes = ad::total_size(&data);
            l.cache_bytes = ad::total_size(&cache);
        }
    }
    l
}

/// Opens the details of `target` over `parent`. `uninstall` (when the app can be removed) is called after the person
/// presses "Uninstall…"; it asks for confirmation itself.
pub fn open(parent: &impl IsA<gtk4::Widget>, target: Target, uninstall: Option<Rc<dyn Fn()>>) {
    let dialog = adw::Dialog::new();
    dialog.set_title(&target.name);
    dialog.set_content_width(580);
    dialog.set_content_height(680);

    let body = gtk4::Box::new(gtk4::Orientation::Vertical, 18);
    body.set_margin_top(12);
    body.set_margin_bottom(24);
    body.set_margin_start(18);
    body.set_margin_end(18);

    // Who this is.
    let head = gtk4::Box::new(gtk4::Orientation::Horizontal, 14);
    let img = match &target.icon {
        Some(i) => gtk4::Image::from_gicon(i),
        None => gtk4::Image::from_icon_name("application-x-executable"),
    };
    img.set_pixel_size(64);
    head.append(&img);
    let titles = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    titles.set_valign(gtk4::Align::Center);
    let name = gtk4::Label::builder().label(&target.name).xalign(0.0).css_classes(vec!["title-2".to_string()]).build();
    let from = match &target.kind {
        Kind::Flatpak(id) => format!("Flatpak app · {id}"),
        Kind::Pacman(p) => format!("Installed from the system packages · {p}"),
        Kind::Other => "Installed manually".to_string(),
    };
    let sub = gtk4::Label::builder().label(format!("{from}{}", if target.version.is_empty() { String::new() } else { format!(" · {}", target.version) })).xalign(0.0).wrap(true).css_classes(vec!["dim-label".to_string()]).build();
    titles.append(&name);
    titles.append(&sub);
    head.append(&titles);
    body.append(&head);

    // Storage.
    let storage = adw::PreferencesGroup::new();
    storage.set_title("Storage");
    let size_row = adw::ActionRow::new();
    size_row.set_title("App");
    let size_label = gtk4::Label::new(Some("Counting…"));
    size_label.add_css_class("dim-label");
    size_row.add_suffix(&size_label);
    storage.add(&size_row);

    let data_row = adw::ActionRow::new();
    data_row.set_title("Saved data");
    data_row.set_subtitle("Settings, accounts and files the app keeps");
    let data_label = gtk4::Label::new(Some("Counting…"));
    data_label.add_css_class("dim-label");
    data_row.add_suffix(&data_label);
    let clear_data = gtk4::Button::with_label("Clear data…");
    clear_data.add_css_class("destructive-action");
    clear_data.set_valign(gtk4::Align::Center);
    clear_data.set_sensitive(false);
    // Clearing saved data is only offered where the data's place is certain: a Flatpak app's own folder.
    clear_data.set_visible(target.flatpak_id().is_some());
    data_row.add_suffix(&clear_data);
    storage.add(&data_row);

    let cache_row = adw::ActionRow::new();
    cache_row.set_title("Cache");
    cache_row.set_subtitle("Temporary files the app can make again");
    let cache_label = gtk4::Label::new(Some("Counting…"));
    cache_label.add_css_class("dim-label");
    cache_row.add_suffix(&cache_label);
    let clear_cache = gtk4::Button::with_label("Clear cache");
    clear_cache.set_valign(gtk4::Align::Center);
    clear_cache.set_sensitive(false);
    cache_row.add_suffix(&clear_cache);
    storage.add(&cache_row);
    body.append(&storage);

    // Notifications.
    let notes = adw::PreferencesGroup::new();
    notes.set_title("Notifications");
    let notif = adw::SwitchRow::new();
    notif.set_title("Show notifications from this app");
    notif.set_subtitle("Pop-ups and the notification history. All your notification options are in Notifications");
    let notif_id = ad::notification_id(&target.desktop_id);
    let plasma_on = {
        let v = kconfig::read("plasmanotifyrc", &["Applications", &notif_id], "ShowPopups");
        v.as_deref() != Some("false")
    };
    notif.set_active(plasma_on);
    notes.add(&notif);
    body.append(&notes);

    // What a Flatpak app may do.
    let perms = adw::PreferencesGroup::new();
    perms.set_title("Permissions");
    perms.set_description(Some("What this app is allowed to reach. Changes apply the next time the app is opened."));
    let network = switch_row("Network", "Use the internet and your local network");
    let sound = switch_row("Sound", "Play sound, and record when it has the microphone");
    let graphics = switch_row("Graphics acceleration", "Use the graphics card for smooth drawing and games");
    let devices = switch_row("All devices", "USB devices, game controllers and other hardware");
    let bluetooth = switch_row("Bluetooth", "Connect to Bluetooth devices");
    let camera = switch_row("Camera", "Allowed through the desktop");
    let microphone = switch_row("Microphone", "Allowed through the desktop");
    let files = adw::ComboRow::new();
    files.set_title("Files");
    files.set_subtitle("Which of your files it can see");
    files.set_model(Some(&gtk4::StringList::new(&["Only what you choose for it", "Your home folder", "Everything on this computer"])));
    files.set_sensitive(false);
    for r in [&network] {
        perms.add(r);
    }
    perms.add(&files);
    for r in [&sound, &graphics, &devices, &bluetooth, &camera, &microphone] {
        perms.add(r);
    }
    let reset = gtk4::Button::with_label("Back to the app's own permissions");
    reset.set_halign(gtk4::Align::Start);
    reset.set_sensitive(false);
    perms.set_visible(target.flatpak_id().is_some());
    body.append(&perms);
    if target.flatpak_id().is_some() {
        body.append(&reset);
    }

    // Remove.
    if let Some(uninstall) = uninstall.clone() {
        let rm = gtk4::Button::with_label("Uninstall…");
        rm.add_css_class("destructive-action");
        rm.set_halign(gtk4::Align::Start);
        let dialog2 = dialog.clone();
        rm.connect_clicked(move |_| {
            dialog2.close();
            uninstall();
        });
        body.append(&rm);
    }

    let clamp = adw::Clamp::new();
    clamp.set_maximum_size(560);
    clamp.set_child(Some(&body));
    let scroll = gtk4::ScrolledWindow::builder().hscrollbar_policy(gtk4::PolicyType::Never).vexpand(true).build();
    scroll.set_child(Some(&clamp));
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&scroll));
    dialog.set_child(Some(&view));
    dialog.present(Some(parent));

    // ── Filling it in ──
    let running = Rc::new(Cell::new(false));
    let updating = Rc::new(Cell::new(true));
    {
        let (kind, names) = (target.kind.clone(), target.folder_names());
        let (size_label, data_label, cache_label) = (size_label.clone(), data_label.clone(), cache_label.clone());
        let (clear_cache, clear_data, files, running, updating) = (clear_cache.clone(), clear_data.clone(), files.clone(), running.clone(), updating.clone());
        let (network, sound, graphics, devices, bluetooth, camera, microphone, reset, notif) =
            (network.clone(), sound.clone(), graphics.clone(), devices.clone(), bluetooth.clone(), camera.clone(), microphone.clone(), reset.clone(), notif.clone());
        in_background(
            move || load(kind, names),
            move |l| {
                size_label.set_text(&l.app_size.clone().unwrap_or_else(|| "Unknown".into()));
                data_label.set_text(&ad::human_size(l.data_bytes));
                cache_label.set_text(&ad::human_size(l.cache_bytes));
                clear_cache.set_sensitive(l.cache_bytes > 0);
                clear_data.set_sensitive(l.data_bytes > 0 || l.cache_bytes > 0);
                running.set(l.running);
                // A Flatpak app's notification choice also lives in the desktop's permission store.
                if l.notifications_portal == Some(false) {
                    updating.set(true);
                    notif.set_active(false);
                }
                if let Some(ctx) = &l.permissions {
                    updating.set(true);
                    for (row, sw) in [(&network, ad::Switch::Network), (&sound, ad::Switch::Sound), (&graphics, ad::Switch::Graphics), (&devices, ad::Switch::AllDevices), (&bluetooth, ad::Switch::Bluetooth)] {
                        row.set_active(sw.allowed_in(ctx));
                        row.set_sensitive(true);
                    }
                    files.set_selected(match ad::files_level(ctx) {
                        ad::Files::Chosen => 0,
                        ad::Files::Home => 1,
                        ad::Files::Everything => 2,
                    });
                    files.set_sensitive(true);
                    camera.set_active(l.camera == Some(true));
                    camera.set_subtitle(if l.camera.is_none() { "Asks the first time it needs it" } else { "Allowed through the desktop" });
                    camera.set_sensitive(true);
                    microphone.set_active(l.microphone == Some(true));
                    microphone.set_subtitle(if l.microphone.is_none() { "Asks the first time it needs it" } else { "Allowed through the desktop" });
                    microphone.set_sensitive(true);
                    reset.set_sensitive(true);
                }
                updating.set(false);
            },
        );
    }

    // Notifications.
    {
        let (id, flatpak, updating) = (notif_id.clone(), target.flatpak_id().map(String::from), updating.clone());
        notif.connect_active_notify(move |r| {
            if updating.get() {
                return;
            }
            let on = r.is_active();
            let (id, flatpak) = (id.clone(), flatpak.clone());
            kconfig::spawn(move || {
                kconfig::write_notify("plasmanotifyrc", &["Applications", &id], "ShowPopups", if on { "true" } else { "false" });
                if let Some(app) = flatpak {
                    let _ = ad::set_portal(&app, ad::Portal::Notifications, on);
                }
            });
        });
    }

    // Permissions: each switch applies at once and goes back if Flatpak refuses.
    if let Some(app) = target.flatpak_id().map(String::from) {
        for (row, sw) in [(network.clone(), ad::Switch::Network), (sound.clone(), ad::Switch::Sound), (graphics.clone(), ad::Switch::Graphics), (devices.clone(), ad::Switch::AllDevices), (bluetooth.clone(), ad::Switch::Bluetooth)] {
            let (app, updating, dialog) = (app.clone(), updating.clone(), dialog.clone());
            row.connect_active_notify(move |r| {
                if updating.get() {
                    return;
                }
                let on = r.is_active();
                let (app, r2, updating, dialog) = (app.clone(), r.clone(), updating.clone(), dialog.clone());
                in_background(
                    move || ad::set_switch(&app, sw, on),
                    move |res| {
                        if let Err(e) = res {
                            updating.set(true);
                            r2.set_active(!on);
                            updating.set(false);
                            message(&dialog, "Couldn't change it", &e);
                        }
                    },
                );
            });
        }
        for (row, which) in [(camera.clone(), ad::Portal::Camera), (microphone.clone(), ad::Portal::Microphone)] {
            let (app, updating, dialog) = (app.clone(), updating.clone(), dialog.clone());
            row.connect_active_notify(move |r| {
                if updating.get() {
                    return;
                }
                let on = r.is_active();
                let (app, r2, updating, dialog) = (app.clone(), r.clone(), updating.clone(), dialog.clone());
                in_background(
                    move || ad::set_portal(&app, which, on),
                    move |res| {
                        if let Err(e) = res {
                            updating.set(true);
                            r2.set_active(!on);
                            updating.set(false);
                            message(&dialog, "Couldn't change it", &e);
                        }
                    },
                );
            });
        }
        {
            let (app, updating, dialog) = (app.clone(), updating.clone(), dialog.clone());
            files.connect_selected_notify(move |r| {
                if updating.get() {
                    return;
                }
                let level = match r.selected() {
                    1 => ad::Files::Home,
                    2 => ad::Files::Everything,
                    _ => ad::Files::Chosen,
                };
                let (app, dialog) = (app.clone(), dialog.clone());
                in_background(move || ad::set_files(&app, level), move |res| {
                    if let Err(e) = res {
                        message(&dialog, "Couldn't change it", &e);
                    }
                });
            });
        }
        {
            let (app, dialog) = (app.clone(), dialog.clone());
            reset.connect_clicked(move |_| {
                let d = adw::AlertDialog::new(
                    Some("Go back to the app's own permissions?"),
                    Some("Everything you changed for this app is undone, including the choice of graphics card."),
                );
                d.add_responses(&[("cancel", "Cancel"), ("reset", "Go back")]);
                d.set_response_appearance("reset", adw::ResponseAppearance::Destructive);
                d.set_close_response("cancel");
                let (app, dialog2) = (app.clone(), dialog.clone());
                d.connect_response(None, move |_, r| {
                    if r != "reset" {
                        return;
                    }
                    let (app, dialog3) = (app.clone(), dialog2.clone());
                    in_background(move || ad::reset_permissions(&app), move |res| match res {
                        Ok(()) => message(&dialog3, "Done", "Close and open the details again to see the app's own permissions. The app picks them up the next time it opens."),
                        Err(e) => message(&dialog3, "Couldn't reset", &e),
                    });
                });
                d.present(Some(&dialog));
            });
        }
    }

    // Clearing.
    {
        let t = target.clone();
        let (cache_label, clear_cache, dialog) = (cache_label.clone(), clear_cache.clone(), dialog.clone());
        clear_cache.clone().connect_clicked(move |b| {
            if t.flatpak_id().is_some_and(ad::flatpak_running) {
                message(&dialog, "Close the app first", "It is open now, and clearing its cache while it runs could confuse it.");
                return;
            }
            b.set_sensitive(false);
            let (kind, names) = (t.kind.clone(), t.folder_names());
            let (label, b2) = (cache_label.clone(), b.clone());
            in_background(
                move || -> Result<u64, String> {
                    match &kind {
                        Kind::Flatpak(id) => {
                            let dir = ad::flatpak_data_dir(id).ok_or("That app can't be changed.")?.join("cache");
                            if dir.is_dir() { ad::clear_contents(&dir) } else { Ok(0) }
                        }
                        _ => {
                            let (_, caches) = ad::native_dirs(&names);
                            let mut freed = 0;
                            for c in caches {
                                freed += ad::clear_contents(&c)?;
                            }
                            Ok(freed)
                        }
                    }
                },
                move |res| match res {
                    Ok(freed) => {
                        label.set_text(&format!("0 B (freed {})", ad::human_size(freed)));
                    }
                    Err(e) => {
                        label.set_text("Not all of it could be removed");
                        b2.set_sensitive(true);
                        let _ = e;
                    }
                },
            );
        });
    }
    if let Some(app) = target.flatpak_id().map(String::from) {
        let (data_label, cache_label, dialog, data_row) = (data_label.clone(), cache_label.clone(), dialog.clone(), data_row.clone());
        clear_data.connect_clicked(move |b| {
            if ad::flatpak_running(&app) {
                message(&dialog, "Close the app first", "It is open now. Close it, then clear its data.");
                return;
            }
            let Some(dir) = ad::flatpak_data_dir(&app) else { return };
            let size = ad::human_size(ad::dir_size(&dir));
            let d = adw::AlertDialog::new(
                Some("Delete this app's saved data?"),
                Some(&format!("This deletes {size} of settings, accounts, downloads and files that “{app}” saved. It can't be undone. The app itself stays installed and starts fresh next time.")),
            );
            d.add_responses(&[("cancel", "Cancel"), ("clear", "Delete the data")]);
            d.set_response_appearance("clear", adw::ResponseAppearance::Destructive);
            d.set_default_response(Some("cancel"));
            d.set_close_response("cancel");
            let (b2, data_label, cache_label, dialog2, dir) = (b.clone(), data_label.clone(), cache_label.clone(), dialog.clone(), dir.clone());
            let _ = &data_row;
            d.connect_response(None, move |_, r| {
                if r != "clear" {
                    return;
                }
                b2.set_sensitive(false);
                let (dir, b3, data_label, cache_label, dialog3) = (dir.clone(), b2.clone(), data_label.clone(), cache_label.clone(), dialog2.clone());
                in_background(
                    move || ad::clear_contents(&dir),
                    move |res| match res {
                        Ok(freed) => {
                            data_label.set_text(&format!("0 B (freed {})", ad::human_size(freed)));
                            cache_label.set_text("0 B");
                        }
                        Err(e) => {
                            b3.set_sensitive(true);
                            message(&dialog3, "Couldn't delete everything", &e);
                        }
                    },
                );
            });
            d.present(Some(&dialog));
        });
    }
}
