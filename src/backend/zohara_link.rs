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
    Some(Identity { name: text(v, "name"), code })
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
    let mut stream = UnixStream::connect(socket_path()).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(3))).ok()?;
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
            Some(Identity { name: "laptop".into(), code: "ab12 cd34 ef56 7890".into() })
        );
        assert!(parse_identity(&json!({"name": "laptop"})).is_none());
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
