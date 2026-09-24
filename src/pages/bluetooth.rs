//! Bluetooth & devices: the adapter's real power state, paired devices with
//! connect / disconnect / forget and battery level, scanning and pairing new
//! devices (Plasma's Bluedevil agent handles any PIN confirmation), connected
//! cameras, and shortcuts to the input-device pages.
//!
//! Uses `bluetoothctl` (BlueZ) for everything; every action runs off the UI
//! thread and the list is re-read afterwards, so what's shown is real.

use crate::backend::worker::in_background;
use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use std::cell::RefCell;
use std::process::Command;
use std::rc::Rc;

#[derive(Clone)]
struct Device {
    mac: String,
    name: String,
    icon: String,
    paired: bool,
    connected: bool,
    battery: Option<u32>,
}

fn btctl(args: &[&str]) -> Option<String> {
    let o = Command::new("bluetoothctl").args(args).output().ok()?;
    Some(String::from_utf8_lossy(&o.stdout).to_string())
}

fn btctl_ok(args: &[&str]) -> bool {
    Command::new("bluetoothctl")
        .args(args)
        .output()
        .map(|o| {
            let out = String::from_utf8_lossy(&o.stdout);
            o.status.success() && !out.contains("Failed") && !out.contains("not available")
        })
        .unwrap_or(false)
}

fn has_adapter() -> bool {
    btctl(&["list"]).map_or(false, |s| s.lines().any(|l| l.starts_with("Controller")))
}

fn powered() -> bool {
    btctl(&["show"]).map_or(false, |s| s.lines().any(|l| l.trim() == "Powered: yes"))
}

fn info(mac: &str) -> Option<Device> {
    let text = btctl(&["info", mac])?;
    let field = |k: &str| text.lines().find_map(|l| l.trim().strip_prefix(k).map(|v| v.trim().to_string()));
    Some(Device {
        mac: mac.to_string(),
        name: field("Name:").or_else(|| field("Alias:")).unwrap_or_else(|| mac.to_string()),
        icon: field("Icon:").unwrap_or_default(),
        paired: field("Paired:").as_deref() == Some("yes"),
        connected: field("Connected:").as_deref() == Some("yes"),
        // "Battery Percentage: 0x55 (85)"
        battery: field("Battery Percentage:")
            .and_then(|v| v.rsplit('(').next().map(|x| x.trim_end_matches(')').to_string()))
            .and_then(|v| v.parse().ok()),
    })
}

fn device_macs(filter: Option<&str>) -> Vec<String> {
    let mut args = vec!["devices"];
    if let Some(f) = filter {
        args.push(f);
    }
    btctl(&args)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.strip_prefix("Device ")?.split_whitespace().next().map(str::to_string))
        .collect()
}

/// Pair through BlueZ's D-Bus API rather than `bluetoothctl pair`: a call
/// from a client without its own agent is handled by the default agent
/// (Plasma's Bluedevil), which shows the PIN/confirmation prompt.
fn pair(mac: &str) -> bool {
    use std::collections::HashMap;
    use zbus::zvariant::{OwnedObjectPath, OwnedValue};
    type Objects = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;
    crate::backend::worker::block_on(async {
        let conn = zbus::Connection::system().await.ok()?;
        let reply = conn
            .call_method(Some("org.bluez"), "/", Some("org.freedesktop.DBus.ObjectManager"), "GetManagedObjects", &())
            .await
            .ok()?;
        let objects: Objects = reply.body().deserialize().ok()?;
        let path = objects.into_iter().find_map(|(path, ifaces)| {
            let addr = ifaces.get("org.bluez.Device1")?.get("Address")?;
            let addr: String = addr.try_clone().ok()?.try_into().ok()?;
            addr.eq_ignore_ascii_case(mac).then_some(path)
        })?;
        conn.call_method(Some("org.bluez"), path.as_str(), Some("org.bluez.Device1"), "Pair", &())
            .await
            .ok()
            .map(|_| ())
    })
    .is_some()
}

fn load_devices() -> Vec<Device> {
    let mut v: Vec<Device> = device_macs(None).iter().filter_map(|m| info(m)).collect();
    v.sort_by(|a, b| b.connected.cmp(&a.connected).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    v
}

fn device_icon(d: &Device) -> String {
    let base = match d.icon.as_str() {
        "audio-headset" | "audio-headphones" | "audio-card" => d.icon.as_str(),
        "input-mouse" | "input-keyboard" | "input-gaming" | "input-tablet" => d.icon.as_str(),
        "phone" | "computer" | "camera-video" | "printer" => d.icon.as_str(),
        _ => "bluetooth",
    };
    format!("{base}-symbolic")
}

struct Ui {
    page: gtk4::Box,
    paired: adw::PreferencesGroup,
    found: adw::PreferencesGroup,
    rows: RefCell<Vec<gtk4::Widget>>,
}

fn clear(ui: &Ui) {
    for w in ui.rows.borrow_mut().drain(..) {
        if let Some(parent) = w.parent() {
            if let Some(g) = parent.ancestor(adw::PreferencesGroup::static_type()).and_downcast::<adw::PreferencesGroup>() {
                g.remove(&w);
            }
        }
    }
}

fn show_error(ui: &Ui, title: &str, body: &str) {
    let d = adw::AlertDialog::new(Some(title), Some(body));
    d.add_response("ok", "OK");
    d.present(Some(&ui.page));
}

/// Run a bluetoothctl action for `mac`, then refresh the lists.
fn act(ui: &Rc<Ui>, button: &gtk4::Button, work: impl FnOnce() -> bool + Send + 'static, fail_title: &'static str) {
    button.set_sensitive(false);
    let ui = ui.clone();
    in_background(work, move |ok| {
        if !ok {
            show_error(&ui, fail_title, "Make sure the device is turned on, nearby, and in pairing mode, then try again.");
        }
        refresh(&ui);
    });
}

fn device_row(ui: &Rc<Ui>, d: &Device) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(&glib::markup_escape_text(&d.name));
    row.add_prefix(&gtk4::Image::from_icon_name(&device_icon(d)));
    let mut status = if d.connected { "Connected".to_string() } else if d.paired { "Paired".to_string() } else { "Available".to_string() };
    if let Some(b) = d.battery {
        status.push_str(&format!(" · Battery {b}%"));
    }
    row.set_subtitle(&status);
    row.set_activatable(false);

    if d.paired {
        let mac = d.mac.clone();
        let (label, verb) = if d.connected { ("Disconnect", "disconnect") } else { ("Connect", "connect") };
        let btn = gtk4::Button::with_label(label);
        btn.set_valign(gtk4::Align::Center);
        let ui2 = ui.clone();
        btn.connect_clicked(move |b| {
            let mac = mac.clone();
            act(&ui2, b, move || btctl_ok(&[verb, &mac]), if verb == "connect" { "Couldn't connect" } else { "Couldn't disconnect" });
        });
        row.add_suffix(&btn);

        let forget = gtk4::Button::from_icon_name("user-trash-symbolic");
        forget.add_css_class("flat");
        forget.set_valign(gtk4::Align::Center);
        forget.set_tooltip_text(Some("Forget this device"));
        let (mac, ui2, name) = (d.mac.clone(), ui.clone(), d.name.clone());
        forget.connect_clicked(move |b| {
            let dlg = adw::AlertDialog::new(
                Some(&format!("Forget {name}?")),
                Some("You'll need to pair it again to use it with this computer."),
            );
            dlg.add_responses(&[("cancel", "Cancel"), ("forget", "Forget")]);
            dlg.set_response_appearance("forget", adw::ResponseAppearance::Destructive);
            let (mac, ui3, b2) = (mac.clone(), ui2.clone(), b.clone());
            dlg.connect_response(None, move |_, r| {
                if r == "forget" {
                    let mac = mac.clone();
                    act(&ui3, &b2, move || btctl_ok(&["remove", &mac]), "Couldn't forget the device");
                }
            });
            dlg.present(Some(b));
        });
        row.add_suffix(&forget);
    } else {
        let btn = gtk4::Button::with_label("Pair");
        btn.add_css_class("suggested-action");
        btn.set_valign(gtk4::Align::Center);
        let (mac, ui2) = (d.mac.clone(), ui.clone());
        btn.connect_clicked(move |b| {
            let mac = mac.clone();
            b.set_label("Pairing…");
            act(
                &ui2,
                b,
                move || pair(&mac) && btctl_ok(&["trust", &mac]) && { let _ = btctl_ok(&["connect", &mac]); true },
                "Couldn't pair",
            );
        });
        row.add_suffix(&btn);
    }
    row
}

fn refresh(ui: &Rc<Ui>) {
    let ui = ui.clone();
    in_background(load_devices, move |devices| {
        clear(&ui);
        let paired: Vec<&Device> = devices.iter().filter(|d| d.paired).collect();
        let found: Vec<&Device> = devices.iter().filter(|d| !d.paired && d.name != d.mac).collect();
        if paired.is_empty() {
            let r = adw::ActionRow::new();
            r.set_title("No paired devices");
            r.set_subtitle("Use Add a device to pair headphones, a mouse, a keyboard or a phone");
            ui.paired.add(&r);
            ui.rows.borrow_mut().push(r.upcast());
        }
        for d in paired {
            let r = device_row(&ui, d);
            ui.paired.add(&r);
            ui.rows.borrow_mut().push(r.upcast());
        }
        for d in found {
            let r = device_row(&ui, d);
            ui.found.add(&r);
            ui.rows.borrow_mut().push(r.upcast());
        }
    });
}

fn cameras_group() -> Option<adw::PreferencesGroup> {
    let out = Command::new("v4l2-ctl").arg("--list-devices").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    // Device names are unindented lines ending in ":"; skip non-camera platform codecs.
    let names: Vec<String> = text
        .lines()
        .filter(|l| !l.starts_with(char::is_whitespace) && l.trim_end().ends_with(':'))
        .map(|l| l.trim_end().trim_end_matches(':').split(" (").next().unwrap_or(l).trim().to_string())
        .filter(|n| !n.to_lowercase().contains("codec") && !n.to_lowercase().contains("isp"))
        .collect();
    let g = adw::PreferencesGroup::new();
    g.set_title("Cameras");
    if names.is_empty() {
        let r = adw::ActionRow::new();
        r.set_title("No cameras connected");
        g.add(&r);
    }
    for n in names {
        let r = adw::ActionRow::new();
        r.set_title(&glib::markup_escape_text(&n));
        r.add_prefix(&gtk4::Image::from_icon_name("camera-web-symbolic"));
        g.add(&r);
    }
    Some(g)
}

fn other_devices_group(page: &gtk4::Box) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("Other devices");
    for (title, sub, icon, target) in [
        ("Mouse", "Pointer speed, buttons, scrolling", "input-mouse-symbolic", "Mouse"),
        ("Touchpad", "Taps, scrolling, speed", "input-touchpad-symbolic", "Touchpad"),
        ("Keyboard", "Typing, layouts, shortcuts", "input-keyboard-symbolic", "Keyboard"),
        ("Printers", "Add printers, print queue, paper size", "printer-symbolic", "Printers"),
        ("Phone", "Link your Android phone with Zohara Link", "phone-symbolic", "Zohara Link"),
    ] {
        let r = adw::ActionRow::new();
        r.set_title(title);
        r.set_subtitle(sub);
        r.add_prefix(&gtk4::Image::from_icon_name(icon));
        r.add_suffix(&gtk4::Image::from_icon_name("go-next-symbolic"));
        r.set_activatable(true);
        let page = page.clone();
        r.connect_activated(move |_| super::goto(&page, target));
        g.add(&r);
    }
    g
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
            .label("Bluetooth & devices")
            .halign(gtk4::Align::Start)
            .css_classes(vec!["win11-page-title".to_string()])
            .build(),
    );

    let bt = adw::PreferencesGroup::new();
    let power = adw::SwitchRow::new();
    power.set_title("Bluetooth");
    power.add_prefix(&gtk4::Image::from_icon_name("bluetooth-symbolic"));
    bt.add(&power);
    root.append(&bt);

    let paired = adw::PreferencesGroup::new();
    paired.set_title("Your devices");
    root.append(&paired);

    let found = adw::PreferencesGroup::new();
    found.set_title("Add a device");
    found.set_description(Some("Put the device in pairing mode, then search"));
    let scan = gtk4::Button::with_label("Search");
    scan.set_valign(gtk4::Align::Center);
    found.set_header_suffix(Some(&scan));
    root.append(&found);

    let ui = Rc::new(Ui { page: root.clone(), paired: paired.clone(), found: found.clone(), rows: RefCell::new(Vec::new()) });

    let adapter = has_adapter();
    let on = adapter && powered();
    power.set_active(on);
    power.set_subtitle(if adapter { "Connect wireless headphones, mice, keyboards and phones" } else { "No Bluetooth adapter found" });
    power.set_sensitive(adapter);
    paired.set_visible(on);
    found.set_visible(on);
    {
        let (ui, paired, found) = (ui.clone(), paired.clone(), found.clone());
        power.connect_active_notify(move |r| {
            let want = r.is_active();
            r.set_sensitive(false);
            let (r, ui, paired, found) = (r.clone(), ui.clone(), paired.clone(), found.clone());
            in_background(
                move || {
                    // Soft-blocked radios (airplane mode) must be unblocked before BlueZ can power on.
                    if want {
                        let _ = Command::new("rfkill").args(["unblock", "bluetooth"]).status();
                    }
                    btctl_ok(&["power", if want { "on" } else { "off" }]);
                    powered()
                },
                move |now| {
                    if now != r.is_active() {
                        r.set_active(now);
                    }
                    r.set_sensitive(true);
                    paired.set_visible(now);
                    found.set_visible(now);
                    if now {
                        refresh(&ui);
                    }
                },
            );
        });
    }

    {
        let ui = ui.clone();
        scan.connect_clicked(move |b| {
            b.set_sensitive(false);
            b.set_label("Searching…");
            let (b, ui) = (b.clone(), ui.clone());
            in_background(
                || {
                    let _ = Command::new("bluetoothctl").args(["--timeout", "10", "scan", "on"]).output();
                },
                move |_| {
                    b.set_label("Search again");
                    b.set_sensitive(true);
                    refresh(&ui);
                },
            );
        });
    }

    if on {
        refresh(&ui);
    }
    if let Some(cams) = cameras_group() {
        root.append(&cams);
    }
    root.append(&other_devices_group(&root));

    scroll.set_child(Some(&root));
    scroll.upcast()
}
