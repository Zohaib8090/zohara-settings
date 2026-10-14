//! Live graphics view (Gaming page): each graphics chip with its load, video memory and temperature, and which apps are
//! using which chip.
//!
//! Everything is read without waking a sleeping chip. On a laptop with an NVIDIA chip the driver powers it down when
//! nothing needs it, and asking it anything (`nvidia-smi`, `lspci`, most `/sys` files) wakes it up, which costs battery
//! and caused a multi-second freeze before (see `gpu.rs`). So the chip's `power/runtime_status` is read first, and only
//! a chip that is already awake is asked for numbers; a sleeping one is shown as "Asleep".
//!
//! Which chip an app uses is worked out from the device files it has open: `/dev/nvidiaN` means that NVIDIA chip;
//! otherwise the render node (`/dev/dri/renderD128`...) tells the chip. An app that has both chips open (a game on the
//! stronger chip that also hands pictures to the screen chip) counts as using the stronger one.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const INTEL: u32 = 0x8086;
const AMD: u32 = 0x1002;
const NVIDIA: u32 = 0x10de;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Gpu {
    /// `/sys/class/drm/cardN`
    pub card: PathBuf,
    /// PCI address such as `0000:01:00.0`
    pub slot: String,
    pub vendor: u32,
    pub name: String,
    /// The chip that was set up first, which on a laptop is the everyday (integrated) one.
    pub primary: bool,
    pub awake: bool,
    pub load: Option<u32>,
    pub vram_used_mib: Option<u64>,
    pub vram_total_mib: Option<u64>,
    pub temp_c: Option<u32>,
    pub mhz: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GpuApp {
    pub pid: u32,
    pub name: String,
    /// Index into the list of chips.
    pub gpu: usize,
}

pub fn vendor_label(v: u32) -> &'static str {
    match v {
        INTEL => "Intel",
        AMD => "AMD",
        NVIDIA => "NVIDIA",
        _ => "Graphics",
    }
}

fn hex(text: &str) -> Option<u32> {
    u32::from_str_radix(text.trim().trim_start_matches("0x"), 16).ok()
}

/// The chip's name from the PCI id list (`pci.ids`): vendor line, then an indented device line.
pub fn pci_name(ids: &str, vendor: u32, device: u32) -> Option<String> {
    let v = format!("{vendor:04x}");
    let d = format!("\t{device:04x}");
    let mut in_vendor = false;
    for line in ids.lines() {
        if !line.starts_with('\t') && !line.starts_with('#') && !line.is_empty() {
            in_vendor = line.starts_with(&v);
        } else if in_vendor && line.starts_with(&d) && !line.starts_with("\t\t") {
            return Some(line[d.len()..].trim().to_string());
        }
    }
    None
}

/// `GA107M [GeForce RTX 3050 Mobile]` -> `NVIDIA GeForce RTX 3050 Mobile`.
pub fn short_name(vendor: u32, raw: &str) -> String {
    let inner = raw.rfind('[').and_then(|a| raw[a + 1..].find(']').map(|b| raw[a + 1..a + 1 + b].to_string()));
    let core = inner.unwrap_or_else(|| raw.to_string());
    let label = vendor_label(vendor);
    if core.to_lowercase().starts_with(&label.to_lowercase()) { core } else { format!("{label} {core}") }
}

/// One row of `nvidia-smi --query-gpu=name,utilization.gpu,memory.used,memory.total,temperature.gpu --format=csv,noheader,nounits`.
#[derive(Debug, PartialEq, Eq)]
pub struct NvStats {
    pub name: String,
    pub load: Option<u32>,
    pub used_mib: Option<u64>,
    pub total_mib: Option<u64>,
    pub temp_c: Option<u32>,
}

pub fn parse_nvidia_smi(text: &str) -> Vec<NvStats> {
    text.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(',').map(str::trim).collect();
            if f.len() < 5 {
                return None;
            }
            Some(NvStats { name: f[0].to_string(), load: f[1].parse().ok(), used_mib: f[2].parse().ok(), total_mib: f[3].parse().ok(), temp_c: f[4].parse().ok() })
        })
        .collect()
}

fn read(path: impl AsRef<Path>) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

fn is_card_name(n: &str) -> bool {
    n.strip_prefix("card").is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
}

/// The chips, with live numbers for the ones that are awake. `pci_ids` is the text of `pci.ids` (may be empty).
pub fn read_gpus(pci_ids: &str) -> Vec<Gpu> {
    let mut cards: Vec<PathBuf> = std::fs::read_dir("/sys/class/drm")
        .map(|rd| rd.flatten().filter(|e| is_card_name(&e.file_name().to_string_lossy())).map(|e| e.path()).collect())
        .unwrap_or_default();
    cards.sort();
    let mut gpus = Vec::new();
    for card in cards {
        let dev = card.join("device");
        let Some(vendor) = read(dev.join("vendor")).as_deref().and_then(hex) else { continue };
        let device = read(dev.join("device")).as_deref().and_then(hex).unwrap_or(0);
        let slot = std::fs::read_link(&dev).ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string())).unwrap_or_default();
        let awake = read(dev.join("power/runtime_status")).map(|s| s == "active").unwrap_or(true);
        let raw = pci_name(pci_ids, vendor, device).unwrap_or_else(|| format!("device {device:04x}"));
        let mut g = Gpu {
            slot,
            vendor,
            name: short_name(vendor, &raw),
            primary: read(dev.join("boot_vga")).as_deref() == Some("1"),
            awake,
            card,
            ..Gpu::default()
        };
        if awake {
            fill_live(&mut g);
        }
        gpus.push(g);
    }
    // One call for all NVIDIA chips, and only if one of them is awake.
    if gpus.iter().any(|g| g.vendor == NVIDIA && g.awake) {
        let out = Command::new("nvidia-smi")
            .args(["--query-gpu=name,utilization.gpu,memory.used,memory.total,temperature.gpu", "--format=csv,noheader,nounits"])
            .stderr(Stdio::null())
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();
        let stats = parse_nvidia_smi(&out);
        for (g, s) in gpus.iter_mut().filter(|g| g.vendor == NVIDIA && g.awake).zip(stats) {
            g.name = short_name(NVIDIA, &s.name);
            g.load = s.load;
            g.vram_used_mib = s.used_mib;
            g.vram_total_mib = s.total_mib;
            g.temp_c = s.temp_c;
        }
    }
    gpus
}

fn fill_live(g: &mut Gpu) {
    let dev = g.card.join("device");
    match g.vendor {
        AMD => {
            g.load = read(dev.join("gpu_busy_percent")).and_then(|s| s.parse().ok());
            g.vram_used_mib = read(dev.join("mem_info_vram_used")).and_then(|s| s.parse::<u64>().ok()).map(|b| b / 1048576);
            g.vram_total_mib = read(dev.join("mem_info_vram_total")).and_then(|s| s.parse::<u64>().ok()).map(|b| b / 1048576);
            g.temp_c = std::fs::read_dir(dev.join("hwmon"))
                .ok()
                .and_then(|rd| rd.flatten().find_map(|e| read(e.path().join("temp1_input"))))
                .and_then(|s| s.parse::<u32>().ok())
                .map(|m| m / 1000);
        }
        INTEL => {
            g.mhz = ["gt/gt0/rps_cur_freq_mhz", "gt_cur_freq_mhz"].iter().find_map(|f| read(g.card.join(f))).and_then(|s| s.parse().ok());
        }
        _ => {}
    }
}

/// The line under a chip's name.
pub fn describe(g: &Gpu) -> String {
    if !g.awake {
        return "Asleep: no app is using it, so it is saving power".into();
    }
    let mut parts: Vec<String> = Vec::new();
    if let Some(l) = g.load {
        parts.push(format!("Load {l}%"));
    }
    match (g.vram_used_mib, g.vram_total_mib) {
        (Some(u), Some(t)) => parts.push(format!("Video memory {u} of {t} MiB")),
        (Some(u), None) => parts.push(format!("Video memory {u} MiB")),
        _ => {}
    }
    if let Some(t) = g.temp_c {
        parts.push(format!("{t} °C"));
    }
    if let Some(m) = g.mhz {
        parts.push(format!("{m} MHz"));
    }
    if parts.is_empty() {
        "Awake (no live numbers for this chip)".into()
    } else {
        parts.join(" · ")
    }
}

// ── Which app uses which chip ──────────────────────────────────────────────

/// Desktop parts and helpers that use the graphics chip all the time but are not "an app".
const SKIP: [&str; 25] = [
    "kwin_wayland", "kwin_x11", "kwin_wayland_wrapper", "xwayland", "xorg", "plasmashell", "ksplashqml", "kded6", "krunner", "sddm-greeter-qt6",
    "kscreenlocker_greet", "xdg-desktop-portal-kde", "xdg-desktop-portal-gtk", "zohara-settings", "zohara-store", "steamwebhelper",
    "nvidia-smi", "explorer.exe", "services.exe", "winedevice.exe", "plugplay.exe", "svchost.exe", "rpcss.exe", "start.exe", "conhost.exe",
];

/// A readable name from a process's command line: its first word's last path part (Windows or Linux separators).
pub fn app_name(cmdline: &[u8], comm: &str) -> String {
    // Normally words are separated by NUL; a program that renames itself (Electron, browsers) leaves plain spaces.
    let first = if cmdline.contains(&0) {
        cmdline.split(|b| *b == 0).next().map(|b| String::from_utf8_lossy(b).to_string()).unwrap_or_default()
    } else {
        String::from_utf8_lossy(cmdline).split_whitespace().next().unwrap_or("").to_string()
    };
    // A renamed program has its options after the name in the same word: cut at the first " -".
    let first = first.split(" -").next().unwrap_or("").to_string();
    let base = first.rsplit(['/', '\\']).next().unwrap_or("").trim().to_string();
    if base.is_empty() { comm.trim().to_string() } else { base }
}

pub fn skip(name: &str) -> bool {
    let n = name.to_lowercase();
    SKIP.iter().any(|s| *s == n) || n.starts_with("plasma") || n.starts_with("org.kde.") || n.starts_with("kwin") || n.starts_with("xdg-desktop-portal")
}

/// Parent process id from the text of `/proc/PID/stat` (the name in brackets can hold spaces and brackets).
pub fn parent_of(stat: &str) -> Option<u32> {
    let after = &stat[stat.rfind(')')? + 1..];
    after.split_whitespace().nth(1)?.parse().ok()
}

/// Helpers and child processes inherit their parent's open files, so they would list the same app many times (an
/// Electron app's renderers, `node` started by an app). A process whose parent is also in the list is dropped.
pub fn without_children(found: Vec<(u32, u32, String, usize)>) -> Vec<GpuApp> {
    let pids: HashSet<u32> = found.iter().map(|f| f.0).collect();
    let mut seen: HashSet<(String, usize)> = HashSet::new();
    let mut out: Vec<GpuApp> = Vec::new();
    for (pid, ppid, name, gpu) in found {
        if pids.contains(&ppid) {
            continue;
        }
        if seen.insert((name.clone(), gpu)) {
            out.push(GpuApp { pid, name, gpu });
        }
    }
    out
}

/// Picks the chip an app uses from what it has open. `nvidia` = indexes of NVIDIA chips whose `/dev/nvidiaN` is open,
/// `nodes` = indexes of chips whose render node is open.
pub fn choose_gpu(nvidia: &[usize], nodes: &[usize], gpus: &[Gpu]) -> Option<usize> {
    if let Some(i) = nvidia.first() {
        return Some(*i);
    }
    nodes.iter().copied().find(|i| gpus.get(*i).map(|g| !g.primary).unwrap_or(false)).or_else(|| nodes.first().copied())
}

fn index_of_slot(gpus: &[Gpu], slot: &str) -> Option<usize> {
    gpus.iter().position(|g| g.slot == slot)
}

fn render_node_slot(name: &str) -> Option<String> {
    std::fs::read_link(format!("/sys/class/drm/{name}/device")).ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
}

/// Apps of the current user that have a graphics chip open. One entry per name and chip.
pub fn apps_on_gpus(gpus: &[Gpu]) -> Vec<GpuApp> {
    let nvidia_idx: Vec<usize> = gpus.iter().enumerate().filter(|(_, g)| g.vendor == NVIDIA).map(|(i, _)| i).collect();
    let mut found: Vec<(u32, u32, String, usize)> = Vec::new();
    let Ok(rd) = std::fs::read_dir("/proc") else { return Vec::new() };
    for e in rd.flatten() {
        let Some(pid) = e.file_name().to_string_lossy().parse::<u32>().ok() else { continue };
        let Ok(fds) = std::fs::read_dir(e.path().join("fd")) else { continue };
        let (mut nv, mut nodes): (Vec<usize>, Vec<usize>) = (Vec::new(), Vec::new());
        for fd in fds.flatten() {
            let Ok(target) = std::fs::read_link(fd.path()) else { continue };
            let t = target.to_string_lossy();
            if let Some(n) = t.strip_prefix("/dev/nvidia").and_then(|n| n.parse::<usize>().ok()) {
                if let Some(i) = nvidia_idx.get(n) {
                    nv.push(*i);
                }
            } else if let Some(node) = t.strip_prefix("/dev/dri/").filter(|n| n.starts_with("renderD")) {
                if let Some(i) = render_node_slot(node).and_then(|s| index_of_slot(gpus, &s)) {
                    nodes.push(i);
                }
            }
        }
        let Some(gpu) = choose_gpu(&nv, &nodes, gpus) else { continue };
        let comm = std::fs::read_to_string(e.path().join("comm")).unwrap_or_default();
        let cmd = std::fs::read(e.path().join("cmdline")).unwrap_or_default();
        let name = app_name(&cmd, &comm);
        if name.is_empty() || skip(&name) || pid == std::process::id() {
            continue;
        }
        let ppid = std::fs::read_to_string(e.path().join("stat")).ok().as_deref().and_then(parent_of).unwrap_or(0);
        found.push((pid, ppid, name, gpu));
    }
    let mut out = without_children(found);
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDS: &str = "# comment\n8086  Intel Corporation\n\ta7a8  Raptor Lake-P [UHD Graphics]\n\t\t1234 5678  sub entry\n10de  NVIDIA Corporation\n\t25a2  GA107M [GeForce RTX 3050 Mobile]\n\t25ac  GA107M [GeForce RTX 3050 6GB Laptop GPU]\n";

    fn gpu(vendor: u32, slot: &str, primary: bool) -> Gpu {
        Gpu { vendor, slot: slot.into(), primary, ..Gpu::default() }
    }

    #[test]
    fn names_come_from_the_pci_list_and_are_made_short() {
        assert_eq!(pci_name(IDS, 0x10de, 0x25ac).as_deref(), Some("GA107M [GeForce RTX 3050 6GB Laptop GPU]"));
        assert_eq!(pci_name(IDS, 0x8086, 0xa7a8).as_deref(), Some("Raptor Lake-P [UHD Graphics]"));
        assert_eq!(pci_name(IDS, 0x10de, 0x9999), None);
        assert_eq!(pci_name(IDS, 0x1002, 0x25ac), None);
        assert_eq!(short_name(0x10de, "GA107M [GeForce RTX 3050 6GB Laptop GPU]"), "NVIDIA GeForce RTX 3050 6GB Laptop GPU");
        assert_eq!(short_name(0x8086, "Raptor Lake-P [UHD Graphics]"), "Intel UHD Graphics");
        assert_eq!(short_name(0x1002, "Navi 23 [Radeon RX 6600]"), "AMD Radeon RX 6600");
        assert_eq!(short_name(0x1002, "Cezanne"), "AMD Cezanne");
        assert_eq!(short_name(0x10de, "NVIDIA GeForce RTX 3050"), "NVIDIA GeForce RTX 3050");
    }

    #[test]
    fn nvidia_smi_rows_are_read() {
        let r = parse_nvidia_smi("NVIDIA GeForce RTX 3050 6GB Laptop GPU, 25, 74, 6144, 68\n");
        assert_eq!(r, vec![NvStats { name: "NVIDIA GeForce RTX 3050 6GB Laptop GPU".into(), load: Some(25), used_mib: Some(74), total_mib: Some(6144), temp_c: Some(68) }]);
        assert!(parse_nvidia_smi("").is_empty());
        assert!(parse_nvidia_smi("No devices were found").is_empty());
        let odd = parse_nvidia_smi("X, [N/A], [N/A], 6144, 50\n");
        assert_eq!(odd[0].load, None);
        assert_eq!(odd[0].total_mib, Some(6144));
    }

    #[test]
    fn the_line_under_a_chip_says_what_is_known() {
        let mut g = Gpu { awake: false, ..Gpu::default() };
        assert!(describe(&g).starts_with("Asleep"));
        g = Gpu { awake: true, load: Some(25), vram_used_mib: Some(74), vram_total_mib: Some(6144), temp_c: Some(68), ..Gpu::default() };
        assert_eq!(describe(&g), "Load 25% · Video memory 74 of 6144 MiB · 68 °C");
        g = Gpu { awake: true, mhz: Some(600), ..Gpu::default() };
        assert_eq!(describe(&g), "600 MHz");
        assert!(describe(&Gpu { awake: true, ..Gpu::default() }).starts_with("Awake"));
    }

    #[test]
    fn app_names_are_readable_for_wine_and_native() {
        assert_eq!(app_name(b"Z:\\home\\me\\Cities_Skylines\\Cities.exe\0-x\0", "Cities.exe"), "Cities.exe");
        assert_eq!(app_name(b"/usr/bin/blender\0--python\0", "blender"), "blender");
        assert_eq!(app_name(b"", "mygame"), "mygame");
        assert!(skip("kwin_wayland") && skip("Explorer.exe") && !skip("Cities.exe"));
    }

    #[test]
    fn helpers_started_by_a_listed_app_are_not_listed_again() {
        assert_eq!(parent_of("57266 (Cities.exe) S 57264 57170 0 0"), Some(57264));
        assert_eq!(parent_of("12 (a (b) c) S 7 1 1"), Some(7), "brackets in the name");
        assert_eq!(parent_of("garbage"), None);
        let found = vec![
            (100, 1, "claude-desktop".to_string(), 1),
            (101, 100, "Claude".to_string(), 1),   // child of 100
            (102, 101, "node".to_string(), 1),     // grandchild
            (200, 1, "Cities.exe".to_string(), 0), // parent is not in the list (a skipped wine helper)
            (300, 1, "blender".to_string(), 0),
        ];
        let names: Vec<String> = without_children(found).into_iter().map(|a| a.name).collect();
        assert_eq!(names, vec!["claude-desktop", "Cities.exe", "blender"]);
    }

    #[test]
    fn a_program_that_renamed_itself_is_still_named() {
        assert_eq!(app_name(b"Claude --gpu-preferences=AAA --shared-files", "claude"), "Claude");
        assert_eq!(app_name(b"Claude --gpu-preferences=AAA\0", "claude"), "Claude");
        assert_eq!(app_name(b"Z:\\home\\me\\My Games\\Game.exe\0", "x"), "Game.exe");
        assert!(skip("plasma-keyboard") && skip("org.kde.krunner"));
    }

    #[test]
    fn the_stronger_chip_wins_when_an_app_has_both_open() {
        let g = vec![gpu(NVIDIA, "0000:01:00.0", false), gpu(INTEL, "0000:00:02.0", true)];
        // /dev/nvidia0 open: NVIDIA, even though the Intel render node is open too
        assert_eq!(choose_gpu(&[0], &[0, 1], &g), Some(0));
        // only render nodes, both open: the one that is not the screen chip
        assert_eq!(choose_gpu(&[], &[1, 0], &g), Some(0));
        // only the Intel node
        assert_eq!(choose_gpu(&[], &[1], &g), Some(1));
        assert_eq!(choose_gpu(&[], &[], &g), None);
    }

    #[test]
    fn this_computers_chips_are_read_without_panicking() {
        let gpus = read_gpus("");
        for g in &gpus {
            assert!(!g.name.is_empty());
        }
        let _ = apps_on_gpus(&gpus);
    }
}
