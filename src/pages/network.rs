use gtk4::prelude::*;
use libadwaita as adw;
use adw::prelude::*;
use gtk4::glib;
use std::process::Command;

use crate::backend::network::{self, WifiNetwork};
use crate::tokio_runtime;

pub fn build() -> gtk4::Widget {
    let scroll = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .build();

    let root_box = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
    root_box.set_margin_start(28);
    root_box.set_margin_end(28);
    root_box.set_margin_top(20);
    root_box.set_margin_bottom(32);

    // ── Page Title ────────────────────────────────────────────────────────────
    let title_lbl = gtk4::Label::builder()
        .label("Network & internet")
        .halign(gtk4::Align::Start)
        .css_classes(vec!["win11-page-title".to_string()])
        .build();
    root_box.append(&title_lbl);

    root_box.append(&super::network_extra::status_group());

    // ── Grouped Rows ──────────────────────────────────────────────────────────
    let rows_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    rows_box.set_css_classes(&["win11-card-group"]);

    // 1. Wi-Fi (Expander with live AP list + in-app password dialog)
    let wifi_exp = adw::ExpanderRow::new();
    wifi_exp.set_title("Wi-Fi");
    wifi_exp.set_subtitle("Connect, manage known networks, metered network");
    wifi_exp.add_prefix(&gtk4::Image::from_icon_name("network-wireless-symbolic"));
    wifi_exp.set_css_classes(&["win11-expander-row"]);

    let wifi_switch = gtk4::Switch::builder()
        .active(super::network_extra::wifi_enabled())
        .valign(gtk4::Align::Center)
        .build();
    wifi_switch.connect_state_set(|_, active| {
        let cmd = if active { "on" } else { "off" };
        let _ = Command::new("nmcli").args(["radio", "wifi", cmd]).spawn();
        glib::Propagation::Proceed
    });
    wifi_exp.add_suffix(&wifi_switch);

    let rescan_row = adw::ActionRow::new();
    rescan_row.set_title("Scan for Wi-Fi Networks");
    let rescan_btn = gtk4::Button::builder()
        .label("Scan")
        .css_classes(vec!["win11-secondary-btn".to_string()])
        .build();
    let rescan_row_clone = rescan_row.clone();

    // ── Shared populate cell ─────────────────────────────────────────────────
    // The populate work is stored in a single RefCell. The Scan button reads
    // it (via a clone) so that pressing Scan → populates the list. The
    // initial-load timer below also reads it. We declare it now so the
    // closure below can write the body; the connect_clicked closure below
    // captures a clone and reads it on every click.
    let populate_cell: std::rc::Rc<std::cell::RefCell<Option<Box<dyn Fn()>>>> =
        std::rc::Rc::new(std::cell::RefCell::new(None));
    let populate_cell_for_btn = populate_cell.clone();

    rescan_btn.connect_clicked(move |btn| {
        btn.set_sensitive(false);
        rescan_row_clone.set_subtitle("Scanning wireless access points…");

        // Rescan via a std::thread + mpsc. The result is consumed by the
        // GTK-side poller; on success we kick off a populate on the global
        // Tokio runtime.
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let ok = Command::new("nmcli")
                .args(["device", "wifi", "rescan"])
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            let _ = tx.send(ok);
        });

        let btn_c = btn.clone();
        let row_c = rescan_row_clone.clone();
        let cell_c = populate_cell_for_btn.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
            match rx.try_recv() {
                Ok(ok) => {
                    row_c.set_subtitle(if ok { "Scan complete" } else { "Scan failed" });
                    if ok {
                        // 1500ms grace period so nmcli flushes its new scan
                        // table to the cache before we read it.
                        let cell = cell_c.clone();
                        glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || {
                            if let Some(f) = cell.borrow().as_ref() {
                                f();
                            }
                        });
                    } else {
                        btn_c.set_sensitive(true);
                    }
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    btn_c.set_sensitive(true);
                    row_c.set_subtitle("Scan failed");
                    glib::ControlFlow::Break
                }
            }
        });
    });
    rescan_row.add_suffix(&rescan_btn);
    wifi_exp.add_row(&rescan_row);

    // ── Live AP list ──────────────────────────────────────────────────────────
    // Replaces the previous two hardcoded "net1" / "net2" rows. The list is
    // populated asynchronously by calling `backend::network::wifi_list` on the
    // global Tokio runtime, then re-rendering the expander rows from the
    // returned Vec<WifiNetwork>. A single loading row is shown while the
    // subprocess runs; the same row is replaced by per-network ActionRows
    // when the data arrives.
    let ap_list_row = adw::ActionRow::new();
    ap_list_row.set_title("Available networks");
    ap_list_row.set_subtitle("Click Scan above to list nearby Wi-Fi access points");
    wifi_exp.add_row(&ap_list_row);

    // The first 8 APs get their own ActionRow; if more are present, the user
    // can still see "and N more" in the subtitle. Putting a hard cap here
    // keeps the GTK layout from ballooning on a dense apartment block.
    const MAX_ROWS: usize = 8;

    let wifi_exp_for_populate = wifi_exp.clone();
    let ap_list_row_for_populate = ap_list_row.clone();
    let rescan_btn_for_repopulate = rescan_btn.clone();

    // Write the populate body into the shared cell. This block runs at
    // page-build time, so by the time the user presses Scan (or the
    // initial-load timer fires), the cell already holds the body.
    *populate_cell.borrow_mut() = Some(Box::new(move || {
        let ap_list_row = ap_list_row_for_populate.clone();
        let wifi_exp = wifi_exp_for_populate.clone();
        let rescan_btn = rescan_btn_for_repopulate.clone();

        // Mark the list as "loading". We don't remove the row here -- the
        // post-rescan callback uses this same state string to know when to
        // stop polling.
        ap_list_row.set_subtitle("Loading access points…");

        glib::spawn_future_local(async move {
            // Hop onto the global Tokio runtime. network::wifi_list already
            // wraps nmcli in a 10s timeout, so this future cannot hang.
            let result = tokio_runtime().spawn(async move {
                network::wifi_list().await
            }).await;

            match result {
                Ok(Ok(networks)) => {
                    let total = networks.len();
                    let shown = networks.into_iter().take(MAX_ROWS).collect::<Vec<_>>();

                    // Replace the loading row with per-AP rows. We can't
                    // mutate the existing row to be per-AP because GtkListBox
                    // rows are positionally stable and the placeholder already
                    // has the wrong subtitle.
                    wifi_exp.remove(&ap_list_row);

                    if total == 0 {
                        let empty = adw::ActionRow::new();
                        empty.set_title("No networks found");
                        empty.set_subtitle("Make sure Wi-Fi is on and try scanning again");
                        empty.add_prefix(&gtk4::Image::from_icon_name("dialog-information-symbolic"));
                        wifi_exp.add_row(&empty);
                    } else {
                        for ap in shown {
                            wifi_exp.add_row(&build_ap_row(ap));
                        }
                        if total > MAX_ROWS {
                            let more = adw::ActionRow::new();
                            more.set_title(&format!("…and {} more", total - MAX_ROWS));
                            more.set_subtitle("Use the Scan button to refresh");
                            wifi_exp.add_row(&more);
                        }
                    }
                    rescan_btn.set_sensitive(true);
                }
                Ok(Err(e)) => {
                    ap_list_row.set_subtitle(&format!("Error: {}", e.user_message()));
                    rescan_btn.set_sensitive(true);
                }
                Err(_) => {
                    ap_list_row.set_subtitle("Background task was cancelled");
                    rescan_btn.set_sensitive(true);
                }
            }
        });
    }));

    // Initial populate once at page build time. The rescan that happens
    // here is implicit: wifi_list reads nmcli's cached scan, which archiso's
    // live env populates shortly after NetworkManager comes up.
    let populate_cell_for_init = populate_cell.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(800), move || {
        if let Some(f) = populate_cell_for_init.borrow().as_ref() {
            f();
        }
        glib::ControlFlow::Break
    });

    rows_box.append(&wifi_exp);

    root_box.append(&rows_box);
    if let Some(eth) = super::network_extra::ethernet_group() {
        root_box.append(&eth);
    }
    root_box.append(&super::network_extra::radios_group(&root_box));
    root_box.append(&super::network_extra::vpn_group(&root_box));
    root_box.append(&super::speedtest::group());
    root_box.append(&super::network_extra::proxy_group());
    root_box.append(&super::network_extra::adapters_group());
    scroll.set_child(Some(&root_box));
    scroll.upcast()
}


/// Build an ActionRow for a single Wi-Fi access point.
///
/// Signal strength → icon: 0–24 "none", 25–49 "weak", 50–74 "ok", 75–100 "good"/"excellent".
/// Active networks get a green-tinted "Connected" subtitle and a disabled "Disconnect" button
/// (Disconnect is left for a future PR; Connect uses the typed backend, which knows the BSSID).
fn build_ap_row(ap: WifiNetwork) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(&ap.ssid);
    row.set_subtitle(&format!(
        "{} • Signal: {}%{}",
        ap.security_label(),
        ap.signal,
        if ap.active { " • Connected" } else { "" }
    ));

    let icon_name = if ap.active {
        "network-wireless-signal-excellent-symbolic"
    } else if ap.signal >= 75 {
        "network-wireless-signal-good-symbolic"
    } else if ap.signal >= 50 {
        "network-wireless-signal-ok-symbolic"
    } else if ap.signal >= 25 {
        "network-wireless-signal-weak-symbolic"
    } else {
        "network-wireless-signal-none-symbolic"
    };
    row.add_prefix(&gtk4::Image::from_icon_name(icon_name));

    if ap.is_enterprise() {
        // 802.1X networks can't be connected to without a per-network CA cert,
        // user cert, and identity string. The typed backend already rejects
        // EnterpriseNotSupported, but a UI marker is more honest than a
        // disabled button that does nothing when clicked.
        let ent = gtk4::Label::new(Some("Enterprise"));
        ent.add_css_class("win11-badge-pill");
        ent.set_valign(gtk4::Align::Center);
        row.add_suffix(&ent);
    } else if ap.active {
        // Don't show a Connect button on the active network; the user can
        // disconnect via the GNOME control center if they need to.
        let conn = gtk4::Label::new(Some("●"));
        conn.add_css_class("accent-green");
        conn.set_valign(gtk4::Align::Center);
        conn.set_margin_end(8);
        row.add_suffix(&conn);
    } else {
        // Connect button: opens a password dialog for secured networks, or
        // calls wifi_connect directly for open networks. Async; the row's
        // subtitle shows the in-flight state.
        let btn = gtk4::Button::builder()
            .label("Connect")
            .css_classes(vec!["win11-secondary-btn".to_string()])
            .valign(gtk4::Align::Center)
            .build();
        let row_for_click = row.clone();
        let ssid = ap.ssid.clone();
        let bssid = if ap.bssid.is_empty() { None } else { Some(ap.bssid.clone()) };
        let needs_password = !ap.security.is_empty() && ap.security != "--";
        btn.connect_clicked(move |btn| {
            if needs_password {
                prompt_for_password(&ssid, bssid.clone(), btn, &row_for_click);
            } else {
                connect_open(&ssid, bssid.clone(), btn, &row_for_click);
            }
        });
        row.add_suffix(&btn);
    }

    row
}

fn prompt_for_password(ssid: &str, bssid: Option<String>, btn: &gtk4::Button, row: &adw::ActionRow) {
    let parent = btn.root().and_downcast::<gtk4::Window>();
    let dialog = adw::MessageDialog::builder()
        .heading(&format!("Connect to {}", ssid))
        .body("Enter the Wi-Fi password:")
        .transient_for(parent.as_ref().unwrap_or(&gtk4::Window::new()))
        .build();
    let pass = gtk4::PasswordEntry::builder()
        .margin_top(8)
        .margin_bottom(8)
        .show_peek_icon(true)
        .build();
    dialog.set_extra_child(Some(&pass));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("connect", "Connect");
    dialog.set_response_appearance("connect", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("connect"));

    let ssid = ssid.to_string();
    let btn_w = btn.clone();
    let row_w = row.clone();
    dialog.connect_response(None, move |d, resp| {
        if resp == "connect" {
            let pw = pass.text().to_string();
            if pw.is_empty() {
                return; // ignore empty submissions
            }
            do_connect(&ssid, bssid.clone(), Some(pw), &btn_w, &row_w);
        }
        d.close();
    });
    dialog.present();
}

fn connect_open(ssid: &str, bssid: Option<String>, btn: &gtk4::Button, row: &adw::ActionRow) {
    do_connect(ssid, bssid, None, btn, row);
}

fn do_connect(ssid: &str, bssid: Option<String>, password: Option<String>, btn: &gtk4::Button, row: &adw::ActionRow) {
    btn.set_sensitive(false);
    row.set_subtitle("Connecting…");
    let ssid = ssid.to_string();
    let btn = btn.clone();
    let row = row.clone();
    let row_for_ok = row.clone();
    let row_for_err = row.clone();
    let btn_for_ok = btn.clone();
    let btn_for_err = btn.clone();
    glib::spawn_future_local(async move {
        let result = tokio_runtime().spawn(async move {
            network::wifi_connect(&ssid, bssid.as_deref(), password.as_deref()).await
        }).await;
        match result {
            Ok(Ok(())) => {
                row_for_ok.set_subtitle("Connected");
            }
            Ok(Err(e)) => {
                row_for_err.set_subtitle(&e.user_message());
                btn_for_err.set_sensitive(true);
            }
            Err(_) => {
                row.set_subtitle("Connection cancelled");
                btn.set_sensitive(true);
            }
        }
        // silence unused-warning when both paths re-enable the button
        let _ = btn_for_ok;
    });
}
