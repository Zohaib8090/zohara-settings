//! Zohara Link — phone-pairing status and controls, embedded directly in
//! Settings rather than as a separate app (see docs/UI-REDESIGN.md for why).
//!
//! This is a real Unix-socket IPC client against zohara-linkd
//! (Zohaib8090/zohara-link's linux-daemon), not a mockup: it connects to
//! `/run/user/$UID/zohara.sock`, sends `GET_STATUS`, and stays connected to
//! receive live `DEVICE_PAIRED` / `PAIR_REQUEST` / `TELEMETRY_UPDATED`
//! events — see that repo's README ("Unix Socket IPC Integration") and
//! `zohara-link-status` for the protocol this mirrors. When the socket
//! doesn't exist or the connection drops, the page says so plainly instead
//! of showing a fake "Connected" — that honest state IS the "little bit of
//! verification system" this page needed: a control that only ever claims
//! to be live when it actually is.

use gtk4::prelude::*;
use libadwaita as adw;
use adw::prelude::*;
use serde_json::Value;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

fn socket_path() -> PathBuf {
    let uid = unsafe {
        extern "C" {
            fn geteuid() -> u32;
        }
        geteuid()
    };
    PathBuf::from(format!("/run/user/{uid}/zohara.sock"))
}

/// Fire a one-shot command line at the daemon on its own short-lived
/// connection, matching the protocol's one-JSON-line-per-command shape.
/// We don't wait for the reply — SEND_CLIPBOARD/LOCK_SCREEN both just
/// return `{"status":"OK"}`, and the visible effect (phone locks, phone
/// clipboard updates) is confirmation enough for a settings-page button.
async fn send_command(cmd: Value) {
    let path = socket_path();
    let Ok(mut stream) = UnixStream::connect(&path).await else {
        log::warn!("zohara-linkd not reachable at {}", path.display());
        return;
    };
    let mut line = serde_json::to_string(&cmd).unwrap_or_default();
    line.push('\n');
    let _ = stream.write_all(line.as_bytes()).await;
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

    // Connection status — reflects a real connection attempt, updated below.
    let status_group = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    status_group.set_css_classes(&["win11-card-group"]);
    let status_row = adw::ActionRow::new();
    status_row.set_title("Daemon connection");
    status_row.set_subtitle("Connecting…");
    let status_icon = gtk4::Image::from_icon_name("network-offline-symbolic");
    status_row.add_prefix(&status_icon);
    status_group.append(&status_row);
    root.append(&status_group);

    // Paired devices
    let devices_label = gtk4::Label::builder()
        .label("Paired devices")
        .halign(gtk4::Align::Start)
        .css_classes(vec!["win11-section-header".to_string()])
        .build();
    root.append(&devices_label);

    let devices_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    devices_box.set_css_classes(&["win11-card-group"]);
    let empty_row = adw::ActionRow::new();
    empty_row.set_title("No devices paired yet");
    empty_row.set_subtitle("Open the Zohara Link app on your phone to pair");
    devices_box.append(&empty_row);
    root.append(&devices_box);

    // Quick actions — disabled until we actually have a live connection.
    let actions_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
    let clip_btn = gtk4::Button::builder()
        .label("Send clipboard to phone")
        .css_classes(vec!["win11-secondary-btn".to_string()])
        .build();
    clip_btn.set_sensitive(false);
    let lock_btn = gtk4::Button::builder()
        .label("Lock phone screen")
        .css_classes(vec!["win11-secondary-btn".to_string()])
        .build();
    lock_btn.set_sensitive(false);
    actions_row.append(&clip_btn);
    actions_row.append(&lock_btn);
    root.append(&actions_row);

    clip_btn.connect_clicked(|_| {
        if let Some(display) = gtk4::gdk::Display::default() {
            let clipboard = display.clipboard();
            clipboard.read_text_async(None::<&gtk4::gio::Cancellable>, |res| {
                if let Ok(Some(text)) = res {
                    let text = text.to_string();
                    glib::spawn_future_local(async move {
                        send_command(serde_json::json!({"command": "SEND_CLIPBOARD", "text": text})).await;
                    });
                }
            });
        }
    });
    lock_btn.connect_clicked(|_| {
        glib::spawn_future_local(async move {
            send_command(serde_json::json!({"command": "LOCK_SCREEN"})).await;
        });
    });

    // ---- Live status: one long-running local future, pinned to the GTK
    // main thread's GLib context (same pattern zohara-apps/update uses for
    // its pacman calls) so every widget touch below is on the right thread
    // with no cross-thread channel needed. Tokio I/O (the Unix socket)
    // still works here because main.rs enters the Tokio runtime for the
    // whole process before starting the GTK main loop.
    {
        let status_row = status_row.clone();
        let status_icon = status_icon.clone();
        let devices_box = devices_box.clone();
        let empty_row = empty_row.clone();
        let clip_btn = clip_btn.clone();
        let lock_btn = lock_btn.clone();

        glib::spawn_future_local(async move {
            let path = socket_path();
            if !path.exists() {
                status_row.set_subtitle("Zohara Link daemon isn't running");
                return;
            }
            let stream = match UnixStream::connect(&path).await {
                Ok(s) => s,
                Err(e) => {
                    status_row.set_subtitle(&format!("Couldn't connect: {e}"));
                    return;
                }
            };
            let (read_half, mut write_half) = tokio::io::split(stream);
            if write_half
                .write_all(b"{\"command\":\"GET_STATUS\"}\n")
                .await
                .is_err()
            {
                status_row.set_subtitle("Zohara Link daemon isn't running");
                return;
            }

            let mut lines = BufReader::new(read_half).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        let Ok(v) = serde_json::from_str::<Value>(&line) else {
                            continue;
                        };
                        if let Some(event) = v.get("event").and_then(Value::as_str) {
                            // A live event arrived — re-request isn't needed
                            // for DEVICE_PAIRED specifically, but simplest
                            // correct behaviour for any state-changing event
                            // is to just say so; PAIR_REQUEST/telemetry are
                            // reflected on the next GET_STATUS-shaped line
                            // the daemon happens to send this connection.
                            status_row.set_subtitle(&format!("Connected · last event: {event}"));
                        } else {
                            // GET_STATUS response: {"paired_devices":[...], "telemetry":{...}, ...}
                            status_row.set_subtitle("Connected to zohara-linkd");
                            status_icon.set_icon_name(Some("network-transmit-receive-symbolic"));
                            clip_btn.set_sensitive(true);
                            lock_btn.set_sensitive(true);

                            let paired = v
                                .get("paired_devices")
                                .and_then(Value::as_array)
                                .cloned()
                                .unwrap_or_default();

                            while let Some(child) = devices_box.first_child() {
                                devices_box.remove(&child);
                            }
                            if paired.is_empty() {
                                devices_box.append(&empty_row);
                            } else {
                                for dev in &paired {
                                    let name = dev
                                        .get("device_name")
                                        .and_then(Value::as_str)
                                        .unwrap_or("Unknown device");
                                    let ip = dev.get("last_ip").and_then(Value::as_str).unwrap_or("");
                                    let row = adw::ActionRow::new();
                                    row.set_title(name);
                                    row.set_subtitle(&format!("Paired · {ip}"));
                                    row.add_prefix(&gtk4::Image::from_icon_name("phone-symbolic"));
                                    devices_box.append(&row);
                                }
                            }
                        }
                    }
                    _ => {
                        status_row.set_subtitle("Zohara Link daemon isn't running");
                        status_icon.set_icon_name(Some("network-offline-symbolic"));
                        clip_btn.set_sensitive(false);
                        lock_btn.set_sensitive(false);
                        break;
                    }
                }
            }
        });
    }

    scroll.set_child(Some(&root));
    scroll.upcast()
}
