//! Zohara Link: pair a phone, see which phones are connected, choose what each may do.
//!
//! The daemon (`zohara-linkd`, repo `zohara-link`) does the real work; this page asks it over its local socket
//! (`backend::zohara_link`) and shows what it says. Nothing here claims "connected" unless the daemon just answered.
//! Pairing is closed on the daemon until the person presses "Pair a phone", and the PIN is shown here and in a
//! notification on this computer, then typed on the phone: it never travels over the network. See
//! `zohara-link/docs/SECURITY.md` for what protects the connection.
//!
//! The page asks the daemon again every two seconds while it is visible and only rebuilds the phone list when it changed,
//! so a switch you are pressing is never replaced under your finger.

use crate::backend::worker::in_background;
use crate::backend::zohara_link as zl;
use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use serde_json::json;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    NotInstalled,
    NotRunning,
    Running,
}

struct Ui {
    /// Weak on purpose: the two-second timer holds `Ui`, and a strong reference would keep the page (and the timer) alive forever.
    page: glib::WeakRef<gtk4::Box>,
    status_row: adw::ActionRow,
    status_icon: gtk4::Image,
    action_btn: gtk4::Button,
    details: gtk4::Box,
    code_row: adw::ActionRow,
    pair_row: adw::ActionRow,
    pair_btn: gtk4::Button,
    pin_row: adw::ActionRow,
    devices_group: adw::PreferencesGroup,
    device_rows: RefCell<Vec<adw::ActionRow>>,
    signature: RefCell<String>,
    mode: Cell<Mode>,
    pairing_open: Cell<bool>,
    busy: Cell<bool>,
    refreshing: Cell<bool>,
}

fn message(w: &impl IsA<gtk4::Widget>, title: &str, body: &str) {
    let d = adw::AlertDialog::new(Some(title), Some(body));
    d.add_response("ok", "OK");
    d.present(Some(w));
}

fn tell(ui: &Ui, title: &str, body: &str) {
    if let Some(page) = ui.page.upgrade() {
        message(&page, title, body);
    }
}

fn refresh(ui: &Rc<Ui>) {
    if ui.busy.get() || ui.refreshing.get() {
        return;
    }
    ui.refreshing.set(true);
    let ui2 = ui.clone();
    in_background(
        || {
            let installed = zl::installed();
            let status = if installed { zl::status() } else { None };
            let identity = if status.is_some() { zl::identity() } else { None };
            (installed, status, identity)
        },
        move |(installed, status, identity)| {
            ui2.refreshing.set(false);
            render(&ui2, installed, status, identity);
        },
    );
}

fn render(ui: &Rc<Ui>, installed: bool, status: Option<zl::Status>, identity: Option<zl::Identity>) {
    let mode = match (installed, &status) {
        (false, _) => Mode::NotInstalled,
        (true, None) => Mode::NotRunning,
        (true, Some(_)) => Mode::Running,
    };
    ui.mode.set(mode);
    ui.details.set_visible(mode == Mode::Running);
    ui.action_btn.set_visible(mode != Mode::Running);
    ui.action_btn.set_sensitive(true);
    match mode {
        Mode::NotInstalled => {
            ui.status_row.set_title("Zohara Link is not set up");
            ui.status_row.set_subtitle("Connect your Android phone: share the clipboard, send files and see its notifications here.");
            ui.status_icon.set_icon_name(Some("network-offline-symbolic"));
            ui.action_btn.set_label("Set up Zohara Link");
        }
        Mode::NotRunning => {
            ui.status_row.set_title("Zohara Link is turned off");
            ui.status_row.set_subtitle("It is installed but not running.");
            ui.status_icon.set_icon_name(Some("network-offline-symbolic"));
            ui.action_btn.set_label("Turn on");
        }
        Mode::Running => {
            let status = status.unwrap_or_default();
            let connected = status.devices.iter().filter(|d| d.connected).count();
            ui.status_row.set_title("Zohara Link is on");
            ui.status_row.set_subtitle(&match connected {
                0 => "No phone connected right now.".to_string(),
                1 => "1 phone connected.".to_string(),
                n => format!("{n} phones connected."),
            });
            ui.status_icon.set_icon_name(Some("network-transmit-receive-symbolic"));

            if let Some(id) = identity {
                ui.code_row.set_subtitle(&id.code);
            }

            ui.pairing_open.set(status.pairing_open);
            ui.pair_btn.set_label(if status.pairing_open { "Stop pairing" } else { "Pair a phone" });
            ui.pair_row.set_subtitle(if status.pairing_open {
                "Open Zohara Link on your phone, pick this computer, and type the PIN shown here."
            } else {
                "Pairing is off until you press this, so nobody else on the Wi-Fi can ask to pair."
            });
            match status.pending.first() {
                Some(p) => {
                    ui.pin_row.set_visible(true);
                    ui.pin_row.set_title(&format!("PIN: {}", zl::pin_for_display(&p.pin)));
                    ui.pin_row.set_subtitle(&format!(
                        "{} is asking to pair. Type this PIN on the phone, and check that the phone shows the same code as \"This computer's code\" above.",
                        if p.name.is_empty() { "A phone" } else { &p.name }
                    ));
                }
                None => ui.pin_row.set_visible(false),
            }

            let signature = format!("{:?}", status.devices);
            if *ui.signature.borrow() != signature {
                *ui.signature.borrow_mut() = signature;
                rebuild_devices(ui, &status.devices);
            }
        }
    }
}

fn rebuild_devices(ui: &Rc<Ui>, devices: &[zl::Device]) {
    for row in ui.device_rows.borrow_mut().drain(..) {
        ui.devices_group.remove(&row);
    }
    if devices.is_empty() {
        let row = adw::ActionRow::new();
        row.set_title("No phone paired yet");
        row.set_subtitle("Press \"Pair a phone\" above, then open Zohara Link on your phone.");
        ui.devices_group.add(&row);
        ui.device_rows.borrow_mut().push(row);
        return;
    }
    for d in devices {
        let row = adw::ActionRow::new();
        row.set_title(&d.name);
        row.set_use_markup(false);
        row.set_subtitle(&format!("{}{}", if d.connected { "Connected" } else { "Not connected" }, if d.last_ip.is_empty() { String::new() } else { format!(" · last seen at {}", d.last_ip) }));
        row.add_prefix(&gtk4::Image::from_icon_name("phone-symbolic"));

        // Moving the mouse from a phone is the riskiest thing it can do, so it is off until chosen here.
        let input = gtk4::Switch::new();
        input.set_valign(gtk4::Align::Center);
        input.set_active(d.allow_input);
        input.set_tooltip_text(Some("Let this phone move the mouse and click"));
        let id = d.id.clone();
        input.connect_state_set(move |_, on| {
            let id = id.clone();
            in_background(move || zl::command_ok(json!({"command": "SET_PERMISSION", "deviceId": id, "input": on})), |_| {});
            glib::Propagation::Proceed
        });
        let input_label = gtk4::Label::new(Some("Control the mouse"));
        input_label.add_css_class("dim-label");
        row.add_suffix(&input_label);
        row.add_suffix(&input);

        let unpair = gtk4::Button::from_icon_name("user-trash-symbolic");
        unpair.add_css_class("flat");
        unpair.set_valign(gtk4::Align::Center);
        unpair.set_tooltip_text(Some("Unpair this phone"));
        let (ui2, id, name) = (ui.clone(), d.id.clone(), d.name.clone());
        unpair.connect_clicked(move |_| {
            let dlg = adw::AlertDialog::new(
                Some(&format!("Unpair {name}?")),
                Some("It is disconnected at once and will have to pair again with a new PIN."),
            );
            dlg.add_responses(&[("cancel", "Cancel"), ("unpair", "Unpair")]);
            dlg.set_response_appearance("unpair", adw::ResponseAppearance::Destructive);
            dlg.set_close_response("cancel");
            let (ui3, id) = (ui2.clone(), id.clone());
            dlg.connect_response(None, move |_, r| {
                if r != "unpair" {
                    return;
                }
                let (ui4, id) = (ui3.clone(), id.clone());
                in_background(
                    move || zl::command_ok(json!({"command": "UNPAIR", "deviceId": id})),
                    move |ok| {
                        if !ok {
                            tell(&ui4, "Could not unpair", "Zohara Link did not answer. Try again in a moment.");
                        }
                        ui4.signature.borrow_mut().clear();
                        refresh(&ui4);
                    },
                );
            });
            if let Some(page) = ui2.page.upgrade() {
                dlg.present(Some(&page));
            }
        });
        row.add_suffix(&unpair);
        ui.devices_group.add(&row);
        ui.device_rows.borrow_mut().push(row);
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

    let title = gtk4::Label::builder()
        .label("Zohara Link")
        .halign(gtk4::Align::Start)
        .css_classes(vec!["win11-page-title".to_string()])
        .build();
    root.append(&title);

    // Status, with the button that sets it up or turns it on.
    let status_group = adw::PreferencesGroup::new();
    let status_row = adw::ActionRow::new();
    status_row.set_title("Looking for Zohara Link…");
    let status_icon = gtk4::Image::from_icon_name("network-offline-symbolic");
    status_row.add_prefix(&status_icon);
    let action_btn = gtk4::Button::with_label("Set up Zohara Link");
    action_btn.add_css_class("suggested-action");
    action_btn.set_valign(gtk4::Align::Center);
    action_btn.set_visible(false);
    status_row.add_suffix(&action_btn);
    status_group.add(&status_row);
    root.append(&status_group);

    // Everything below only shows while the daemon is running.
    let details = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
    details.set_visible(false);

    let pair_group = adw::PreferencesGroup::new();
    pair_group.set_title("Pair a phone");
    let code_row = adw::ActionRow::new();
    code_row.set_title("This computer's code");
    code_row.set_subtitle("…");
    code_row.set_subtitle_selectable(true);
    code_row.add_prefix(&gtk4::Image::from_icon_name("security-high-symbolic"));
    pair_group.add(&code_row);
    let pair_row = adw::ActionRow::new();
    pair_row.set_title("Pair a phone");
    pair_row.set_subtitle_lines(0);
    let pair_btn = gtk4::Button::with_label("Pair a phone");
    pair_btn.add_css_class("suggested-action");
    pair_btn.set_valign(gtk4::Align::Center);
    pair_row.add_suffix(&pair_btn);
    pair_group.add(&pair_row);
    let pin_row = adw::ActionRow::new();
    pin_row.set_visible(false);
    pin_row.set_subtitle_lines(0);
    pin_row.set_use_markup(false);
    pin_row.add_css_class("accent");
    pair_group.add(&pin_row);
    details.append(&pair_group);

    let devices_group = adw::PreferencesGroup::new();
    devices_group.set_title("Your phones");
    details.append(&devices_group);

    let actions_group = adw::PreferencesGroup::new();
    actions_group.set_title("Quick actions");
    let clip_row = adw::ActionRow::new();
    clip_row.set_title("Send my clipboard to the phone");
    clip_row.set_subtitle("What you copied here appears on every connected phone's clipboard.");
    let clip_btn = gtk4::Button::with_label("Send");
    clip_btn.set_valign(gtk4::Align::Center);
    clip_row.add_suffix(&clip_btn);
    actions_group.add(&clip_row);
    details.append(&actions_group);
    root.append(&details);

    let ui = Rc::new(Ui {
        page: root.downgrade(),
        status_row,
        status_icon,
        action_btn: action_btn.clone(),
        details,
        code_row,
        pair_row,
        pair_btn: pair_btn.clone(),
        pin_row,
        devices_group,
        device_rows: RefCell::new(Vec::new()),
        signature: RefCell::new(String::new()),
        mode: Cell::new(Mode::NotRunning),
        pairing_open: Cell::new(false),
        busy: Cell::new(false),
        refreshing: Cell::new(false),
    });

    {
        let ui2 = ui.clone();
        action_btn.connect_clicked(move |b| {
            b.set_sensitive(false);
            ui2.busy.set(true);
            let mode = ui2.mode.get();
            ui2.status_row.set_subtitle(if mode == Mode::NotInstalled { "Setting up… you may be asked for your password." } else { "Turning on…" });
            let ui3 = ui2.clone();
            in_background(
                move || if mode == Mode::NotInstalled { zl::install_and_start() } else { zl::start() },
                move |res| {
                    ui3.busy.set(false);
                    if let Err(e) = res {
                        tell(&ui3, "Zohara Link could not be turned on", if e.is_empty() { "Something went wrong. Try again." } else { &e });
                    }
                    // The daemon needs a moment to open its socket.
                    let ui4 = ui3.clone();
                    glib::timeout_add_local_once(Duration::from_millis(1200), move || refresh(&ui4));
                },
            );
        });
    }
    {
        let ui2 = ui.clone();
        pair_btn.connect_clicked(move |b| {
            b.set_sensitive(false);
            let open = !ui2.pairing_open.get();
            let ui3 = ui2.clone();
            in_background(
                move || zl::command_ok(json!({"command": if open { "OPEN_PAIRING" } else { "CLOSE_PAIRING" }})),
                move |ok| {
                    ui3.pair_btn.set_sensitive(true);
                    if !ok {
                        tell(&ui3, "Could not change pairing", "Zohara Link did not answer. Try again in a moment.");
                    }
                    refresh(&ui3);
                },
            );
        });
    }
    {
        let ui2 = ui.clone();
        clip_btn.connect_clicked(move |_| {
            let Some(display) = gtk4::gdk::Display::default() else { return };
            let ui3 = ui2.clone();
            display.clipboard().read_text_async(None::<&gtk4::gio::Cancellable>, move |res| {
                let Ok(Some(text)) = res else {
                    tell(&ui3, "Nothing to send", "Copy some text first, then press Send.");
                    return;
                };
                let text = text.to_string();
                in_background(move || zl::command_ok(json!({"command": "SEND_CLIPBOARD", "text": text})), |_| {});
            });
        });
    }

    // Ask now, then every two seconds for as long as the page exists.
    refresh(&ui);
    let weak = root.downgrade();
    let ui_tick = ui.clone();
    glib::timeout_add_local(Duration::from_secs(2), move || {
        if weak.upgrade().is_none() {
            return glib::ControlFlow::Break;
        }
        refresh(&ui_tick);
        glib::ControlFlow::Continue
    });

    scroll.set_child(Some(&root));
    scroll.upcast()
}
