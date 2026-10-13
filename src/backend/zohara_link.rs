//! Talking to `zohara-linkd`, the Zohara Link daemon (repo `zohara-link`), over its local Unix socket.
//!
//! The daemon does the real work (TLS to the phone, pairing, the PIN, file transfer); Settings only asks it things and tells it
//! what the person pressed. Everything here is blocking (a few lines over a local socket), so callers run it with
//! `worker::in_background`. The JSON parsing is separate from the socket code so it can be tested without a daemon.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

pub const SERVICE: &str = "zohara-linkd.service";

/// `ZOHARA_LINK_SOCKET` overrides the path (tests, a second instance), like in the daemon.
pub fn socket_path() -> PathBuf {
    if let Some(p) = std::env::var_os("ZOHARA_LINK_SOCKET") {
        return PathBuf::from(p);
    }
    extern "C" {
        fn geteuid() -> u32;
    }
    PathBuf::from(format!("/run/user/{}/zohara.sock", unsafe { geteuid() }))
}

/// Is the daemon's package installed on this computer?
pub fn installed() -> bool {
    std::path::Path::new("/usr/bin/zohara-linkd").exists()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub last_ip: String,
    pub connected: bool,
    pub allow_input: bool,
    pub allow_screen: bool,
}

/// A phone that asked to pair and is waiting for its PIN to be typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub name: String,
    pub pin: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    pub devices: Vec<Device>,
    pub pairing_open: bool,
    pub pending: Vec<Pending>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub name: String,
    /// The short code (`ab12 cd34 ef56 7890`) the phone shows too, so the person can see they match.
    pub code: String,
    /// The port the daemon listens on (the phone needs it together with the address when it cannot find the computer by itself).
    pub port: u16,
}

/// One address of this computer on a real network: `192.168.1.20` on `wlan0`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Address {
    pub iface: String,
    pub ip: String,
}

/// Network interfaces that only exist inside this computer (containers, virtual machines): their addresses are useless to a phone
/// and only confuse the person, so they are not listed.
fn is_virtual_iface(name: &str) -> bool {
    ["lo", "docker", "br-", "virbr", "veth", "vmnet", "vboxnet", "lxc", "lxd", "podman", "cni", "flannel"]
        .iter()
        .any(|p| name == *p || (p.len() > 2 && name.starts_with(p)))
}

/// "Wi-Fi", "Ethernet" or "Network", from the interface name Linux gave it.
pub fn iface_kind(name: &str) -> &'static str {
    if name.starts_with("wl") {
        "Wi-Fi"
    } else if name.starts_with("en") || name.starts_with("eth") {
        "Ethernet"
    } else {
        "Network"
    }
}

/// The output of `ip -4 -o addr show scope global`, one line per address:
/// `2: wlan0    inet 192.168.1.20/24 brd 192.168.1.255 scope global dynamic noprefixroute wlan0 ...`
/// Wi-Fi first, then Ethernet, then the rest; interfaces that only exist inside the computer are left out.
pub fn parse_addresses(out: &str) -> Vec<Address> {
    let mut found: Vec<Address> = out
        .lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            words.next()?; // "2:"
            let iface = words.next()?.trim_end_matches(':').split('@').next()?.to_string();
            let ip = words.skip_while(|w| *w != "inet").nth(1)?.split('/').next()?.to_string();
            if is_virtual_iface(&iface) || ip.is_empty() {
                return None;
            }
            Some(Address { iface, ip })
        })
        .collect();
    found.sort_by_key(|a| match iface_kind(&a.iface) {
        "Wi-Fi" => 0,
        "Ethernet" => 1,
        _ => 2,
    });
    found
}

/// This computer's addresses on real networks (asks `ip`, which every Arch system has).
pub fn local_addresses() -> Vec<Address> {
    Command::new("ip")
        .args(["-4", "-o", "addr", "show", "scope", "global"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|o| parse_addresses(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
}

fn text(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

/// The answer to `GET_STATUS`. `None` when the line is not a status answer at all.
pub fn parse_status(v: &Value) -> Option<Status> {
    let paired = v.get("paired_devices")?.as_array()?;
    let connected: Vec<&str> = v
        .get("connected_device_ids")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let mut devices: Vec<Device> = paired
        .iter()
        .map(|d| {
            let id = text(d, "device_id");
            Device {
                connected: connected.contains(&id.as_str()),
                name: if text(d, "device_name").is_empty() { "Unknown phone".into() } else { text(d, "device_name") },
                last_ip: text(d, "last_ip"),
                allow_input: d.get("allow_input").and_then(Value::as_bool).unwrap_or(false),
                allow_screen: d.get("allow_screen").and_then(Value::as_bool).unwrap_or(false),
                id,
            }
        })
        .collect();
    devices.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.id.cmp(&b.id)));
    let pending = v
        .get("pending_pairings")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(|p| Pending { name: text(p, "name"), pin: text(p, "pin") }).collect())
        .unwrap_or_default();
    Some(Status { devices, pairing_open: v.get("pairing_open").and_then(Value::as_bool).unwrap_or(false), pending })
}

/// The answer to `GET_IDENTITY`.
pub fn parse_identity(v: &Value) -> Option<Identity> {
    let code = text(v, "fingerprint_short");
    if code.is_empty() {
        return None;
    }
    let port = v.get("port").and_then(Value::as_u64).and_then(|p| u16::try_from(p).ok()).unwrap_or(42424);
    Some(Identity { name: text(v, "name"), code, port })
}

/// `123456` as `123 456`, easier to read off a screen.
pub fn pin_for_display(pin: &str) -> String {
    if pin.len() == 6 {
        format!("{} {}", &pin[..3], &pin[3..])
    } else {
        pin.to_string()
    }
}

/// Sends one command and returns its answer, skipping the live event lines the daemon also sends on this connection.
/// `None` when the daemon is not there or does not answer within a few seconds.
pub fn request(cmd: &Value) -> Option<Value> {
    request_within(cmd, 3)
}

fn request_within(cmd: &Value, secs: u64) -> Option<Value> {
    let mut stream = UnixStream::connect(socket_path()).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(secs))).ok()?;
    stream.set_write_timeout(Some(Duration::from_secs(3))).ok()?;
    let mut line = serde_json::to_string(cmd).ok()?;
    line.push('\n');
    stream.write_all(line.as_bytes()).ok()?;
    let mut reader = BufReader::new(stream);
    loop {
        let mut buf = String::new();
        if reader.read_line(&mut buf).ok()? == 0 {
            return None;
        }
        let v: Value = serde_json::from_str(buf.trim()).ok()?;
        if v.get("event").is_none() {
            return Some(v);
        }
    }
}

pub fn status() -> Option<Status> {
    parse_status(&request(&json!({"command": "GET_STATUS"}))?)
}

pub fn identity() -> Option<Identity> {
    parse_identity(&request(&json!({"command": "GET_IDENTITY"}))?)
}

/// True when the daemon answered `{"status":"OK"}`.
pub fn command_ok(cmd: Value) -> bool {
    request(&cmd).map(|v| v.get("status").and_then(Value::as_str) == Some("OK")).unwrap_or(false)
}

fn run(cmd: &mut Command) -> Result<(), String> {
    let out = cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).output().map_err(|e| e.to_string())?;
    match out.status.code() {
        Some(0) => Ok(()),
        Some(126) | Some(127) => Err("Administrator approval was not given.".into()),
        _ => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
    }
}

/// An address of this computer on an overlay network (Tailscale, ZeroTier, NetBird, WireGuard): reachable from outside the home Wi-Fi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overlay {
    pub provider: String,
    pub address: String,
    pub dns_name: String,
}

/// Where this computer can be reached from away from home.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Remote {
    pub overlays: Vec<Overlay>,
    pub tailscale_installed: bool,
    /// Tailscale's own word: `Running`, `NeedsLogin`, `Stopped`, `NoState`... empty when not installed.
    pub tailscale_state: String,
}

/// The answer to `GET_REMOTE`.
pub fn parse_remote(v: &Value) -> Option<Remote> {
    let overlays = v.get("overlays")?.as_array()?;
    let ts = v.get("tailscale");
    Some(Remote {
        overlays: overlays
            .iter()
            .map(|o| Overlay { provider: text(o, "provider"), address: text(o, "address"), dns_name: text(o, "dnsName") })
            .collect(),
        tailscale_installed: ts.and_then(|t| t.get("installed")).and_then(Value::as_bool).unwrap_or(false),
        tailscale_state: ts.map(|t| text(t, "state")).unwrap_or_default(),
    })
}

pub fn remote() -> Option<Remote> {
    parse_remote(&request_within(&json!({"command": "GET_REMOTE"}), 6)?)
}

/// Asks Tailscale for its sign-in link (it can take several seconds). `None` when it printed none.
pub fn remote_login() -> Option<String> {
    let v = request_within(&json!({"command": "REMOTE_LOGIN"}), 25)?;
    let url = text(&v, "loginUrl");
    if url.starts_with("https://") { Some(url) } else { None }
}

/// Installs Tailscale (administrator prompt), starts it, and lets this user control it so the sign-in button needs no password.
pub fn install_tailscale() -> Result<(), String> {
    run(Command::new("pkexec").args([
        "sh",
        "-c",
        "pacman -S --needed --noconfirm tailscale && systemctl enable --now tailscaled && tailscale set --operator=\"$(id -nu \"$PKEXEC_UID\")\"",
    ]))
    .map_err(|e| {
        if e.contains("target not found") {
            "Tailscale is not in your update channel. Update the system first (Settings > Zohara Update).".to_string()
        } else {
            e
        }
    })
}

/// Turns the daemon on for this user, and on at every sign-in.
pub fn start() -> Result<(), String> {
    run(Command::new("systemctl").args(["--user", "enable", "--now", SERVICE]))
}

/// Installs the daemon's package (administrator prompt) and starts it. Uses the package lists already on the computer:
/// a bare `pacman -Sy` is never safe on Arch, so if the package is missing the message says to update first.
pub fn install_and_start() -> Result<(), String> {
    if !installed() {
        run(Command::new("pkexec").args(["pacman", "-S", "--needed", "--noconfirm", "zohara-linkd"])).map_err(|e| {
            if e.contains("target not found") {
                "Zohara Link is not in your update channel yet. Update the system first (Settings > Zohara Update), or try the alpha channel.".to_string()
            } else {
                e
            }
        })?;
    }
    start()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    use std::sync::Mutex;

    /// The two socket tests change a process-wide environment variable, so they must not run at the same time.
    static ENV: Mutex<()> = Mutex::new(());

    #[test]
    fn status_lists_devices_sorted_with_connection_and_input() {
        let v = json!({
            "paired_devices": [
                {"device_id": "b", "device_name": "Zed phone", "last_ip": "10.0.0.5", "allow_input": true},
                {"device_id": "a", "device_name": "alpha phone", "last_ip": "10.0.0.4"},
                {"device_id": "c", "device_name": ""}
            ],
            "connected_device_ids": ["b"],
            "pairing_open": true,
            "pending_pairings": [{"deviceId": "x", "name": "New phone", "pin": "123456"}]
        });
        let s = parse_status(&v).unwrap();
        // Sorted by name, ignoring case: "alpha phone", "Unknown phone" (no name given), "Zed phone".
        assert_eq!(s.devices.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), ["a", "c", "b"]);
        assert!(!s.devices[0].connected && s.devices[2].connected);
        assert!(s.devices[2].allow_input && !s.devices[0].allow_input);
        assert_eq!(s.devices[1].name, "Unknown phone");
        assert!(s.pairing_open);
        assert_eq!(s.pending, vec![Pending { name: "New phone".into(), pin: "123456".into() }]);
    }

    #[test]
    fn remote_answer_is_read() {
        let r = parse_remote(&json!({
            "overlays": [{"provider": "Tailscale", "address": "100.1.2.3", "dnsName": "laptop.tail1.ts.net"}],
            "tailscale": {"installed": true, "state": "Running"}
        }))
        .unwrap();
        assert_eq!(r.overlays, vec![Overlay { provider: "Tailscale".into(), address: "100.1.2.3".into(), dns_name: "laptop.tail1.ts.net".into() }]);
        assert!(r.tailscale_installed && r.tailscale_state == "Running");
        assert_eq!(parse_remote(&json!({"overlays": []})).unwrap(), Remote::default());
        assert!(parse_remote(&json!({"status": "OK"})).is_none());
    }

    #[test]
    fn screen_permission_defaults_to_off() {
        let s = parse_status(&json!({"paired_devices": [{"device_id": "a", "device_name": "p", "allow_screen": true}, {"device_id": "b", "device_name": "q"}]})).unwrap();
        assert!(s.devices[0].allow_screen && !s.devices[1].allow_screen);
    }

    #[test]
    fn status_from_an_older_daemon_still_parses() {
        let s = parse_status(&json!({"paired_devices": [], "telemetry": {}})).unwrap();
        assert_eq!(s, Status::default());
    }

    #[test]
    fn a_line_that_is_not_a_status_answer_is_rejected() {
        assert!(parse_status(&json!({"status": "OK"})).is_none());
        assert!(parse_status(&json!({})).is_none());
    }

    #[test]
    fn identity_needs_a_code() {
        assert_eq!(
            parse_identity(&json!({"name": "laptop", "fingerprint_short": "ab12 cd34 ef56 7890"})),
            Some(Identity { name: "laptop".into(), code: "ab12 cd34 ef56 7890".into(), port: 42424 })
        );
        assert!(parse_identity(&json!({"name": "laptop"})).is_none());
    }

    #[test]
    fn identity_port_comes_from_the_daemon() {
        let id = parse_identity(&json!({"name": "pc", "fingerprint_short": "aaaa bbbb cccc dddd", "port": 5000})).unwrap();
        assert_eq!(id.port, 5000);
    }

    const IP_OUT: &str = "\
1: lo    inet 127.0.0.1/8 scope host lo\\       valid_lft forever preferred_lft forever
2: enp3s0    inet 192.168.1.30/24 brd 192.168.1.255 scope global dynamic noprefixroute enp3s0\\       valid_lft 86000sec
3: wlan0    inet 192.168.1.20/24 brd 192.168.1.255 scope global dynamic noprefixroute wlan0\\       valid_lft 85000sec
4: docker0    inet 172.17.0.1/16 brd 172.17.255.255 scope global docker0\\       valid_lft forever
5: virbr0    inet 192.168.122.1/24 brd 192.168.122.255 scope global virbr0\\       valid_lft forever
6: br-1a2b3c    inet 172.18.0.1/16 scope global br-1a2b3c\\       valid_lft forever
7: tun0    inet 10.8.0.2/24 scope global tun0\\       valid_lft forever
";

    #[test]
    fn addresses_skip_virtual_networks_and_put_wifi_first() {
        let a = parse_addresses(IP_OUT);
        assert_eq!(
            a,
            vec![
                Address { iface: "wlan0".into(), ip: "192.168.1.20".into() },
                Address { iface: "enp3s0".into(), ip: "192.168.1.30".into() },
                Address { iface: "tun0".into(), ip: "10.8.0.2".into() },
            ]
        );
    }

    #[test]
    fn addresses_from_nothing_or_junk_are_empty() {
        assert!(parse_addresses("").is_empty());
        assert!(parse_addresses("garbage\nmore garbage").is_empty());
    }

    #[test]
    fn interface_kinds_are_named_for_people() {
        assert_eq!(iface_kind("wlan0"), "Wi-Fi");
        assert_eq!(iface_kind("wlp2s0"), "Wi-Fi");
        assert_eq!(iface_kind("enp3s0"), "Ethernet");
        assert_eq!(iface_kind("tun0"), "Network");
    }

    #[test]
    fn pin_is_grouped_for_reading() {
        assert_eq!(pin_for_display("123456"), "123 456");
        assert_eq!(pin_for_display("12"), "12");
    }

    #[test]
    fn request_skips_event_lines_and_returns_the_answer() {
        let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let path = std::env::temp_dir().join(format!("zl-settings-test-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let t = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut got = String::new();
            BufReader::new(s.try_clone().unwrap()).read_line(&mut got).unwrap();
            assert!(got.contains("GET_IDENTITY"));
            s.write_all(b"{\"event\":\"TELEMETRY_UPDATED\",\"data\":{}}\n{\"name\":\"pc\",\"fingerprint_short\":\"aaaa bbbb cccc dddd\"}\n").unwrap();
        });
        std::env::set_var("ZOHARA_LINK_SOCKET", &path);
        let id = identity().unwrap();
        assert_eq!(id.code, "aaaa bbbb cccc dddd");
        t.join().unwrap();
        std::env::remove_var("ZOHARA_LINK_SOCKET");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn no_daemon_means_none_not_a_crash() {
        let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let path = std::env::temp_dir().join(format!("zl-settings-missing-{}.sock", std::process::id()));
        std::env::set_var("ZOHARA_LINK_SOCKET", &path);
        assert!(status().is_none());
        assert!(!command_ok(json!({"command": "OPEN_PAIRING"})));
        std::env::remove_var("ZOHARA_LINK_SOCKET");
    }
}
