//! Memory behaviour the person can tune: how eagerly the system moves memory to swap (`vm.swappiness`), how long it
//! keeps file caches (`vm.vfs_cache_pressure`) and whether swap is compressed in RAM first (zswap). Values are read
//! live from /proc and /sys; a change is written to the files that apply at every boot and applied now, in one
//! administrator prompt.

use std::process::Command;

pub const SWAPPINESS_DEFAULT: u32 = 60;
pub const CACHE_DEFAULT: u32 = 100;

/// `(label, value)`; the default is marked. Other values (set by hand or an older tool) show as "Custom".
pub const SWAPPINESS: [(&str, u32); 4] = [
    ("Keep apps in memory (10)", 10),
    ("Balanced (60, default)", SWAPPINESS_DEFAULT),
    ("Use swap sooner (100)", 100),
    ("Use swap eagerly (150)", 150),
];

pub const CACHE: [(&str, u32); 3] = [("Keep files cached longer (50)", 50), ("Normal (100, default)", CACHE_DEFAULT), ("Free cache sooner (200)", 200)];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Memory {
    pub swappiness: u32,
    pub cache_pressure: u32,
    pub zswap: bool,
}

impl Default for Memory {
    fn default() -> Self {
        Memory { swappiness: SWAPPINESS_DEFAULT, cache_pressure: CACHE_DEFAULT, zswap: true }
    }
}

fn number(path: &str) -> Option<u32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// What the running kernel does now.
pub fn current() -> Memory {
    Memory {
        swappiness: number("/proc/sys/vm/swappiness").unwrap_or(SWAPPINESS_DEFAULT),
        cache_pressure: number("/proc/sys/vm/vfs_cache_pressure").unwrap_or(CACHE_DEFAULT),
        zswap: std::fs::read_to_string("/sys/module/zswap/parameters/enabled").map(|s| s.trim() == "Y").unwrap_or(false),
    }
}

pub fn zswap_available() -> bool {
    std::path::Path::new("/sys/module/zswap/parameters/enabled").exists()
}

pub fn index_of(options: &[(&str, u32)], value: u32) -> Option<usize> {
    options.iter().position(|o| o.1 == value)
}

fn clamp(m: Memory) -> Memory {
    Memory { swappiness: m.swappiness.min(200), cache_pressure: m.cache_pressure.clamp(1, 500), zswap: m.zswap }
}

/// Files that keep the choice across reboots, as `(path under root, content)`. Everything equal to the system's own
/// default is not written, so choosing the defaults gives the person the untouched system back.
pub fn files(m: Memory) -> Vec<(&'static str, String)> {
    let m = clamp(m);
    let mut f = Vec::new();
    let mut sysctl = String::from("# Written by Zohara Settings (Power & battery > Memory). Delete this file to go back to the system's own values.\n");
    let mut any = false;
    if m.swappiness != SWAPPINESS_DEFAULT {
        sysctl += &format!("vm.swappiness = {}\n", m.swappiness);
        any = true;
    }
    if m.cache_pressure != CACHE_DEFAULT {
        sysctl += &format!("vm.vfs_cache_pressure = {}\n", m.cache_pressure);
        any = true;
    }
    if any {
        f.push(("/etc/sysctl.d/99-zohara-memory.conf", sysctl));
    }
    if !m.zswap {
        f.push(("/etc/tmpfiles.d/zohara-zswap.conf", "# Written by Zohara Settings. Turns the compressed swap cache off at boot.\nw /sys/module/zswap/parameters/enabled - - - - N\n".to_string()));
    }
    f
}

const OWNED: [&str; 2] = ["/etc/sysctl.d/99-zohara-memory.conf", "/etc/tmpfiles.d/zohara-zswap.conf"];

/// Shell script that makes the files right under `root` ("" on a real system). Only numbers and fixed text end up in
/// it, never text the person typed.
pub fn file_script(m: Memory, root: &str) -> String {
    let mut s = String::from("set -e\n");
    for path in OWNED {
        s += &format!("rm -f '{root}{path}'\n");
    }
    for (path, content) in files(m) {
        let dir = path.rsplit_once('/').map(|d| d.0).unwrap_or("");
        s += &format!("mkdir -p '{root}{dir}'\nprintf '%s' '{}' > '{root}{path}'\n", content.replace('\'', ""));
    }
    s
}

/// Script for the real system: files, then the same values into the running kernel.
pub fn apply_script(m: Memory) -> String {
    let m = clamp(m);
    let mut s = file_script(m, "");
    s += &format!("sysctl -q -w vm.swappiness={} vm.vfs_cache_pressure={}\n", m.swappiness, m.cache_pressure);
    s += &format!("[ -w /sys/module/zswap/parameters/enabled ] && echo {} > /sys/module/zswap/parameters/enabled || true\n", if m.zswap { "Y" } else { "N" });
    s
}

/// Applies with one administrator prompt; false when it was cancelled or failed.
pub fn apply(m: Memory) -> bool {
    Command::new("pkexec").args(["sh", "-c", &apply_script(m)]).status().map(|s| s.success()).unwrap_or(false)
}

/// A line for the page: how much memory and swap there is and how much is used.
pub fn summary() -> String {
    let get = |k: &str, text: &str| -> u64 {
        text.lines().find(|l| l.starts_with(k)).and_then(|l| l.split_whitespace().nth(1)).and_then(|v| v.parse().ok()).unwrap_or(0)
    };
    let text = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let gib = |kb: u64| kb as f64 / 1024.0 / 1024.0;
    let (total, avail) = (get("MemTotal:", &text), get("MemAvailable:", &text));
    let (st, sf) = (get("SwapTotal:", &text), get("SwapFree:", &text));
    let swap = if st == 0 { "no swap".to_string() } else { format!("swap {:.1} GiB, {:.1} GiB used", gib(st), gib(st - sf)) };
    format!("Memory {:.1} GiB, {:.1} GiB in use; {swap}", gib(total), gib(total.saturating_sub(avail)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_write_nothing_so_the_system_is_left_alone() {
        assert!(files(Memory::default()).is_empty());
    }

    #[test]
    fn a_changed_value_is_written_for_every_boot() {
        let f = files(Memory { swappiness: 10, cache_pressure: 100, zswap: true });
        assert_eq!(f.len(), 1);
        assert!(f[0].1.contains("vm.swappiness = 10") && !f[0].1.contains("vfs_cache_pressure"));
    }

    #[test]
    fn zswap_off_needs_its_own_boot_rule() {
        let f = files(Memory { zswap: false, ..Memory::default() });
        assert_eq!(f.len(), 1);
        assert!(f[0].0.contains("tmpfiles.d") && f[0].1.contains("enabled - - - - N"));
    }

    #[test]
    fn values_are_kept_in_range() {
        let s = apply_script(Memory { swappiness: 9999, cache_pressure: 0, zswap: true });
        assert!(s.contains("vm.swappiness=200") && s.contains("vm.vfs_cache_pressure=1"));
    }

    #[test]
    fn the_script_writes_and_removes_the_files() {
        let root = std::env::temp_dir().join(format!("zohara-mem-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let r = root.to_str().unwrap();
        let run = |m| {
            let ok = Command::new("sh").arg("-c").arg(file_script(m, r)).status().unwrap().success();
            assert!(ok);
        };
        run(Memory { swappiness: 150, cache_pressure: 50, zswap: false });
        let sysctl = std::fs::read_to_string(root.join("etc/sysctl.d/99-zohara-memory.conf")).unwrap();
        assert!(sysctl.contains("vm.swappiness = 150") && sysctl.contains("vm.vfs_cache_pressure = 50"));
        assert!(root.join("etc/tmpfiles.d/zohara-zswap.conf").exists());
        run(Memory::default());
        assert!(!root.join("etc/sysctl.d/99-zohara-memory.conf").exists());
        assert!(!root.join("etc/tmpfiles.d/zohara-zswap.conf").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_live_system_values_are_readable() {
        let m = current();
        assert!(m.swappiness <= 200 && m.cache_pressure >= 1);
        assert!(summary().starts_with("Memory "));
    }
}

#[cfg(test)]
mod syntax {
    use super::*;

    #[test]
    fn the_administrator_script_is_valid_shell_for_every_combination() {
        for sw in [0, 10, 60, 200] {
            for z in [true, false] {
                let s = apply_script(Memory { swappiness: sw, cache_pressure: 50, zswap: z });
                let ok = Command::new("sh").args(["-n", "-c", &s]).status().unwrap().success();
                assert!(ok, "{s}");
            }
        }
    }
}
