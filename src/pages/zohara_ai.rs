//! Zohara AI: install the app, open it, remove it. The AI itself (models, keys, chats, MCP, scheduled tasks) lives in the app.
//!
//! The page shows what it found on the computer when it was opened and after every button; nothing is claimed that was not
//! just checked (`pacman -Q`, the program file).

use crate::backend::worker::in_background;
use crate::backend::zohara_ai as ai;
use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use std::rc::Rc;

/// Zohara AI is only offered on the alpha channel (the package exists nowhere else yet).
pub fn visible() -> bool {
    crate::sysupdate::channel::current() == "alpha"
}

struct Ui {
    page: glib::WeakRef<gtk4::Box>,
    status_row: adw::ActionRow,
    status_icon: gtk4::Image,
    action_btn: gtk4::Button,
    details: gtk4::Box,
    version_row: adw::ActionRow,
}

fn message(w: &impl IsA<gtk4::Widget>, title: &str, body: &str) {
    let d = adw::AlertDialog::new(Some(title), Some(body));
    d.add_response("ok", "OK");
    d.present(Some(w));
}

fn refresh(ui: &Rc<Ui>) {
    let ui2 = ui.clone();
    in_background(
        || (ai::installed(), ai::version()),
        move |(installed, version)| render(&ui2, installed, version),
    );
}

fn render(ui: &Rc<Ui>, installed: bool, version: Option<String>) {
    ui.action_btn.set_sensitive(true);
    ui.details.set_visible(installed);
    ui.action_btn.set_visible(!installed);
    if installed {
        ui.status_row.set_title("Zohara AI is installed");
        ui.status_row.set_subtitle("Open it from here, from the application menu, or press Ctrl+Super+C for the quick window.");
        ui.status_icon.set_icon_name(Some("object-select-symbolic"));
        ui.version_row.set_subtitle(version.as_deref().unwrap_or("Unknown"));
    } else {
        ui.status_row.set_title("Zohara AI is not installed");
        ui.status_row.set_subtitle("The AI built into Zohara OS: chat, projects, scheduled tasks, and it can control the computer when you allow it.");
        ui.status_icon.set_icon_name(Some("dialog-information-symbolic"));
        ui.action_btn.set_label("Install Zohara AI");
    }
}

pub fn build() -> gtk4::Widget {
    let scroll = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .build();

    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
    root.set_margin_start(28);
    root.set_margin_end(28);
    root.set_margin_top(20);
    root.set_margin_bottom(32);

    root.append(&gtk4::Label::builder().label("Zohara AI").halign(gtk4::Align::Start).css_classes(vec!["win11-page-title".to_string()]).build());

    let status_group = adw::PreferencesGroup::new();
    let status_row = adw::ActionRow::new();
    status_row.set_title("Looking for Zohara AI…");
    status_row.set_subtitle_lines(0);
    let status_icon = gtk4::Image::from_icon_name("content-loading-symbolic");
    status_row.add_prefix(&status_icon);
    let action_btn = gtk4::Button::with_label("Install Zohara AI");
    action_btn.add_css_class("suggested-action");
    action_btn.set_valign(gtk4::Align::Center);
    action_btn.set_visible(false);
    status_row.add_suffix(&action_btn);
    status_group.add(&status_row);
    root.append(&status_group);

    let details = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
    details.set_visible(false);
    let group = adw::PreferencesGroup::new();
    group.set_title("The app");

    let open_row = adw::ActionRow::new();
    open_row.set_title("Open Zohara AI");
    open_row.set_subtitle("Models, keys, connected apps (MCP), scheduled tasks and the permission mode are set inside the app, under its Settings.");
    open_row.set_subtitle_lines(0);
    open_row.add_prefix(&gtk4::Image::from_icon_name("system-run-symbolic"));
    let open_btn = gtk4::Button::with_label("Open");
    open_btn.add_css_class("suggested-action");
    open_btn.set_valign(gtk4::Align::Center);
    open_row.add_suffix(&open_btn);
    group.add(&open_row);

    let version_row = adw::ActionRow::new();
    version_row.set_title("Version");
    version_row.set_subtitle("…");
    version_row.set_subtitle_selectable(true);
    version_row.add_prefix(&gtk4::Image::from_icon_name("help-about-symbolic"));
    group.add(&version_row);

    let remove_row = adw::ActionRow::new();
    remove_row.set_title("Remove Zohara AI");
    remove_row.set_subtitle("Removes the program. Your chats, projects and settings stay in your home folder, and come back if you install it again.");
    remove_row.set_subtitle_lines(0);
    remove_row.add_prefix(&gtk4::Image::from_icon_name("user-trash-symbolic"));
    let remove_btn = gtk4::Button::with_label("Remove");
    remove_btn.add_css_class("destructive-action");
    remove_btn.set_valign(gtk4::Align::Center);
    remove_row.add_suffix(&remove_btn);
    group.add(&remove_row);
    details.append(&group);
    root.append(&details);

    scroll.set_child(Some(&root));

    let ui = Rc::new(Ui { page: root.downgrade(), status_row, status_icon, action_btn: action_btn.clone(), details, version_row });

    {
        let ui = ui.clone();
        action_btn.connect_clicked(move |b| {
            b.set_sensitive(false);
            ui.status_row.set_subtitle("Installing… you may be asked for your password.");
            let ui = ui.clone();
            let b = b.clone();
            in_background(ai::install, move |res| {
                if let Err(e) = res {
                    message(&b, "Zohara AI could not be installed", &e);
                }
                refresh(&ui);
            });
        });
    }
    open_btn.connect_clicked(|b| {
        if let Err(e) = ai::open() {
            message(b, "Zohara AI did not start", &e);
        }
    });
    {
        let ui = ui.clone();
        remove_btn.connect_clicked(move |b| {
            let d = adw::AlertDialog::new(Some("Remove Zohara AI?"), Some("The program is removed. Your chats, projects and settings stay."));
            d.add_response("cancel", "Cancel");
            d.add_response("remove", "Remove");
            d.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
            d.set_default_response(Some("cancel"));
            d.set_close_response("cancel");
            let ui = ui.clone();
            let anchor = b.clone();
            d.connect_response(None, move |_, resp| {
                if resp != "remove" {
                    return;
                }
                let ui = ui.clone();
                let anchor = anchor.clone();
                in_background(ai::remove, move |res| {
                    if let Err(e) = res {
                        message(&anchor, "Zohara AI could not be removed", &e);
                    }
                    refresh(&ui);
                });
            });
            d.present(Some(b));
        });
    }

    refresh(&ui);
    let _ = ui.page.upgrade();
    scroll.upcast()
}
