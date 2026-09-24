//! Network page sections backed by NetworkManager (nmcli), rfkill and KDE's
//! proxy config: connection status and metered setting, Ethernet with IP
//! settings, VPN import/connect, mobile hotspot, airplane mode, proxy, and
//! the list of network adapters.

use crate::backend::kconfig;
use crate::backend::worker::in_background;
use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use std::process::Command;

fn nmcli(args: &[&str]) -> String {
    Command::new("nmcli")
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

fn nmcli_result(args: &[&str]) -> Result<(), String> {
    let o = Command::new("nmcli").args(args).output().map_err(|e| e.to_string())?;
    if o.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&o.stderr).trim().trim_start_matches("Error: ").to_string())
    }
}

/// Splits a `nmcli -t` line on unescaped colons.
fn fields(line: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(n) = chars.next() {
                    out.last_mut().unwrap().push(n);
                }
            }
            ':' => out.push(String::new()),
            _ => out.last_mut().unwrap().push(c),
        }
    }
    out
}

fn message(w: &impl IsA<gtk4::Widget>, title: &str, body: &str) {
    let d = adw::AlertDialog::new(Some(title), Some(body));
    d.add_response("ok", "OK");
    d.present(Some(w));
}

fn value_label(text: &str) -> gtk4::Label {
    let l = gtk4::Label::new(Some(text));
    l.add_css_class("dim-label");
    l.set_selectable(true);
    l.set_wrap(true);
    l.set_xalign(1.0);
    l
}

fn info_row(title: &str, value: &str) -> adw::ActionRow {
    let r = adw::ActionRow::new();
    r.set_title(title);
    r.add_suffix(&value_label(value));
    r
}

// ── Status ─────────────────────────────────────────────────────────────────

#[derive(Default)]
struct Active {
    name: String,
    kind: String,
    device: String,
    ip: String,
    gateway: String,
    dns: String,
    metered: String,
    wifi: Option<(u32, u32, String)>, // signal %, frequency MHz, security
}

fn active_connection() -> Option<Active> {
    let line = nmcli(&["-t", "-f", "NAME,TYPE,DEVICE", "connection", "show", "--active"])
        .lines()
        .map(fields)
        .find(|f| f.len() >= 3 && !f[2].is_empty() && f[1] != "loopback" && f[1] != "bridge" && !f[2].starts_with("docker"))?;
    let (name, kind, device) = (line[0].clone(), line[1].clone(), line[2].clone());
    let dev = nmcli(&["-t", "-f", "IP4.ADDRESS,IP4.GATEWAY,IP4.DNS", "device", "show", &device]);
    let get = |k: &str| {
        dev.lines()
            .filter(|l| l.starts_with(k))
            .filter_map(|l| l.split_once(':').map(|(_, v)| v.to_string()))
            .filter(|v| !v.is_empty())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let metered = nmcli(&["-g", "connection.metered", "connection", "show", &name]).trim().to_string();
    let wifi = (kind == "802-11-wireless").then(|| {
        nmcli(&["-t", "-f", "ACTIVE,SIGNAL,FREQ,SECURITY", "device", "wifi", "list", "ifname", &device])
            .lines()
            .map(fields)
            .find(|f| f.first().map(String::as_str) == Some("yes"))
            .map(|f| {
                let freq = f.get(2).and_then(|x| x.split_whitespace().next()?.parse().ok()).unwrap_or(0);
                (f.get(1).and_then(|s| s.parse().ok()).unwrap_or(0), freq, f.get(3).cloned().unwrap_or_default())
            })
    })
    .flatten();
    Some(Active {
        ip: get("IP4.ADDRESS"),
        gateway: get("IP4.GATEWAY"),
        dns: get("IP4.DNS"),
        name,
        kind,
        device,
        metered,
        wifi,
    })
}

fn kind_label(kind: &str) -> &'static str {
    match kind {
        "802-11-wireless" => "Wi-Fi",
        "802-3-ethernet" => "Ethernet",
        "vpn" | "wireguard" => "VPN",
        "gsm" | "cdma" => "Mobile broadband",
        _ => "Network",
    }
}

pub fn status_group() -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("Status");
    let loading = adw::ActionRow::new();
    loading.set_title("Checking your connection…");
    g.add(&loading);

    let g2 = g.clone();
    in_background(active_connection, move |active| {
        g2.remove(&loading);
        let Some(a) = active else {
            let r = adw::ActionRow::new();
            r.set_title("Not connected");
            r.set_subtitle("Connect to Wi-Fi or plug in an Ethernet cable");
            r.add_prefix(&gtk4::Image::from_icon_name("network-offline-symbolic"));
            g2.add(&r);
            return;
        };
        let head = adw::ActionRow::new();
        head.set_title(&glib::markup_escape_text(&a.name));
        let mut sub = format!("Connected · {}", kind_label(&a.kind));
        if let Some((signal, freq, sec)) = &a.wifi {
            let band = if *freq >= 5925 { "6 GHz" } else if *freq >= 4900 { "5 GHz" } else { "2.4 GHz" };
            sub.push_str(&format!(" · {band} · Signal {signal}%"));
            if sec.is_empty() || sec == "--" {
                sub.push_str(" · Not secured");
            }
        }
        head.set_subtitle(&sub);
        head.add_prefix(&gtk4::Image::from_icon_name(if a.kind == "802-3-ethernet" {
            "network-wired-symbolic"
        } else {
            "network-wireless-signal-excellent-symbolic"
        }));
        g2.add(&head);

        let details = adw::ExpanderRow::new();
        details.set_title("Connection details");
        details.add_row(&info_row("IP address", if a.ip.is_empty() { "—" } else { &a.ip }));
        details.add_row(&info_row("Gateway", if a.gateway.is_empty() { "—" } else { &a.gateway }));
        details.add_row(&info_row("DNS servers", if a.dns.is_empty() { "—" } else { &a.dns }));
        details.add_row(&info_row("Adapter", &a.device));
        g2.add(&details);

        let metered = adw::ComboRow::new();
        metered.set_title("Metered connection");
        metered.set_subtitle("Apps and updates use less data on metered connections");
        let options = [("unknown", "Automatic"), ("yes", "Yes"), ("no", "No")];
        metered.set_model(Some(&gtk4::StringList::new(&options.map(|o| o.1))));
        let cur = options.iter().position(|o| a.metered.starts_with(o.0) || (o.0 == "unknown" && a.metered.contains("guess"))).unwrap_or(0);
        metered.set_selected(cur as u32);
        let name = a.name.clone();
        metered.connect_selected_notify(move |r| {
            let v = options[r.selected() as usize].0;
            let (name, r2) = (name.clone(), r.clone());
            in_background(
                move || nmcli_result(&["connection", "modify", &name, "connection.metered", v]),
                move |res| {
                    if let Err(e) = res {
                        message(&r2, "Setting not changed", &e);
                    }
                },
            );
        });
        g2.add(&metered);

        let ip = adw::ActionRow::new();
        ip.set_title("IP and DNS settings");
        ip.set_subtitle("Automatic (DHCP) or a manual address");
        ip.add_suffix(&gtk4::Image::from_icon_name("go-next-symbolic"));
        ip.set_activatable(true);
        let name = a.name.clone();
        ip.connect_activated(move |r| ip_dialog(r.upcast_ref(), &name));
        g2.add(&ip);
    });
    g
}

// ── IP settings dialog (shared by Wi-Fi and Ethernet) ──────────────────────

fn ip_dialog(parent: &gtk4::Widget, conn: &str) {
    let get = |k: &str| nmcli(&["-g", k, "connection", "show", conn]).trim().to_string();
    let method = get("ipv4.method");
    let d = adw::AlertDialog::new(Some("IP and DNS settings"), Some(&format!("For “{conn}”")));
    let form = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    let mode = gtk4::DropDown::from_strings(&["Automatic (DHCP)", "Manual"]);
    mode.set_selected((method == "manual") as u32);
    let addr = gtk4::Entry::builder().placeholder_text("Address, e.g. 192.168.1.50/24").text(get("ipv4.addresses")).build();
    let gw = gtk4::Entry::builder().placeholder_text("Gateway, e.g. 192.168.1.1").text(get("ipv4.gateway")).build();
    let dns = gtk4::Entry::builder()
        .placeholder_text("DNS servers, e.g. 1.1.1.1, 8.8.8.8 (optional)")
        .text(get("ipv4.dns").replace(',', ", "))
        .build();
    for w in [&addr, &gw] {
        w.set_sensitive(method == "manual");
    }
    {
        let (addr, gw) = (addr.clone(), gw.clone());
        mode.connect_selected_notify(move |m| {
            let manual = m.selected() == 1;
            addr.set_sensitive(manual);
            gw.set_sensitive(manual);
        });
    }
    form.append(&mode);
    form.append(&addr);
    form.append(&gw);
    form.append(&dns);
    d.set_extra_child(Some(&form));
    d.add_responses(&[("cancel", "Cancel"), ("save", "Save and reconnect")]);
    d.set_response_appearance("save", adw::ResponseAppearance::Suggested);
    d.set_close_response("cancel");
    let (conn, parent2) = (conn.to_string(), parent.clone());
    d.connect_response(None, move |_, r| {
        if r != "save" {
            return;
        }
        let manual = mode.selected() == 1;
        let (a, g, n) = (addr.text().trim().to_string(), gw.text().trim().to_string(), dns.text().to_string());
        let dns_list: Vec<String> = n.split([',', ' ']).filter(|s| !s.is_empty()).map(str::to_string).collect();
        if manual && !a.contains('/') {
            message(&parent2, "Settings not saved", "Enter the address with its prefix length, for example 192.168.1.50/24.");
            return;
        }
        let (conn, parent3) = (conn.clone(), parent2.clone());
        in_background(
            move || {
                let dns = dns_list.join(",");
                let mut args: Vec<String> = vec!["connection".into(), "modify".into(), conn.clone()];
                if manual {
                    args.extend(["ipv4.method", "manual", "ipv4.addresses", &a, "ipv4.gateway", &g].map(String::from));
                } else {
                    args.extend(["ipv4.method", "auto", "ipv4.addresses", "", "ipv4.gateway", ""].map(String::from));
                }
                args.extend(["ipv4.dns".to_string(), dns.clone()]);
                args.extend(["ipv4.ignore-auto-dns".to_string(), if dns.is_empty() { "no" } else { "yes" }.to_string()]);
                let refs: Vec<&str> = args.iter().map(String::as_str).collect();
                nmcli_result(&refs).and_then(|_| nmcli_result(&["connection", "up", &conn]))
            },
            move |res| match res {
                Ok(()) => message(&parent3, "Settings saved", "The connection has been restarted with the new settings."),
                Err(e) => message(&parent3, "Settings not saved", &e),
            },
        );
    });
    d.present(Some(parent));
}

// ── Ethernet ───────────────────────────────────────────────────────────────

pub fn ethernet_group() -> Option<adw::PreferencesGroup> {
    let devices: Vec<Vec<String>> = nmcli(&["-t", "-f", "DEVICE,TYPE,STATE,CONNECTION", "device"])
        .lines()
        .map(fields)
        .filter(|f| f.get(1).map(String::as_str) == Some("ethernet"))
        .collect();
    if devices.is_empty() {
        return None;
    }
    let g = adw::PreferencesGroup::new();
    g.set_title("Ethernet");
    for f in devices {
        let (dev, state, conn) = (f[0].clone(), f[2].clone(), f.get(3).cloned().unwrap_or_default());
        let row = adw::ActionRow::new();
        row.set_title(&dev);
        row.add_prefix(&gtk4::Image::from_icon_name("network-wired-symbolic"));
        let speed = std::fs::read_to_string(format!("/sys/class/net/{dev}/speed"))
            .ok()
            .and_then(|s| s.trim().parse::<i64>().ok())
            .filter(|s| *s > 0);
        let sub = match state.split_whitespace().next().unwrap_or("") {
            "connected" => match speed {
                Some(s) if s >= 1000 => format!("Connected · {} Gbps", s / 1000),
                Some(s) => format!("Connected · {s} Mbps"),
                None => "Connected".into(),
            },
            "unavailable" => "Cable unplugged".into(),
            "disconnected" => "Disconnected".into(),
            other => other.to_string(),
        };
        row.set_subtitle(&sub);
        if !conn.is_empty() && conn != "--" {
            let btn = gtk4::Button::with_label("IP settings…");
            btn.set_valign(gtk4::Align::Center);
            btn.connect_clicked(move |b| ip_dialog(b.upcast_ref(), &conn));
            row.add_suffix(&btn);
        }
        g.add(&row);
    }
    Some(g)
}

// ── VPN ────────────────────────────────────────────────────────────────────

pub fn vpn_group(page: &gtk4::Box) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("VPN");
    let add = gtk4::Button::with_label("Import…");
    add.set_valign(gtk4::Align::Center);
    add.set_tooltip_text(Some("Import an OpenVPN (.ovpn) or WireGuard (.conf) file from your VPN provider"));
    g.set_header_suffix(Some(&add));

    let rows: std::rc::Rc<std::cell::RefCell<Vec<gtk4::Widget>>> = Default::default();
    let fill: std::rc::Rc<dyn Fn()> = {
        let (g, rows, page) = (g.clone(), rows.clone(), page.clone());
        std::rc::Rc::new(move || {
            for r in rows.borrow_mut().drain(..) {
                g.remove(&r);
            }
            let vpns: Vec<Vec<String>> = nmcli(&["-t", "-f", "NAME,TYPE,ACTIVE", "connection", "show"])
                .lines()
                .map(fields)
                .filter(|f| matches!(f.get(1).map(String::as_str), Some("vpn") | Some("wireguard")))
                .collect();
            if vpns.is_empty() {
                let r = adw::ActionRow::new();
                r.set_title("No VPN connections");
                r.set_subtitle("Import the configuration file your VPN provider gives you");
                g.add(&r);
                rows.borrow_mut().push(r.upcast());
            }
            for f in vpns {
                let (name, active) = (f[0].clone(), f.get(2).map(String::as_str) == Some("yes"));
                let row = adw::SwitchRow::new();
                row.set_title(&glib::markup_escape_text(&name));
                row.set_subtitle(if f[1] == "wireguard" { "WireGuard" } else { "OpenVPN" });
                row.add_prefix(&gtk4::Image::from_icon_name("network-vpn-symbolic"));
                row.set_active(active);
                let (n2, page2) = (name.clone(), page.clone());
                row.connect_active_notify(move |r| {
                    let (n3, page3, r2, on) = (n2.clone(), page2.clone(), r.clone(), r.is_active());
                    r.set_sensitive(false);
                    in_background(
                        move || nmcli_result(&["connection", if on { "up" } else { "down" }, &n3]),
                        move |res| {
                            r2.set_sensitive(true);
                            if let Err(e) = res {
                                message(&page3, if on { "Couldn't connect" } else { "Couldn't disconnect" }, &e);
                                page3.activate_action("network.vpn-refresh", None).ok();
                            }
                        },
                    );
                });
                let del = gtk4::Button::from_icon_name("user-trash-symbolic");
                del.add_css_class("flat");
                del.set_valign(gtk4::Align::Center);
                del.set_tooltip_text(Some("Remove"));
                let (n2, page2) = (name.clone(), page.clone());
                del.connect_clicked(move |_| {
                    let d = adw::AlertDialog::new(Some(&format!("Remove {n2}?")), Some("You'll need the configuration file to add it again."));
                    d.add_responses(&[("cancel", "Cancel"), ("remove", "Remove")]);
                    d.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
                    let (n3, page3) = (n2.clone(), page2.clone());
                    d.connect_response(None, move |_, r| {
                        if r == "remove" {
                            let (n4, page4) = (n3.clone(), page3.clone());
                            in_background(
                                move || nmcli_result(&["connection", "delete", &n4]),
                                move |_| {
                                    page4.activate_action("network.vpn-refresh", None).ok();
                                },
                            );
                        }
                    });
                    d.present(Some(&page2));
                });
                row.add_suffix(&del);
                g.add(&row);
                rows.borrow_mut().push(row.upcast());
            }
        })
    };
    fill();

    let actions = gtk4::gio::SimpleActionGroup::new();
    let refresh = gtk4::gio::SimpleAction::new("vpn-refresh", None);
    {
        let fill = fill.clone();
        refresh.connect_activate(move |_, _| fill());
    }
    actions.add_action(&refresh);
    page.insert_action_group("network", Some(&actions));

    let page2 = page.clone();
    add.connect_clicked(move |b| {
        let dialog = gtk4::FileDialog::new();
        dialog.set_title("Import VPN configuration");
        let filter = gtk4::FileFilter::new();
        filter.set_name(Some("VPN configuration (.ovpn, .conf)"));
        filter.add_suffix("ovpn");
        filter.add_suffix("conf");
        let filters = gtk4::gio::ListStore::new::<gtk4::FileFilter>();
        filters.append(&filter);
        dialog.set_filters(Some(&filters));
        let page3 = page2.clone();
        dialog.open(b.root().and_downcast_ref::<gtk4::Window>(), None::<&gtk4::gio::Cancellable>, move |res| {
            let Some(path) = res.ok().and_then(|f| f.path()) else { return };
            let kind = if path.extension().and_then(|e| e.to_str()) == Some("conf") { "wireguard" } else { "openvpn" };
            let page4 = page3.clone();
            let p = path.to_string_lossy().to_string();
            in_background(
                move || nmcli_result(&["connection", "import", "type", kind, "file", &p]),
                move |res| {
                    match res {
                        Ok(()) => message(&page4, "VPN imported", "Turn it on in the list to connect."),
                        Err(e) => message(&page4, "Couldn't import the VPN", &e),
                    }
                    page4.activate_action("network.vpn-refresh", None).ok();
                },
            );
        });
    });
    g
}

// ── Hotspot and airplane mode ──────────────────────────────────────────────

fn hotspot_active() -> Option<String> {
    nmcli(&["-t", "-f", "NAME,TYPE", "connection", "show", "--active"])
        .lines()
        .map(fields)
        .find(|f| f[0] == "Hotspot")
        .map(|f| f[0].clone())
}

fn random_password() -> String {
    let mut seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1)
        ^ std::process::id() as u64;
    const CHARS: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    (0..12)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            CHARS[(seed % CHARS.len() as u64) as usize] as char
        })
        .collect()
}

fn airplane_on() -> bool {
    let out = Command::new("rfkill").args(["-J", "-o", "TYPE,SOFT"]).output().map(|o| o.stdout).unwrap_or_default();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap_or_default();
    let devs: Vec<&serde_json::Value> = v["rfkilldevices"].as_array().map(|a| a.iter().collect()).unwrap_or_default();
    !devs.is_empty() && devs.iter().all(|d| d["soft"].as_str() == Some("blocked"))
}

pub fn radios_group(page: &gtk4::Box) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();

    let hotspot = adw::SwitchRow::new();
    hotspot.set_title("Mobile hotspot");
    hotspot.add_prefix(&gtk4::Image::from_icon_name("network-wireless-hotspot-symbolic"));
    let active = hotspot_active().is_some();
    hotspot.set_active(active);
    hotspot.set_subtitle(if active { "On · other devices can connect" } else { "Share this computer's internet connection over Wi-Fi" });
    let reverting = std::rc::Rc::new(std::cell::Cell::new(false));
    let page2 = page.clone();
    hotspot.connect_active_notify(move |r| {
        if reverting.get() {
            return;
        }
        let revert = {
            let (r, reverting) = (r.clone(), reverting.clone());
            move |to: bool| {
                reverting.set(true);
                r.set_active(to);
                reverting.set(false);
            }
        };
        if !r.is_active() {
            let r2 = r.clone();
            in_background(
                || nmcli_result(&["connection", "down", "Hotspot"]),
                move |_| r2.set_subtitle("Share this computer's internet connection over Wi-Fi"),
            );
            return;
        }
        let host = std::fs::read_to_string("/etc/hostname").map(|s| s.trim().to_string()).unwrap_or_else(|_| "Zohara".into());
        let d = adw::AlertDialog::new(Some("Turn on mobile hotspot"), Some("Wi-Fi will be used for the hotspot, so this computer disconnects from its current Wi-Fi network."));
        let form = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        let ssid = gtk4::Entry::builder().text(format!("{host} hotspot")).placeholder_text("Network name").build();
        let pass = gtk4::Entry::builder().text(random_password()).placeholder_text("Password (at least 8 characters)").build();
        form.append(&ssid);
        form.append(&pass);
        d.set_extra_child(Some(&form));
        d.add_responses(&[("cancel", "Cancel"), ("on", "Turn on")]);
        d.set_response_appearance("on", adw::ResponseAppearance::Suggested);
        d.set_close_response("cancel");
        let (r2, page3) = (r.clone(), page2.clone());
        d.connect_response(None, move |_, resp| {
            if resp != "on" {
                revert(false);
                return;
            }
            let (s, p) = (ssid.text().to_string(), pass.text().to_string());
            if p.chars().count() < 8 || s.trim().is_empty() {
                revert(false);
                message(&page3, "Hotspot not started", "Enter a network name and a password of at least 8 characters.");
                return;
            }
            let (r3, page4, revert) = (r2.clone(), page3.clone(), revert.clone());
            in_background(
                move || {
                    let res = nmcli_result(&["device", "wifi", "hotspot", "con-name", "Hotspot", "ssid", &s, "password", &p]);
                    res.map(|_| (s, p))
                },
                move |res| match res {
                    Ok((s, p)) => r3.set_subtitle(&format!("On · Network “{s}” · Password {p}")),
                    Err(e) => {
                        revert(false);
                        message(&page4, "Hotspot not started", &e);
                    }
                },
            );
        });
        d.present(Some(&page2));
    });
    g.add(&hotspot);

    let air = adw::SwitchRow::new();
    air.set_title("Airplane mode");
    air.set_subtitle("Turn off Wi-Fi, Bluetooth and mobile broadband");
    air.add_prefix(&gtk4::Image::from_icon_name("airplane-mode-symbolic"));
    air.set_active(airplane_on());
    air.connect_active_notify(|r| {
        let on = r.is_active();
        let r2 = r.clone();
        in_background(
            move || {
                let _ = Command::new("rfkill").args([if on { "block" } else { "unblock" }, "all"]).status();
                airplane_on()
            },
            move |now| {
                if now != on {
                    r2.set_active(now);
                }
            },
        );
    });
    g.add(&air);
    g
}

/// Whether Wi-Fi radio is on, for the Wi-Fi switch's initial state.
pub fn wifi_enabled() -> bool {
    nmcli(&["radio", "wifi"]).trim() == "enabled"
}

// ── Proxy ──────────────────────────────────────────────────────────────────

const KIO: &str = "kioslaverc";
const PROXY: &str = "Proxy Settings";

/// KDE stores proxies as "http://host port".
fn split_proxy(v: &str) -> (String, String) {
    let v = v.trim();
    match v.rsplit_once(' ') {
        Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) => (h.to_string(), p.to_string()),
        _ => (v.to_string(), String::new()),
    }
}

fn notify_kio() {
    let _ = Command::new("dbus-send")
        .args(["--session", "--type=signal", "/KIO/Scheduler", "org.kde.KIO.Scheduler.reparseSlaveConfiguration", "string:"])
        .status();
}

pub fn proxy_group() -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("Proxy");
    g.set_description(Some("Used by the file manager, browsers and other apps that follow the desktop's proxy setting"));

    let mode = adw::ComboRow::new();
    mode.set_title("Proxy");
    mode.set_model(Some(&gtk4::StringList::new(&["Off", "Automatic (configuration URL)", "Manual"])));
    let ty = kconfig::read(KIO, &[PROXY], "ProxyType").unwrap_or_else(|| "0".into());
    mode.set_selected(match ty.as_str() {
        "2" => 1,
        "1" => 2,
        _ => 0,
    });
    g.add(&mode);

    let pac = adw::EntryRow::new();
    pac.set_title("Configuration URL");
    pac.set_text(&kconfig::read(KIO, &[PROXY], "Proxy Config Script").unwrap_or_default());
    pac.set_show_apply_button(true);
    g.add(&pac);

    let mk = |title: &str, key: &'static str| {
        let (host, port) = split_proxy(&kconfig::read(KIO, &[PROXY], key).unwrap_or_default());
        let row = adw::EntryRow::new();
        row.set_title(title);
        row.set_text(&if port.is_empty() { host } else { format!("{host}:{port}") });
        row.set_show_apply_button(true);
        row.connect_apply(move |e| {
            // Accept "host:port" and store it the way KDE expects.
            let text = e.text().trim().to_string();
            let value = match text.rsplit_once(':') {
                Some((h, p)) if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) => format!("{h} {p}"),
                _ => text,
            };
            kconfig::spawn(move || {
                kconfig::write(KIO, &[PROXY], key, &value);
                notify_kio();
            });
        });
        g.add(&row);
        row
    };
    let http = mk("HTTP proxy (host:port)", "httpProxy");
    let https = mk("HTTPS proxy (host:port)", "httpsProxy");
    let socks = mk("SOCKS proxy (host:port)", "socksProxy");
    let no = adw::EntryRow::new();
    no.set_title("Don't use the proxy for");
    no.set_text(&kconfig::read(KIO, &[PROXY], "NoProxyFor").unwrap_or_else(|| "localhost,127.0.0.1".into()));
    no.set_show_apply_button(true);
    no.connect_apply(|e| {
        let v = e.text().to_string();
        kconfig::spawn(move || {
            kconfig::write(KIO, &[PROXY], "NoProxyFor", &v);
            notify_kio();
        });
    });
    g.add(&no);

    let manual_rows = [http.clone(), https.clone(), socks.clone(), no.clone()];
    let show = {
        let (pac, manual_rows) = (pac.clone(), manual_rows.clone());
        move |sel: u32| {
            pac.set_visible(sel == 1);
            for r in &manual_rows {
                r.set_visible(sel == 2);
            }
        }
    };
    show(mode.selected());
    pac.connect_apply(|e| {
        let v = e.text().to_string();
        kconfig::spawn(move || {
            kconfig::write(KIO, &[PROXY], "Proxy Config Script", &v);
            notify_kio();
        });
    });
    mode.connect_selected_notify(move |m| {
        show(m.selected());
        let ty = match m.selected() {
            1 => "2",
            2 => "1",
            _ => "0",
        };
        kconfig::spawn(move || {
            kconfig::write(KIO, &[PROXY], "ProxyType", ty);
            notify_kio();
        });
    });
    g
}

// ── Adapters ───────────────────────────────────────────────────────────────

pub fn adapters_group() -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("Network adapters");
    for f in nmcli(&["-t", "-f", "DEVICE,TYPE,STATE,CONNECTION", "device"]).lines().map(fields) {
        if f.len() < 3 || matches!(f[1].as_str(), "loopback" | "bridge" | "tun" | "wifi-p2p") {
            continue;
        }
        let dev = &f[0];
        let exp = adw::ExpanderRow::new();
        exp.set_title(dev);
        exp.set_subtitle(&format!("{} · {}", kind_label(match f[1].as_str() {
            "wifi" => "802-11-wireless",
            "ethernet" => "802-3-ethernet",
            "gsm" => "gsm",
            other => other,
        }), f[2]));
        let show = nmcli(&["-t", "-f", "GENERAL.HWADDR,GENERAL.DRIVER,GENERAL.VENDOR,GENERAL.PRODUCT,IP4.ADDRESS,IP6.ADDRESS", "device", "show", dev]);
        for line in show.lines() {
            let Some((k, v)) = line.split_once(':') else { continue };
            if v.is_empty() {
                continue;
            }
            let label = match k.split('[').next().unwrap_or(k) {
                "GENERAL.HWADDR" => "Hardware address",
                "GENERAL.DRIVER" => "Driver",
                "GENERAL.VENDOR" => "Manufacturer",
                "GENERAL.PRODUCT" => "Model",
                "IP4.ADDRESS" => "IPv4 address",
                "IP6.ADDRESS" => "IPv6 address",
                _ => continue,
            };
            exp.add_row(&info_row(label, &v.replace("\\:", ":")));
        }
        g.add(&exp);
    }
    g
}
