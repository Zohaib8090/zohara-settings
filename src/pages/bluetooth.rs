use gtk4::prelude::*;
use libadwaita as adw;
use adw::prelude::*;

pub fn build() -> gtk4::Widget {
    let prefs_page = adw::PreferencesPage::new();

    // ── Adapter ───────────────────────────────────────────────────────────────
    let adapter_group = adw::PreferencesGroup::new();
    adapter_group.set_title("Adapter");

    let bt_row = adw::SwitchRow::new();
    bt_row.set_title("Bluetooth");
    bt_row.set_subtitle("Discoverable as Zohara");

    let bt_row_clone = bt_row.clone();
    glib::spawn_future_local(async move {
        match crate::backend::dbus::bluez_adapter_powered().await {
            Ok(powered) => bt_row_clone.set_active(powered),
            Err(_) => bt_row_clone.set_subtitle("No Bluetooth adapter found"),
        }
    });

    bt_row.connect_active_notify(|row| {
        let on = row.is_active();
        glib::spawn_future_local(async move {
            let _ = crate::backend::dbus::bluez_set_adapter_power(on).await;
        });
    });

    adapter_group.add(&bt_row);

    // ── Devices (Paired + Nearby) ─────────────────────────────────────────────
    let devices_group = adw::PreferencesGroup::new();
    devices_group.set_title("Devices");

    let refresh_btn = gtk4::Button::builder()
        .icon_name("view-refresh-symbolic")
        .css_classes(vec!["flat".to_string()])
        .build();
    devices_group.set_header_suffix(Some(&refresh_btn));

    // Spinner shown while scanning
    let scan_row = adw::ActionRow::new();
    scan_row.set_title("Scanning for devices…");
    let spinner = gtk4::Spinner::new();
    spinner.start();
    scan_row.add_suffix(&spinner);
    devices_group.add(&scan_row);

    let devices_group_clone = devices_group.clone();
    let bt_row_clone_2 = bt_row.clone();
    let load_devices = move || {
        let devices_group = devices_group_clone.clone();
        let scan_row = scan_row.clone();
        let bt_row = bt_row_clone_2.clone();

        glib::spawn_future_local(async move {
            // Check if Bluetooth controller actually exists
            let show_res = tokio::process::Command::new("bluetoothctl")
                .arg("show")
                .output()
                .await;

            let has_controller = if let Ok(out) = show_res {
                let stdout = String::from_utf8_lossy(&out.stdout);
                !stdout.contains("No default controller available") && !stdout.is_empty()
            } else {
                false
            };

            devices_group.remove(&scan_row);

            if !has_controller {
                bt_row.set_subtitle("No Bluetooth adapter found (Virtual Machine / Missing Hardware)");
                bt_row.set_sensitive(false);
                let empty_row = adw::ActionRow::new();
                empty_row.set_title("No Bluetooth adapter detected");
                empty_row.set_subtitle("Virtual Machine or no Bluetooth hardware present on this system");
                devices_group.add(&empty_row);
                return;
            }

            // Controller exists, trigger background scan asynchronously
            let _ = tokio::process::Command::new("bluetoothctl")
                .args(["--timeout", "2", "scan", "on"])
                .spawn();

            // Query devices
            let result = tokio::process::Command::new("bluetoothctl")
                .args(["devices"])
                .output()
                .await;

            let mut count = 0;
            if let Ok(out) = result {
                let stdout = String::from_utf8_lossy(&out.stdout);
                for line in stdout.lines() {
                    let parts: Vec<&str> = line.splitn(3, ' ').collect();
                    if parts.len() < 3 { continue; }
                    let mac = parts[1];
                    let name = parts[2];
                    if name.is_empty() { continue; }
                    count += 1;

                    let info = tokio::process::Command::new("bluetoothctl")
                        .args(["info", mac])
                        .output()
                        .await
                        .ok()
                        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                        .unwrap_or_default();

                    let connected = info.contains("Connected: yes");
                    let paired = info.contains("Paired: yes");

                    let row = adw::ActionRow::new();
                    row.set_title(name);
                    row.set_subtitle(if connected {
                        "● Connected"
                    } else if paired {
                        "Paired"
                    } else {
                        "Discovered"
                    });

                    let mac_owned = mac.to_string();
                    if connected {
                        let btn = gtk4::Button::builder()
                            .label("Disconnect")
                            .valign(gtk4::Align::Center)
                            .build();
                        btn.connect_clicked(move |_| {
                            let mac = mac_owned.clone();
                            glib::spawn_future_local(async move {
                                let _ = tokio::process::Command::new("bluetoothctl")
                                    .args(["disconnect", &mac]).output().await;
                            });
                        });
                        row.add_suffix(&btn);
                    } else {
                        let btn = gtk4::Button::builder()
                            .label("Connect")
                            .css_classes(vec!["suggested-action".to_string()])
                            .valign(gtk4::Align::Center)
                            .build();
                        btn.connect_clicked(move |_| {
                            let mac = mac_owned.clone();
                            glib::spawn_future_local(async move {
                                if !mac.is_empty() {
                                    let _ = tokio::process::Command::new("bluetoothctl")
                                        .args(["pair", &mac]).output().await;
                                    let _ = tokio::process::Command::new("bluetoothctl")
                                        .args(["connect", &mac]).output().await;
                                }
                            });
                        });
                        row.add_suffix(&btn);
                    }

                    devices_group.add(&row);
                }
            }

            if count == 0 {
                let empty_row = adw::ActionRow::new();
                empty_row.set_title("No Bluetooth devices found or paired");
                empty_row.set_subtitle("Adapter active. Bring devices close and set to pair mode.");
                devices_group.add(&empty_row);
            }
        });
    };

    load_devices();
    refresh_btn.connect_clicked(move |_| load_devices());

    prefs_page.add(&adapter_group);
    prefs_page.add(&devices_group);

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&adw::HeaderBar::new());
    toolbar_view.set_content(Some(&prefs_page));
    toolbar_view.upcast()
}
