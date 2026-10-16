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
    address_row: adw::ActionRow,
    /// The first address, for the Copy button.
    first_address: RefCell<String>,
    remote_row: adw::ActionRow,
    remote_btn: gtk4::Button,
    /// The sign-in page of a login that is waiting, so the button can open it again without starting another.
    auth_url: RefCell<String>,
    /// What the remote button does now: 0 nothing, 1 set up Tailscale, 2 sign in, 3 sign out, 4 turn on.
    remote_action: Cell<u8>,
    pair_row: adw::ActionRow,
    pair_btn: gtk4::Button,
    pin_row: adw::ActionRow,
    clip_files: adw::SwitchRow,
    /// True while the page itself sets the switch (from the daemon's answer), so that is not taken for a click.
    syncing: Cell<bool>,
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
            let addresses = if status.is_some() { zl::local_addresses() } else { Vec::new() };
            let remote = if status.is_some() { zl::remote() } else { None };
            let ts = if status.is_some() { zl::ts_status() } else { None };
            (installed, status, identity, addresses, remote, ts)
        },
        move |(installed, status, identity, addresses, remote, ts)| {
            ui2.refreshing.set(false);
            render(&ui2, installed, status, identity, addresses);
            render_remote(&ui2, remote, ts);
        },
    );
}

fn render(ui: &Rc<Ui>, installed: bool, status: Option<zl::Status>, identity: Option<zl::Identity>, addresses: Vec<zl::Address>) {
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

            let port = identity.as_ref().map(|i| i.port).unwrap_or(42424);
            if let Some(id) = identity {
                ui.code_row.set_subtitle(&id.code);
            }
            // The address the phone can type if it does not find this computer by itself, so nobody has to dig it out of the Wi-Fi page.
            if addresses.is_empty() {
                *ui.first_address.borrow_mut() = String::new();
                ui.address_row.set_subtitle("No network connection found. Connect this computer to your Wi-Fi.");
            } else {
                *ui.first_address.borrow_mut() = addresses[0].ip.clone();
                let lines: Vec<String> = addresses
                    .iter()
                    .map(|a| format!("{}  ({}) · port {}", a.ip, zl::iface_kind(&a.iface), port))
                    .collect();
                ui.address_row.set_subtitle(&format!("{}\nIf the phone does not find this computer, type this address in the app.", lines.join("\n")));
            }

            ui.syncing.set(true);
            ui.clip_files.set_active(status.clipboard_files);
            ui.syncing.set(false);
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

/// "Away from home": shows the overlay addresses (Tailscale and the like) the phone can use outside the home Wi-Fi, or the one button
/// that gets the person there.
fn render_remote(ui: &Rc<Ui>, remote: Option<zl::Remote>, ts: Option<zl::TsInfo>) {
    let Some(r) = remote else { return };
    // Tailscale's own answer is trusted over the daemon's: it comes from the same moment.
    let state = ts.as_ref().map(|t| t.state.clone()).unwrap_or_else(|| r.tailscale_state.clone());
    *ui.auth_url.borrow_mut() = ts.as_ref().map(|t| t.auth_url.clone()).unwrap_or_default();
    let tailscale_up = state == "Running";
    if !r.overlays.is_empty() || tailscale_up {
        let lines: Vec<String> = r
            .overlays
            .iter()
            .map(|o| if o.dns_name.is_empty() { format!("{} · {}", o.address, o.provider) } else { format!("{} · {} ({})", o.address, o.provider, o.dns_name) })
            .collect();
        ui.remote_row.set_title("Away from home: ready");
        let provider = r.overlays.first().map(|o| o.provider.clone()).unwrap_or_else(|| "Tailscale".into());
        ui.remote_row.set_subtitle(&format!(
            "{}{}A phone you paired at home reaches this computer through this address when it is on the same {} account, with no extra steps.",
            lines.join("\n"),
            if lines.is_empty() { "" } else { "\n" },
            provider
        ));
        if tailscale_up {
            ui.remote_btn.set_label("Sign out");
            ui.remote_action.set(3);
        } else {
            ui.remote_action.set(0);
        }
    } else if !r.tailscale_installed {
        ui.remote_row.set_title("Away from home: not set up");
        ui.remote_row.set_subtitle("Install Tailscale here, and the Tailscale app on the phone with the same account, and Zohara Link works from any network. ZeroTier, NetBird and WireGuard are found automatically if you already use them.");
        ui.remote_btn.set_label("Set up Tailscale");
        ui.remote_action.set(1);
    } else if state == "Stopped" {
        ui.remote_row.set_title("Away from home: turned off");
        ui.remote_row.set_subtitle("You are signed in to Tailscale, but it is switched off on this computer.");
        ui.remote_btn.set_label("Turn on");
        ui.remote_action.set(4);
    } else if !ui.auth_url.borrow().is_empty() {
        ui.remote_row.set_title("Away from home: finish signing in");
        ui.remote_row.set_subtitle("Waiting for you to sign in in the browser. This page updates by itself when you are done. If the page did not open, press the button.");
        ui.remote_btn.set_label("Open the sign-in page");
        ui.remote_action.set(2);
    } else {
        ui.remote_row.set_title("Away from home: sign in");
        ui.remote_row.set_subtitle("Tailscale is installed but not connected. Sign in with the same account the phone uses.");
        ui.remote_btn.set_label("Sign in");
        ui.remote_action.set(2);
    }
    ui.remote_btn.set_visible(ui.remote_action.get() != 0);
    ui.remote_btn.set_sensitive(true);
}

/// Game streaming: the video stream from this computer to the phone (see zohara-link/docs/STREAMING.md). Two buttons: install what it
/// needs, and let the desktop share the screen with it once.
fn streaming_group() -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("Game streaming");
    g.set_description(Some("Watch and play this computer on the phone: live video with a game controller. A phone may only use it when you allow \"Show the screen\" and \"Control the mouse\" for it above."));

    let row = adw::ActionRow::new();
    row.set_title("Video stream");
    row.set_subtitle_lines(0);
    let btn = gtk4::Button::with_label("Set up game streaming");
    btn.add_css_class("suggested-action");
    btn.set_valign(gtk4::Align::Center);
    row.add_suffix(&btn);
    g.add(&row);

    let share = adw::ActionRow::new();
    share.set_title("Screen sharing");
    share.set_subtitle("The desktop asks once whether Zohara Link may share the screen. Press Allow, then press Share in the window that appears.");
    share.set_subtitle_lines(0);
    let share_btn = gtk4::Button::with_label("Allow screen sharing");
    share_btn.set_valign(gtk4::Align::Center);
    share.add_suffix(&share_btn);
    g.add(&share);

    let refresh: Rc<dyn Fn()> = {
        let (row, btn, share_btn) = (row.clone(), btn.clone(), share_btn.clone());
        Rc::new(move || {
            let st = zl::streaming_state();
            if st.missing.is_empty() {
                row.set_subtitle(&format!("Ready. Video encoder: {}.", st.encoders.join(", ")));
                btn.set_visible(false);
                share_btn.set_sensitive(true);
            } else {
                row.set_subtitle(&format!("Not set up. Missing: {}. Setting up installs the video parts (about 100 MB) and asks for your password.", st.missing.join(", ")));
                btn.set_visible(true);
                btn.set_sensitive(true);
                share_btn.set_sensitive(false);
            }
        })
    };
    refresh();
    {
        let (refresh, row) = (refresh.clone(), row.clone());
        btn.connect_clicked(move |b| {
            b.set_sensitive(false);
            row.set_subtitle("Installing… you may be asked for your password.");
            let (refresh, row, b) = (refresh.clone(), row.clone(), b.clone());
            in_background(zl::install_streaming, move |res| {
                if let Err(e) = res {
                    message(&b, "Game streaming could not be set up", if e.is_empty() { "Something went wrong. Try again." } else { &e });
                }
                let _ = &row;
                refresh();
            });
        });
    }
    {
        let share = share.clone();
        share_btn.connect_clicked(move |b| {
            b.set_sensitive(false);
            share.set_subtitle("Waiting for you to press Share in the desktop's window…");
            let (b, share) = (b.clone(), share.clone());
            in_background(zl::grant_screen, move |res| {
                b.set_sensitive(true);
                match res {
                    Ok(()) => share.set_subtitle("Allowed. It is remembered, so the desktop does not ask again."),
                    Err(e) => {
                        share.set_subtitle("The desktop asks once whether Zohara Link may share the screen. Press Allow, then press Share in the window that appears.");
                        message(&b, "Screen sharing was not allowed", &e);
                    }
                }
            });
        });
    }
    g
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
        // Seeing the screen shows everything on it, so it has its own switch, off until chosen here.
        let screen = gtk4::Switch::new();
        screen.set_valign(gtk4::Align::Center);
        screen.set_active(d.allow_screen);
        screen.set_tooltip_text(Some("Let this phone see this computer's screen"));
        let id = d.id.clone();
        screen.connect_state_set(move |_, on| {
            let id = id.clone();
            in_background(move || zl::command_ok(json!({"command": "SET_PERMISSION", "deviceId": id, "screen": on})), |_| {});
            glib::Propagation::Proceed
        });
        let screen_label = gtk4::Label::new(Some("Show the screen"));
        screen_label.add_css_class("dim-label");
        row.add_suffix(&screen_label);
        row.add_suffix(&screen);
        let input_label = gtk4::Label::new(Some("Control the mouse"));
        input_label.add_css_class("dim-label");
        row.add_suffix(&input_label);
        row.add_suffix(&input);

        // laptop to phone: files or a folder; the phone has to accept before anything is sent
        let send = gtk4::Button::from_icon_name("document-send-symbolic");
        let send_folder = gtk4::Button::from_icon_name("folder-symbolic");
        for (btn, tip) in [(&send, "Send files to this phone"), (&send_folder, "Send a folder to this phone")] {
            btn.add_css_class("flat");
            btn.set_valign(gtk4::Align::Center);
            btn.set_sensitive(d.connected);
            btn.set_tooltip_text(Some(if d.connected { tip } else { "Connect the phone first" }));
        }
        // sends every chosen path in the background and tells the person once
        let offer = {
            let (ui, id, name) = (ui.clone(), d.id.clone(), d.name.clone());
            Rc::new(move |paths: Vec<std::path::PathBuf>| {
                if paths.is_empty() {
                    return;
                }
                let (ui2, name2, id2) = (ui.clone(), name.clone(), id.clone());
                in_background(
                    move || {
                        paths
                            .iter()
                            .map(|p| {
                                let shown = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                                (shown, zl::send_file(&id2, &p.to_string_lossy()))
                            })
                            .collect::<Vec<_>>()
                    },
                    move |results| {
                        let (title, body) = zl::send_summary(&results, &name2);
                        tell(&ui2, &title, &body);
                    },
                );
            })
        };
        {
            let offer = offer.clone();
            send.connect_clicked(move |b| {
                let dialog = gtk4::FileDialog::new();
                dialog.set_title("Choose files to send to the phone");
                let offer = offer.clone();
                dialog.open_multiple(b.root().and_downcast_ref::<gtk4::Window>(), None::<&gtk4::gio::Cancellable>, move |res| {
                    let Ok(files) = res else { return };
                    let paths: Vec<std::path::PathBuf> = (0..files.n_items()).filter_map(|i| files.item(i).and_downcast::<gtk4::gio::File>()).filter_map(|f| f.path()).collect();
                    offer(paths);
                });
            });
        }
        {
            let offer = offer.clone();
            send_folder.connect_clicked(move |b| {
                let dialog = gtk4::FileDialog::new();
                dialog.set_title("Choose a folder to send to the phone");
                let offer = offer.clone();
                dialog.select_folder(b.root().and_downcast_ref::<gtk4::Window>(), None::<&gtk4::gio::Cancellable>, move |res| {
                    if let Some(path) = res.ok().and_then(|f| f.path()) {
                        offer(vec![path]);
                    }
                });
            });
        }
        row.add_suffix(&send_folder);
        row.add_suffix(&send);

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
    let address_row = adw::ActionRow::new();
    address_row.set_title("This computer's address");
    address_row.set_subtitle("…");
    address_row.set_subtitle_lines(0);
    address_row.set_subtitle_selectable(true);
    address_row.add_prefix(&gtk4::Image::from_icon_name("network-wired-symbolic"));
    let copy_btn = gtk4::Button::with_label("Copy");
    copy_btn.set_valign(gtk4::Align::Center);
    address_row.add_suffix(&copy_btn);
    pair_group.add(&address_row);
    let remote_row = adw::ActionRow::new();
    remote_row.set_title("Away from home");
    remote_row.set_subtitle("…");
    remote_row.set_subtitle_lines(0);
    remote_row.set_use_markup(false);
    remote_row.set_subtitle_selectable(true);
    remote_row.add_prefix(&gtk4::Image::from_icon_name("network-vpn-symbolic"));
    let remote_btn = gtk4::Button::with_label("Set up Tailscale");
    remote_btn.add_css_class("suggested-action");
    remote_btn.set_valign(gtk4::Align::Center);
    remote_btn.set_visible(false);
    remote_row.add_suffix(&remote_btn);
    pair_group.add(&remote_row);
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
    let clip_files = adw::SwitchRow::new();
    clip_files.set_title("Offer copied files, folders and pictures");
    clip_files.set_subtitle("Copy a file, a folder or a picture here and every connected phone is asked if it wants it. Nothing is saved on the phone until its owner presses Accept.");
    clip_files.set_subtitle_lines(0);
    actions_group.add(&clip_files);
    details.append(&actions_group);
    details.append(&streaming_group());
    root.append(&details);

    let ui = Rc::new(Ui {
        page: root.downgrade(),
        status_row,
        status_icon,
        action_btn: action_btn.clone(),
        details,
        code_row,
        address_row,
        first_address: RefCell::new(String::new()),
        remote_row,
        remote_btn: remote_btn.clone(),
        auth_url: RefCell::new(String::new()),
        remote_action: Cell::new(0),
        pair_row,
        pair_btn: pair_btn.clone(),
        pin_row,
        clip_files: clip_files.clone(),
        syncing: Cell::new(false),
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
        clip_files.connect_active_notify(move |row| {
            if ui2.syncing.get() {
                return;
            }
            let (on, ui3, row) = (row.is_active(), ui2.clone(), row.clone());
            in_background(move || zl::set_clipboard_files(on), move |ok| {
                if !ok {
                    // the daemon did not take it: put the switch back
                    ui3.syncing.set(true);
                    row.set_active(!on);
                    ui3.syncing.set(false);
                    tell(&ui3, "Zohara Link did not answer", "Make sure Zohara Link is turned on, then try again.");
                }
            });
        });
    }

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
        remote_btn.connect_clicked(move |b| {
            b.set_sensitive(false);
            let ui3 = ui2.clone();
            match ui2.remote_action.get() {
                1 => {
                    ui2.remote_row.set_subtitle("Installing Tailscale… you may be asked for your password.");
                    in_background(zl::install_tailscale, move |res| {
                        if let Err(e) = res {
                            tell(&ui3, "Tailscale could not be set up", if e.is_empty() { "Something went wrong. Try again." } else { &e });
                        }
                        ui3.remote_btn.set_sensitive(true);
                        refresh(&ui3);
                    });
                }
                2 => {
                    // a sign-in that is already waiting is opened again, not restarted
                    let waiting = ui2.auth_url.borrow().clone();
                    if waiting.starts_with("https://") {
                        let _ = gtk4::gio::AppInfo::launch_default_for_uri(&waiting, None::<&gtk4::gio::AppLaunchContext>);
                        ui2.remote_btn.set_sensitive(true);
                        return;
                    }
                    ui2.remote_row.set_subtitle("Asking Tailscale for the sign-in page…");
                    in_background(zl::ts_login, move |res| {
                        match res {
                            zl::Login::Url(u) => {
                                let _ = gtk4::gio::AppInfo::launch_default_for_uri(&u, None::<&gtk4::gio::AppLaunchContext>);
                            }
                            zl::Login::SignedIn => {}
                            zl::Login::Failed(e) => tell(&ui3, "Couldn't sign in", &e),
                        }
                        ui3.remote_btn.set_sensitive(true);
                        refresh(&ui3);
                    });
                }
                3 => {
                    // signing out stops phones reaching this computer from outside, so it is asked first
                    let d = adw::AlertDialog::new(
                        Some("Sign out of Tailscale?"),
                        Some("This computer leaves your Tailscale account. A phone can then only reach it on the home Wi-Fi until you sign in again. Tailscale stays installed."),
                    );
                    d.add_response("cancel", "Cancel");
                    d.add_response("out", "Sign out");
                    d.set_response_appearance("out", adw::ResponseAppearance::Destructive);
                    let ui4 = ui3.clone();
                    d.connect_response(None, move |_, resp| {
                        if resp != "out" {
                            ui4.remote_btn.set_sensitive(true);
                            return;
                        }
                        ui4.remote_row.set_subtitle("Signing out…");
                        let ui5 = ui4.clone();
                        in_background(zl::ts_logout, move |res| {
                            if let Err(e) = res {
                                tell(&ui5, "Couldn't sign out", &e);
                            }
                            ui5.remote_btn.set_sensitive(true);
                            refresh(&ui5);
                        });
                    });
                    if let Some(page) = ui3.page.upgrade() {
                        d.present(Some(&page));
                    }
                }
                4 => {
                    ui2.remote_row.set_subtitle("Turning on…");
                    in_background(zl::ts_up, move |res| {
                        if let Err(e) = res {
                            tell(&ui3, "Couldn't turn Tailscale on", &e);
                        }
                        ui3.remote_btn.set_sensitive(true);
                        refresh(&ui3);
                    });
                }
                _ => b.set_sensitive(true),
            }
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
        copy_btn.connect_clicked(move |_| {
            let ip = ui2.first_address.borrow().clone();
            if ip.is_empty() {
                return;
            }
            if let Some(display) = gtk4::gdk::Display::default() {
                display.clipboard().set_text(&ip);
            }
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
