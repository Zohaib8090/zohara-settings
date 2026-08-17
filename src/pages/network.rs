use gtk4::prelude::*;
use libadwaita as adw;
use adw::prelude::*;
use std::rc::Rc;
use std::cell::{Cell, RefCell};

use crate::backend::network as net;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkOperation {
    Idle,
    Scanning,
    Connecting,
    Refreshing,
    TogglingWifi,
}

/// Utility to find the top-level GtkWindow for presenting dialogs cleanly.
/// Falls back to active application window if root widget window is unavailable.
fn find_parent_window(widget: &impl IsA<gtk4::Widget>) -> Option<gtk4::Window> {
    widget
        .root()
        .and_downcast::<gtk4::Window>()
        .or_else(|| {
            gtk4::Application::default().active_window().and_downcast::<gtk4::Window>()
        })
}

pub fn build() -> gtk4::Widget {
    let prefs_page = adw::PreferencesPage::new();

    // ── Centralized State Machine ─────────────────────────────────────────────
    let current_op = Rc::new(Cell::new(NetworkOperation::Idle));

    // ── Wi-Fi Toggle ──────────────────────────────────────────────────────────
    let wifi_group = adw::PreferencesGroup::new();
    wifi_group.set_title("Wireless");

    let wifi_row = adw::SwitchRow::new();
    wifi_row.set_title("Wi-Fi");
    wifi_row.set_subtitle("Loading…");

    let init_guard = Rc::new(Cell::new(true));

    // ── Available Networks ───────────────────────────────────────────────────
    let networks_group = adw::PreferencesGroup::new();
    networks_group.set_title("Available networks");

    let refresh_btn = gtk4::Button::builder()
        .icon_name("view-refresh-symbolic")
        .css_classes(vec!["flat".to_string()])
        .tooltip_text("Refresh network list")
        .build();
    networks_group.set_header_suffix(Some(&refresh_btn));

    let scan_row = adw::ActionRow::new();
    scan_row.set_title("Scanning for networks…");
    let spinner = gtk4::Spinner::new();
    spinner.start();
    scan_row.add_suffix(&spinner);

    let network_rows: Rc<RefCell<Vec<adw::ActionRow>>> = Rc::new(RefCell::new(Vec::new()));

    // Dedicated Refresh Controller
    let refresh_controller = Rc::new(RefreshController {
        current_op:     current_op.clone(),
        networks_group: networks_group.clone(),
        scan_row:       scan_row.clone(),
        network_rows:   network_rows.clone(),
        refresh_btn:    refresh_btn.clone(),
        wifi_row:       wifi_row.clone(),
    });

    // Connect Wi-Fi switch toggle signal with full state-machine serialization and auto-refresh
    let guard_for_toggle      = init_guard.clone();
    let row_for_toggle        = wifi_row.clone();
    let op_for_toggle         = current_op.clone();
    let controller_for_toggle = refresh_controller.clone();

    wifi_row.connect_active_notify(move |row| {
        if guard_for_toggle.get() {
            return;
        }

        if op_for_toggle.get() != NetworkOperation::Idle {
            return;
        }

        let enabled = row.is_active();
        let row_clone = row_for_toggle.clone();
        let guard_clone = guard_for_toggle.clone();
        let op_clone = op_for_toggle.clone();
        let controller_clone = controller_for_toggle.clone();

        op_clone.set(NetworkOperation::TogglingWifi);
        row.set_sensitive(false);

        glib::spawn_future_local(async move {
            match crate::backend::dbus::nm_set_wifi(enabled).await {
                Ok(()) => {
                    let actual_state = crate::backend::dbus::nm_wifi_enabled().await.unwrap_or(enabled);
                    guard_clone.set(true);
                    row_clone.set_active(actual_state);
                    row_clone.set_subtitle(if actual_state { "On" } else { "Off" });
                    guard_clone.set(false);

                    // Trigger scan, letting perform_refresh transition state Scanning -> Idle cleanly
                    controller_clone.perform_refresh(NetworkOperation::Scanning);
                }
                Err(e) => {
                    guard_clone.set(true);
                    row_clone.set_active(!enabled);
                    guard_clone.set(false);

                    let err_msg = format!("Failed to set Wi-Fi: {}", e);
                    row_clone.set_subtitle(&err_msg);

                    if let Some(win) = find_parent_window(&row_clone) {
                        let dialog = adw::AlertDialog::new(
                            Some("Wi-Fi Error"),
                            Some(&err_msg),
                        );
                        dialog.add_response("ok", "OK");
                        dialog.present(Some(&win));
                    }

                    op_clone.set(NetworkOperation::Idle);
                    row_clone.set_sensitive(true);
                }
            }
        });
    });

    let row_for_init = wifi_row.clone();
    let guard_for_init = init_guard.clone();
    row_for_init.set_sensitive(false);

    glib::spawn_future_local(async move {
        match crate::backend::dbus::nm_wifi_enabled().await {
            Ok(enabled) => {
                guard_for_init.set(true);
                row_for_init.set_active(enabled);
                row_for_init.set_subtitle(if enabled { "On" } else { "Off" });
                guard_for_init.set(false);
            }
            Err(e) => {
                row_for_init.set_subtitle(&format!("Unavailable: {}", e));
            }
        }
        row_for_init.set_sensitive(true);
    });

    wifi_group.add(&wifi_row);

    // Initial load
    refresh_controller.perform_refresh(NetworkOperation::Scanning);

    // Refresh button
    let controller_for_btn = refresh_controller.clone();
    let op_for_btn = current_op.clone();
    refresh_btn.connect_clicked(move |_| {
        if op_for_btn.get() == NetworkOperation::Idle {
            controller_for_btn.perform_refresh(NetworkOperation::Refreshing);
        }
    });

    // ── Ethernet ──────────────────────────────────────────────────────────────
    let eth_group = adw::PreferencesGroup::new();
    eth_group.set_title("Ethernet");

    let eth_row = adw::ActionRow::new();
    eth_row.set_title("Ethernet");
    eth_row.set_subtitle("Checking…");

    let eth_row_clone = eth_row.clone();
    glib::spawn_future_local(async move {
        match net::ethernet_status().await {
            Ok(ifaces) if ifaces.is_empty() => {
                eth_row_clone.set_subtitle("No Ethernet interface detected");
            }
            Ok(ifaces) => {
                let summary: Vec<String> = ifaces.iter().map(|i| {
                    if i.state == "connected" {
                        format!("{}: Connected ({})", i.device,
                            if i.connection.is_empty() { "Wired" } else { &i.connection })
                    } else {
                        format!("{}: {}", i.device, i.state)
                    }
                }).collect();
                eth_row_clone.set_subtitle(&summary.join("  •  "));
            }
            Err(e) => {
                eth_row_clone.set_subtitle(&e.user_message());
            }
        }
    });

    eth_group.add(&eth_row);

    prefs_page.add(&wifi_group);
    prefs_page.add(&networks_group);
    prefs_page.add(&eth_group);

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&adw::HeaderBar::new());
    toolbar_view.set_content(Some(&prefs_page));
    toolbar_view.upcast()
}

// ── Shared Refresh Controller ─────────────────────────────────────────────────

pub struct RefreshController {
    current_op:     Rc<Cell<NetworkOperation>>,
    networks_group: adw::PreferencesGroup,
    scan_row:       adw::ActionRow,
    network_rows:   Rc<RefCell<Vec<adw::ActionRow>>>,
    refresh_btn:    gtk4::Button,
    wifi_row:       adw::SwitchRow,
}

impl RefreshController {
    pub fn perform_refresh(self: &Rc<Self>, target_op: NetworkOperation) {
        self.current_op.set(target_op);
        self.wifi_row.set_sensitive(false);
        self.refresh_btn.set_sensitive(false);

        // Clear existing dynamic rows
        {
            let rows = self.network_rows.borrow();
            for r in rows.iter() {
                self.networks_group.remove(r);
            }
        }
        self.network_rows.borrow_mut().clear();
        self.networks_group.add(&self.scan_row);

        let controller = self.clone();

        glib::spawn_future_local(async move {
            let rescan_res = net::wifi_rescan().await;
            if let Err(ref e) = rescan_res {
                eprintln!("[NetworkUI] Rescan warning: {}", e);
            }

            let list_result = net::wifi_list().await;
            controller.networks_group.remove(&controller.scan_row);

            match list_result {
                Ok(networks) if networks.is_empty() => {
                    let empty_row = adw::ActionRow::new();
                    let wifi_active = controller.wifi_row.is_active();
                    if !wifi_active {
                        empty_row.set_title("Wi-Fi is turned off");
                        empty_row.set_subtitle("Turn on Wi-Fi to see available networks");
                    } else {
                        empty_row.set_title("No wireless networks found");
                        empty_row.set_subtitle("Ensure you are in range of a Wi-Fi network");
                    }
                    controller.networks_group.add(&empty_row);
                    controller.network_rows.borrow_mut().push(empty_row);
                }
                Ok(networks) => {
                    for network in &networks {
                        let row = build_network_row(
                            network,
                            &controller.current_op,
                            controller.clone(),
                        );
                        controller.networks_group.add(&row);
                        controller.network_rows.borrow_mut().push(row);
                    }
                }
                Err(e) => {
                    let err_row = adw::ActionRow::new();
                    err_row.set_title("Failed to scan networks");
                    err_row.set_subtitle(&e.user_message());
                    err_row.add_css_class("error");
                    controller.networks_group.add(&err_row);
                    controller.network_rows.borrow_mut().push(err_row);
                }
            }

            // Guaranteed cleanup path: unlock all controls and reset state to Idle
            controller.wifi_row.set_sensitive(true);
            controller.refresh_btn.set_sensitive(true);
            controller.current_op.set(NetworkOperation::Idle);
        });
    }
}

// ── Network Row Builder ───────────────────────────────────────────────────────

fn build_network_row(
    network:            &net::WifiNetwork,
    current_op:         &Rc<Cell<NetworkOperation>>,
    refresh_controller: Rc<RefreshController>,
) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(&network.ssid);
    row.set_subtitle(&format!(
        "{} • {}% signal{}",
        network.security_label(),
        network.signal,
        if network.active { " • Connected" } else { "" }
    ));
    row.set_activatable(!network.active);

    let icon_name = if network.active {
        "network-wireless-connected-symbolic"
    } else if network.signal > 70 {
        "network-wireless-signal-excellent-symbolic"
    } else if network.signal > 40 {
        "network-wireless-signal-good-symbolic"
    } else {
        "network-wireless-signal-weak-symbolic"
    };

    let icon = gtk4::Image::from_icon_name(icon_name);
    if network.active {
        icon.add_css_class("accent");
    }
    row.add_prefix(&icon);

    if network.active {
        return row;
    }

    let connect_btn = gtk4::Button::builder()
        .label("Connect")
        .css_classes(vec!["suggested-action".to_string()])
        .valign(gtk4::Align::Center)
        .build();

    let ssid          = network.ssid.clone();
    let bssid         = if network.bssid.is_empty() { None } else { Some(network.bssid.clone()) };
    let is_enterprise = network.is_enterprise();
    let is_secured    = !network.security.is_empty() && network.security != "--";

    let op_clone   = current_op.clone();
    let row_clone  = row.clone();
    let btn_clone  = connect_btn.clone();

    connect_btn.connect_clicked(move |_| {
        if op_clone.get() != NetworkOperation::Idle {
            return;
        }

        if is_enterprise {
            if let Some(win) = find_parent_window(&row_clone) {
                let dialog = adw::AlertDialog::new(
                    Some("Enterprise Network Unsupported"),
                    Some("Enterprise networks (802.1X) require active domain credentials or certificates, which are currently unsupported."),
                );
                dialog.add_response("ok", "OK");
                dialog.present(Some(&win));
            }
            return;
        }

        btn_clone.set_sensitive(false);
        btn_clone.set_label("Connecting…");
        row_clone.remove_css_class("error");

        let dialog_heading   = format!("Connect to \"{}\"", ssid);
        let ssid_for_connect = ssid.clone();
        let bssid_for_conn   = bssid.clone();

        let op          = op_clone.clone();
        let row_ref     = row_clone.clone();
        let btn_ref     = btn_clone.clone();
        let controller  = refresh_controller.clone();

        let execute_connect = move |password: Option<String>| {
            let ssid        = ssid_for_connect.clone();
            let bssid       = bssid_for_conn.clone();
            let op          = op.clone();
            let row_ref     = row_ref.clone();
            let btn_ref     = btn_ref.clone();
            let controller  = controller.clone();

            op.set(NetworkOperation::Connecting);

            glib::spawn_future_local(async move {
                let res = net::wifi_connect(&ssid, bssid.as_deref(), password.as_deref()).await;

                match res {
                    Ok(()) => {
                        // Sequence: Connecting -> Refreshing -> Idle
                        controller.perform_refresh(NetworkOperation::Refreshing);
                    }
                    Err(e) => {
                        btn_ref.set_sensitive(true);
                        btn_ref.set_label("Connect");
                        row_ref.add_css_class("error");

                        if let Some(win) = find_parent_window(&row_ref) {
                            let dialog = adw::AlertDialog::new(
                                Some("Connection Failed"),
                                Some(&e.user_message()),
                            );
                            dialog.add_response("ok", "OK");
                            dialog.present(Some(&win));
                        }

                        op.set(NetworkOperation::Idle);
                    }
                }
            });
        };

        if is_secured {
            let dialog = adw::AlertDialog::new(
                Some(&dialog_heading),
                Some("Enter the Wi-Fi password to connect."),
            );
            dialog.add_response("cancel", "Cancel");
            dialog.add_response("connect", "Connect");
            dialog.set_response_appearance("connect", adw::ResponseAppearance::Suggested);
            dialog.set_default_response(Some("connect"));
            dialog.set_close_response("cancel");

            let entry = gtk4::PasswordEntry::new();
            entry.set_show_peek_icon(true);
            entry.set_placeholder_text(Some("Password"));
            entry.set_margin_top(8);
            entry.set_margin_bottom(4);
            dialog.set_extra_child(Some(&entry));

            let btn_cancel = btn_clone.clone();
            let row_cancel = row_clone.clone();
            let op_cancel  = op_clone.clone();
            let exec_cb    = Rc::new(execute_connect);

            dialog.connect_response(None, move |_, response| {
                if response != "connect" {
                    // Cancelled: unlock button and reset state to Idle
                    btn_cancel.set_sensitive(true);
                    btn_cancel.set_label("Connect");
                    op_cancel.set(NetworkOperation::Idle);
                    return;
                }
                let pw = entry.text().to_string();
                if pw.is_empty() {
                    btn_cancel.set_sensitive(true);
                    btn_cancel.set_label("Connect");
                    row_cancel.add_css_class("error");
                    op_cancel.set(NetworkOperation::Idle);
                    return;
                }
                exec_cb(Some(pw));
            });

            if let Some(win) = find_parent_window(&row_clone) {
                dialog.present(Some(&win));
            } else {
                btn_clone.set_sensitive(true);
                btn_clone.set_label("Connect");
                op_clone.set(NetworkOperation::Idle);
            }
        } else {
            execute_connect(None);
        }
    });

    row.add_suffix(&connect_btn);
    row
}
