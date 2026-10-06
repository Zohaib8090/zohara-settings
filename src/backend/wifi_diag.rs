//! "Why is there no Wi-Fi?": looks at the hardware, the driver, the radio switches and NetworkManager, and says which
//! one is the problem in plain words. Reading is separate from judging: `gather` collects facts from the system,
//! `diagnose` is a pure function over them (and is unit-tested), so every verdict can be checked without a laptop.
//!
//! Found on a real laptop (2026-10-06): the Network page simply showed an empty list when the Wi-Fi adapter was not
//! working, with no hint whether the hardware, a driver, a switch or NetworkManager was at fault.

use std::path::Path;
use std::process::Command;

/// A network controller found on the PCI bus.
#[derive(Debug, Clone, PartialEq)]
pub struct NetHardware {
    /// Vendor and device id, like `10ec:c821`.
    pub id: String,
    /// Human name when `lspci` knows it, otherwise the id.
    pub name: String,
    /// Kernel driver bound to it, if any.
    pub driver: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Facts {
    /// Network interfaces that are wireless (`/sys/class/net/*/wireless`).
    pub wireless_ifaces: Vec<String>,
    /// Wi-Fi controllers on the PCI bus (class 0x0280).
    pub hardware: Vec<NetHardware>,
    /// `rfkill` for the "wlan" type: any soft-blocked / hard-blocked.
    pub soft_blocked: bool,
    pub hard_blocked: bool,
    /// Whether NetworkManager answers.
    pub nm_running: bool,
    /// Whether NetworkManager lists a Wi-Fi device.
    pub nm_has_wifi_device: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// All good: there is an interface and NetworkManager manages it.
    Ready,
    /// No Wi-Fi hardware at all (a desktop, or a USB adapter that is not plugged in).
    NoAdapter,
    /// Hardware is there but the kernel has no driver for it.
    NoDriver { hardware: String },
    /// A driver is loaded but no interface appeared (typically missing firmware, or the chip did not wake up).
    NoInterface { hardware: String, driver: String },
    /// The airplane-mode switch on the keyboard or in software.
    HardBlocked,
    SoftBlocked,
    /// The interface exists but NetworkManager does not list it.
    NotManaged,
    NetworkManagerDown,
}

impl Verdict {
    pub fn is_ready(&self) -> bool {
        *self == Verdict::Ready
    }

    pub fn title(&self) -> &'static str {
        match self {
            Verdict::Ready => "Wi-Fi adapter is working",
            Verdict::NoAdapter => "No Wi-Fi adapter found",
            Verdict::NoDriver { .. } => "Wi-Fi hardware found, but no driver for it",
            Verdict::NoInterface { .. } => "The Wi-Fi driver loaded, but no Wi-Fi appeared",
            Verdict::HardBlocked => "Wi-Fi is switched off by a hardware switch",
            Verdict::SoftBlocked => "Wi-Fi is turned off",
            Verdict::NotManaged => "Wi-Fi exists but the network service isn't using it",
            Verdict::NetworkManagerDown => "The network service isn't running",
        }
    }

    /// What it means and what to do, in plain words.
    pub fn explanation(&self) -> String {
        match self {
            Verdict::Ready => "Wi-Fi is detected and managed. If there are no networks, scan again or move closer to the router.".into(),
            Verdict::NoAdapter => "This computer doesn't show any Wi-Fi hardware. If you use a USB Wi-Fi adapter, plug it in; otherwise use an Ethernet cable.".into(),
            Verdict::NoDriver { hardware } => format!("{hardware} is installed, but Linux has no driver loaded for it. Use \"Save a problem report\" in Troubleshoot and send it to the Zohara team: it names the exact chip so the driver can be added."),
            Verdict::NoInterface { hardware, driver } => format!("{hardware} uses the {driver} driver, which loaded, but no Wi-Fi interface came up. That is usually missing firmware or a chip that didn't wake up after sleep. \"Reload the Wi-Fi driver\" often fixes the second case."),
            Verdict::HardBlocked => "Look for an airplane-mode key or a switch on the side of the laptop and turn Wi-Fi back on.".into(),
            Verdict::SoftBlocked => "Airplane mode or a software switch has turned Wi-Fi off. \"Turn Wi-Fi on\" switches it back.".into(),
            Verdict::NotManaged => "The adapter is there but NetworkManager isn't managing it. \"Restart the network service\" usually fixes this.".into(),
            Verdict::NetworkManagerDown => "NetworkManager, the service that handles connections, isn't running. \"Restart the network service\" starts it again.".into(),
        }
    }
}

/// Decides what is wrong. Order matters: a switch that is off explains everything else, so it comes first.
pub fn diagnose(f: &Facts) -> Verdict {
    if f.hard_blocked {
        return Verdict::HardBlocked;
    }
    if f.soft_blocked {
        return Verdict::SoftBlocked;
    }
    if !f.nm_running {
        return Verdict::NetworkManagerDown;
    }
    if !f.wireless_ifaces.is_empty() {
        return if f.nm_has_wifi_device { Verdict::Ready } else { Verdict::NotManaged };
    }
    // No wireless interface at all: is there hardware that should have made one?
    match f.hardware.first() {
        None => Verdict::NoAdapter,
        Some(h) => match &h.driver {
            None => Verdict::NoDriver { hardware: h.name.clone() },
            Some(d) => Verdict::NoInterface { hardware: h.name.clone(), driver: d.clone() },
        },
    }
}

// ── Reading the system ──────────────────────────────────────────────────────

fn read(p: impl AsRef<Path>) -> String {
    std::fs::read_to_string(p).map(|s| s.trim().to_string()).unwrap_or_default()
}

fn wireless_ifaces() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir("/sys/class/net")
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join("wireless").exists())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

/// `lspci -s SLOT -nn` gives "Network controller [0280]: Realtek ... [10ec:c821]"; keep the part after the colon.
fn pci_name(slot: &str, id: &str) -> String {
    let out = Command::new("lspci").args(["-s", slot, "-nn"]).output().ok().filter(|o| o.status.success());
    out.and_then(|o| {
        let t = String::from_utf8_lossy(&o.stdout).trim().to_string();
        t.splitn(2, ": ").nth(1).map(|s| s.to_string())
    })
    .unwrap_or_else(|| format!("Wi-Fi controller [{id}]"))
}

fn pci_wifi_hardware() -> Vec<NetHardware> {
    let mut out = Vec::new();
    for e in std::fs::read_dir("/sys/bus/pci/devices").into_iter().flatten().flatten() {
        let p = e.path();
        // Class 0x0280xx = "Network controller" (Wi-Fi); 0x0200xx is Ethernet.
        if !read(p.join("class")).starts_with("0x0280") {
            continue;
        }
        let id = format!("{}:{}", read(p.join("vendor")).trim_start_matches("0x"), read(p.join("device")).trim_start_matches("0x"));
        let driver = std::fs::read_link(p.join("driver")).ok().and_then(|d| d.file_name().map(|n| n.to_string_lossy().into_owned()));
        let slot = e.file_name().to_string_lossy().into_owned();
        out.push(NetHardware { name: pci_name(&slot, &id), id, driver });
    }
    out
}

/// Whether the "wlan" radios are blocked, from `/sys/class/rfkill` (no tools or rights needed).
fn rfkill_blocks() -> (bool, bool) {
    let (mut soft, mut hard) = (false, false);
    for e in std::fs::read_dir("/sys/class/rfkill").into_iter().flatten().flatten() {
        let p = e.path();
        if read(p.join("type")) != "wlan" {
            continue;
        }
        soft |= read(p.join("soft")) == "1";
        hard |= read(p.join("hard")) == "1";
    }
    (soft, hard)
}

fn nm_state() -> (bool, bool) {
    let o = Command::new("nmcli").args(["-t", "-f", "DEVICE,TYPE", "device"]).output();
    match o {
        Ok(o) if o.status.success() => {
            let t = String::from_utf8_lossy(&o.stdout);
            (true, t.lines().any(|l| l.rsplit(':').next() == Some("wifi")))
        }
        _ => (false, false),
    }
}

/// Reads everything from the system (fast: a few files and one `nmcli`). Call off the UI thread.
pub fn gather() -> Facts {
    let (soft_blocked, hard_blocked) = rfkill_blocks();
    let (nm_running, nm_has_wifi_device) = nm_state();
    Facts { wireless_ifaces: wireless_ifaces(), hardware: pci_wifi_hardware(), soft_blocked, hard_blocked, nm_running, nm_has_wifi_device }
}

/// Plain-text report for support (copied or saved).
pub fn report(f: &Facts, v: &Verdict) -> String {
    let mut s = format!("Wi-Fi check: {}\n{}\n\n", v.title(), v.explanation());
    s.push_str(&format!("wireless interfaces: {:?}\n", f.wireless_ifaces));
    for h in &f.hardware {
        s.push_str(&format!("hardware: {} [{}] driver: {}\n", h.name, h.id, h.driver.as_deref().unwrap_or("none")));
    }
    s.push_str(&format!("blocked: soft={} hard={}\nNetworkManager: running={} wifi device={}\n", f.soft_blocked, f.hard_blocked, f.nm_running, f.nm_has_wifi_device));
    s
}

/// The kernel module to reload for the Wi-Fi chip: the driver of the PCI controller, or of the wireless interface.
pub fn driver_to_reload(f: &Facts) -> Option<String> {
    f.hardware.iter().find_map(|h| h.driver.clone()).filter(|d| d.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok() -> Facts {
        Facts { wireless_ifaces: vec!["wlan0".into()], nm_running: true, nm_has_wifi_device: true, ..Default::default() }
    }
    fn chip(driver: Option<&str>) -> NetHardware {
        NetHardware { id: "10ec:c821".into(), name: "Realtek RTL8821CE".into(), driver: driver.map(String::from) }
    }

    #[test]
    fn working_wifi_is_ready() {
        assert_eq!(diagnose(&ok()), Verdict::Ready);
    }

    #[test]
    fn no_hardware_at_all() {
        let f = Facts { nm_running: true, ..Default::default() };
        assert_eq!(diagnose(&f), Verdict::NoAdapter);
    }

    #[test]
    fn hardware_without_a_driver() {
        let f = Facts { hardware: vec![chip(None)], nm_running: true, ..Default::default() };
        assert_eq!(diagnose(&f), Verdict::NoDriver { hardware: "Realtek RTL8821CE".into() });
    }

    #[test]
    fn driver_loaded_but_no_interface_is_what_happens_after_a_bad_resume() {
        let f = Facts { hardware: vec![chip(Some("rtw_8821ce"))], nm_running: true, ..Default::default() };
        assert_eq!(diagnose(&f), Verdict::NoInterface { hardware: "Realtek RTL8821CE".into(), driver: "rtw_8821ce".into() });
        assert_eq!(driver_to_reload(&f).as_deref(), Some("rtw_8821ce"));
    }

    #[test]
    fn a_switch_that_is_off_explains_everything_else() {
        let mut f = Facts { hardware: vec![chip(None)], ..Default::default() };
        f.soft_blocked = true;
        assert_eq!(diagnose(&f), Verdict::SoftBlocked);
        f.hard_blocked = true;
        assert_eq!(diagnose(&f), Verdict::HardBlocked);
    }

    #[test]
    fn interface_that_networkmanager_does_not_list() {
        let mut f = ok();
        f.nm_has_wifi_device = false;
        assert_eq!(diagnose(&f), Verdict::NotManaged);
    }

    #[test]
    fn networkmanager_down() {
        let mut f = ok();
        f.nm_running = false;
        assert_eq!(diagnose(&f), Verdict::NetworkManagerDown);
    }

    #[test]
    fn module_names_are_checked_before_being_passed_to_modprobe() {
        let f = Facts { hardware: vec![chip(Some("bad;name"))], ..Default::default() };
        assert_eq!(driver_to_reload(&f), None);
    }

    #[test]
    fn every_verdict_says_something() {
        for v in [Verdict::Ready, Verdict::NoAdapter, Verdict::NoDriver { hardware: "x".into() }, Verdict::NoInterface { hardware: "x".into(), driver: "y".into() }, Verdict::HardBlocked, Verdict::SoftBlocked, Verdict::NotManaged, Verdict::NetworkManagerDown] {
            assert!(!v.title().is_empty() && !v.explanation().is_empty());
        }
    }
}
