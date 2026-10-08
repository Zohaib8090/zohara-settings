//! Recommended programs: the ones Zohara adds to the standard install after a computer was set up.
//!
//! The list ships with Settings (`/usr/share/zohara/recommended.json`). Zohara Update offers each program that is
//! not installed yet, with a tick box. Whatever is left unticked is remembered as skipped and not offered again; the
//! "Skipped" list on the page brings one back. A program can name hardware it applies to (`"when": "intel-gpu"`), so
//! an AMD-only computer is not offered Intel video acceleration.
//!
//! Only names from the shipped list can be installed: nothing typed or fetched elsewhere reaches pacman.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;

const LIST_PATH: &str = "/usr/share/zohara/recommended.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    /// The pacman package.
    pub package: String,
    /// What a person sees: "Screenshots".
    pub title: String,
    /// One plain sentence on why.
    pub why: String,
    /// Hardware this applies to: `intel-gpu`, `amd-gpu`, `nvidia-gpu` or `hybrid-gpu`. Empty: every computer.
    #[serde(default)]
    pub when: Option<String>,
    /// A service to turn on after installing, like `switcheroo-control.service`.
    #[serde(default)]
    pub enable: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct List {
    #[serde(default)]
    items: Vec<Item>,
}

/// What the page shows: programs to offer now, and the ones this person skipped.
#[derive(Debug, Default, Clone)]
pub struct Offer {
    pub new: Vec<Item>,
    pub skipped: Vec<Item>,
}

// ── Hardware ───────────────────────────────────────────────────────────────

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Gpus {
    pub intel: bool,
    pub amd: bool,
    pub nvidia: bool,
}

impl Gpus {
    fn count(&self) -> usize {
        [self.intel, self.amd, self.nvidia].iter().filter(|b| **b).count()
    }
}

/// Graphics chips from `lspci -nn` output.
pub fn parse_gpus(lspci: &str) -> Gpus {
    let mut g = Gpus::default();
    for line in lspci.lines() {
        let l = line.to_lowercase();
        if !(l.contains("vga compatible controller") || l.contains("3d controller") || l.contains("display controller")) {
            continue;
        }
        if l.contains("nvidia") {
            g.nvidia = true;
        } else if l.contains("intel") {
            g.intel = true;
        } else if l.contains("amd") || l.contains("advanced micro devices") || l.contains("ati ") {
            g.amd = true;
        }
    }
    g
}

pub fn detect_gpus() -> Gpus {
    Command::new("lspci")
        .arg("-nn")
        .stderr(Stdio::null())
        .output()
        .map(|o| parse_gpus(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
}

/// Whether an item with this `when` applies. A condition this version doesn't know is not offered.
pub fn applies(when: Option<&str>, gpus: &Gpus) -> bool {
    match when.map(str::trim).filter(|w| !w.is_empty()) {
        None => true,
        Some("intel-gpu") => gpus.intel,
        Some("amd-gpu") => gpus.amd,
        Some("nvidia-gpu") => gpus.nvidia,
        Some("hybrid-gpu") => gpus.count() >= 2,
        Some(_) => false,
    }
}

// ── What this person skipped ───────────────────────────────────────────────

#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    #[serde(default)]
    skipped: Vec<String>,
}

fn state_path() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config"));
    base.join("zohara").join("recommended.json")
}

fn load_state_at(path: &std::path::Path) -> State {
    std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn save_state_at(path: &std::path::Path, state: &State) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_string_pretty(state) {
        let _ = std::fs::write(path, text);
    }
}

fn skip_at(path: &std::path::Path, packages: &[String]) {
    let mut s = load_state_at(path);
    for p in packages {
        if !s.skipped.contains(p) {
            s.skipped.push(p.clone());
        }
    }
    save_state_at(path, &s);
}

/// Offer a skipped program again.
pub fn unskip(package: &str) {
    let path = state_path();
    let mut s = load_state_at(&path);
    s.skipped.retain(|p| p != package);
    save_state_at(&path, &s);
}

// ── The list, and what to offer ────────────────────────────────────────────

pub fn parse_list(text: &str) -> Vec<Item> {
    serde_json::from_str::<List>(text).map(|l| l.items).unwrap_or_default()
}

fn load_list() -> Vec<Item> {
    // Development builds can try another list; a release build only ever reads the shipped one.
    #[cfg(debug_assertions)]
    let path = std::env::var("ZOHARA_RECOMMENDED_LIST").unwrap_or_else(|_| LIST_PATH.to_string());
    #[cfg(not(debug_assertions))]
    let path = LIST_PATH.to_string();
    std::fs::read_to_string(path).map(|t| parse_list(&t)).unwrap_or_default()
}

fn installed_packages() -> HashSet<String> {
    Command::new("pacman")
        .arg("-Qq")
        .stderr(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().map(|l| l.trim().to_string()).collect())
        .unwrap_or_default()
}

/// Pure part of `check`: from the list, what is installed, what was skipped and the hardware.
pub fn decide(list: Vec<Item>, installed: &HashSet<String>, skipped: &[String], gpus: &Gpus) -> Offer {
    let mut offer = Offer::default();
    for item in list {
        if installed.contains(&item.package) || !applies(item.when.as_deref(), gpus) {
            continue;
        }
        if skipped.contains(&item.package) {
            offer.skipped.push(item);
        } else {
            offer.new.push(item);
        }
    }
    offer
}

/// What to offer on this computer. Reads the system; call off the UI thread.
pub fn check() -> Offer {
    decide(load_list(), &installed_packages(), &load_state_at(&state_path()).skipped, &detect_gpus())
}

// ── Installing ─────────────────────────────────────────────────────────────

fn safe_token(s: &str) -> bool {
    !s.is_empty() && s.len() < 200 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"@._+-".contains(&b))
}

/// Installs the ticked programs and remembers the unticked ones as skipped. Only programs from the shipped list are
/// accepted. `pinned` says the package date is pinned right now (see `manifest::current_pinned_date`): refreshing
/// the package lists without a full upgrade is only safe then.
pub fn install(ticked: &[String], unticked: &[String], pinned: bool, tx: &Sender<String>) -> Result<(), String> {
    let list = load_list();
    install_from(&list, &state_path(), ticked, unticked, pinned, tx, &mut |cmd| super::updates::run_logged(cmd, tx))
}

fn install_from(
    list: &[Item],
    state: &std::path::Path,
    ticked: &[String],
    unticked: &[String],
    pinned: bool,
    tx: &Sender<String>,
    run: &mut dyn FnMut(Command) -> Result<(), String>,
) -> Result<(), String> {
    let chosen: Vec<&Item> = list.iter().filter(|i| ticked.contains(&i.package)).collect();
    if ticked.iter().any(|t| !list.iter().any(|i| &i.package == t)) || chosen.iter().any(|i| !safe_token(&i.package)) {
        return Err("A program name looked wrong, so nothing was changed.".into());
    }
    let skip_now: Vec<String> = unticked.iter().filter(|u| list.iter().any(|i| &i.package == *u)).cloned().collect();
    // Remembered first: unticking means "not this one", whatever happens to the rest.
    skip_at(state, &skip_now);
    if chosen.is_empty() {
        return Ok(());
    }
    if !super::manifest::sync_args_safe(&["-Sy", "--noconfirm"], pinned) {
        return Err("Refused: refreshing the package lists without a full upgrade could break the system.".into());
    }
    let units: Vec<&str> = chosen.iter().filter_map(|i| i.enable.as_deref()).collect();
    if units.iter().any(|u| !safe_token(u)) {
        return Err("A service name looked wrong, so nothing was changed.".into());
    }
    let _ = tx.send(format!("Installing {}…", chosen.iter().map(|i| i.title.as_str()).collect::<Vec<_>>().join(", ")));
    let script = "units=\"$1\"; shift; pacman -Sy --noconfirm && pacman -S --noconfirm --needed \"$@\" && \
                  for u in $units; do systemctl enable --now \"$u\" || true; done";
    let mut c = Command::new("pkexec");
    c.args(["sh", "-c", script, "sh", &units.join(" ")]);
    c.args(chosen.iter().map(|i| i.package.as_str()));
    run(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(pkg: &str, when: Option<&str>) -> Item {
        Item { package: pkg.into(), title: pkg.into(), why: "why".into(), when: when.map(String::from), enable: None }
    }
    fn set(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }
    fn temp_state(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("zs-rec-{tag}-{}", std::process::id())).join("recommended.json")
    }

    const DELL: &str = "00:02.0 VGA compatible controller [0300]: Intel Corporation Raptor Lake-P [UHD Graphics] [8086:a7a8] (rev 04)\n01:00.0 VGA compatible controller [0300]: NVIDIA Corporation GA107BM [GeForce RTX 3050 6GB Laptop GPU] [10de:25ac] (rev a1)\n00:1f.3 Audio device [0403]: Intel Corporation Raptor Lake-P/U/H cAVS\n";

    #[test]
    fn graphics_chips_are_read_from_lspci_and_audio_is_ignored() {
        assert_eq!(parse_gpus(DELL), Gpus { intel: true, amd: false, nvidia: true });
        assert_eq!(parse_gpus("03:00.0 VGA compatible controller [0300]: Advanced Micro Devices, Inc. [AMD/ATI] Navi 23\n"), Gpus { intel: false, amd: true, nvidia: false });
        assert_eq!(parse_gpus("00:1f.3 Audio device: Intel Corporation cAVS\n"), Gpus::default());
        assert_eq!(parse_gpus("00:02.0 Display controller [0380]: Intel Corporation Arc A370M\n").intel, true);
    }

    #[test]
    fn hardware_conditions() {
        let hybrid = parse_gpus(DELL);
        let amd_only = Gpus { intel: false, amd: true, nvidia: false };
        assert!(applies(None, &amd_only));
        assert!(applies(Some(""), &amd_only));
        assert!(applies(Some("intel-gpu"), &hybrid));
        assert!(!applies(Some("intel-gpu"), &amd_only));
        assert!(applies(Some("hybrid-gpu"), &hybrid));
        assert!(!applies(Some("hybrid-gpu"), &Gpus { intel: true, amd: false, nvidia: false }));
        assert!(!applies(Some("something-newer"), &hybrid), "an unknown condition is not offered");
    }

    #[test]
    fn offers_what_is_missing_and_fits_and_separates_the_skipped() {
        let list = vec![item("spectacle", None), item("intel-media-driver", Some("intel-gpu")), item("switcheroo-control", Some("hybrid-gpu")), item("have", None), item("waydroid", None)];
        let amd = Gpus { intel: false, amd: true, nvidia: false };
        let o = decide(list.clone(), &set(&["have"]), &["waydroid".to_string()], &amd);
        assert_eq!(o.new.iter().map(|i| i.package.as_str()).collect::<Vec<_>>(), vec!["spectacle"]);
        assert_eq!(o.skipped.iter().map(|i| i.package.as_str()).collect::<Vec<_>>(), vec!["waydroid"]);
        let o = decide(list, &set(&[]), &[], &parse_gpus(DELL));
        assert_eq!(o.new.len(), 5 - 0, "all five fit a hybrid Intel+NVIDIA laptop");
    }

    #[test]
    fn the_shipped_list_file_parses() {
        let text = include_str!("../../data/recommended.json");
        let items = parse_list(text);
        assert!(!items.is_empty());
        for i in &items {
            assert!(safe_token(&i.package), "{}", i.package);
            assert!(!i.title.is_empty() && !i.why.is_empty(), "{}", i.package);
            if let Some(w) = &i.when {
                assert!(["intel-gpu", "amd-gpu", "nvidia-gpu", "hybrid-gpu"].contains(&w.as_str()), "{w}");
            }
            if let Some(u) = &i.enable {
                assert!(safe_token(u), "{u}");
            }
        }
    }

    #[test]
    fn bad_json_gives_an_empty_list_not_a_crash() {
        assert!(parse_list("not json").is_empty());
        assert!(parse_list("{}").is_empty());
    }

    #[test]
    fn only_programs_from_the_list_can_be_installed() {
        let list = vec![item("spectacle", None)];
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut ran = 0;
        let r = install_from(&list, &temp_state("bad"), &["spectacle".into(), "rm-rf".into()], &[], true, &tx, &mut |_| {
            ran += 1;
            Ok(())
        });
        assert!(r.is_err());
        assert_eq!(ran, 0);
    }

    #[test]
    fn installing_builds_one_administrator_command_and_enables_the_service() {
        let mut sw = item("switcheroo-control", Some("hybrid-gpu"));
        sw.enable = Some("switcheroo-control.service".into());
        let list = vec![item("spectacle", None), sw];
        let (tx, _rx) = std::sync::mpsc::channel();
        let state = temp_state("ok");
        let mut seen: Vec<String> = Vec::new();
        let r = install_from(&list, &state, &["switcheroo-control".into()], &["spectacle".into()], true, &tx, &mut |c| {
            seen = std::iter::once(c.get_program()).chain(c.get_args()).map(|a| a.to_string_lossy().into_owned()).collect();
            Ok(())
        });
        assert!(r.is_ok());
        assert_eq!(seen[0], "pkexec");
        assert!(seen.contains(&"switcheroo-control".to_string()));
        assert!(seen.contains(&"switcheroo-control.service".to_string()));
        assert!(!seen.contains(&"spectacle".to_string()), "the unticked one is not installed");
        assert!(load_state_at(&state).skipped.contains(&"spectacle".to_string()), "and is remembered as skipped");
        let _ = std::fs::remove_dir_all(state.parent().unwrap());
    }

    #[test]
    fn refreshing_the_lists_without_the_date_pinned_is_refused() {
        let list = vec![item("spectacle", None)];
        let (tx, _rx) = std::sync::mpsc::channel();
        let r = install_from(&list, &temp_state("unpinned"), &["spectacle".into()], &[], false, &tx, &mut |_| Ok(()));
        assert!(r.is_err());
    }

    /// What this computer would be offered. `cargo test offer_here -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn offer_here() {
        let list = parse_list(include_str!("../../data/recommended.json"));
        let gpus = detect_gpus();
        println!("gpus: {gpus:?}");
        let o = decide(list, &installed_packages(), &[], &gpus);
        for i in &o.new {
            println!("OFFER  {:<22} {}  [{}]", i.package, i.title, i.when.as_deref().unwrap_or("all"));
        }
        println!("{} offered", o.new.len());
    }
}
