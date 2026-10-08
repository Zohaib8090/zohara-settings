//! Can this computer hibernate? Hibernate writes RAM to swap and reads it back at boot, so it needs swap at least as big
//! as the RAM in use and a `resume=` kernel option. The installer used to offer only "No swap" and "Swap (no Hibernate)",
//! so on those installs the Hibernate choices in Power & battery could never work. Settings hides them unless both hold.

use std::fs;

/// Swap size in KiB from the text of `/proc/swaps` (header line, then `name type size used priority`).
fn swap_kib(swaps: &str) -> u64 {
    swaps.lines().skip(1).filter_map(|l| l.split_whitespace().nth(2)?.parse::<u64>().ok()).sum()
}

/// `MemTotal` in KiB from the text of `/proc/meminfo`.
fn mem_kib(meminfo: &str) -> u64 {
    meminfo
        .lines()
        .find_map(|l| l.strip_prefix("MemTotal:"))
        .and_then(|v| v.split_whitespace().next()?.parse().ok())
        .unwrap_or(0)
}

/// Pure decision: swap at least as big as RAM and a non-empty `resume=` kernel option.
pub fn can_hibernate_from(swaps: &str, meminfo: &str, cmdline: &str) -> bool {
    let mem = mem_kib(meminfo);
    mem > 0 && swap_kib(swaps) >= mem && cmdline.split_whitespace().any(|w| w.starts_with("resume=") && w.len() > "resume=".len())
}

pub fn can_hibernate() -> bool {
    let read = |p: &str| fs::read_to_string(p).unwrap_or_default();
    can_hibernate_from(&read("/proc/swaps"), &read("/proc/meminfo"), &read("/proc/cmdline"))
}

#[cfg(test)]
mod tests {
    use super::*;
    const MEM: &str = "MemTotal:        8000000 kB\nMemFree: 1 kB\n";
    const HDR: &str = "Filename\tType\tSize\tUsed\tPriority\n";

    #[test]
    fn no_swap_cannot_hibernate() {
        assert!(!can_hibernate_from(HDR, MEM, "root=UUID=x resume=UUID=y"));
    }
    #[test]
    fn small_swap_cannot_hibernate() {
        let s = format!("{HDR}/dev/zram0\tpartition\t4000000\t0\t100\n");
        assert!(!can_hibernate_from(&s, MEM, "resume=UUID=y"));
    }
    #[test]
    fn big_swap_without_resume_cannot_hibernate() {
        let s = format!("{HDR}/dev/sda3\tpartition\t8000000\t0\t-2\n");
        assert!(!can_hibernate_from(&s, MEM, "root=UUID=x quiet"));
        assert!(!can_hibernate_from(&s, MEM, "resume="));
    }
    #[test]
    fn big_swap_with_resume_can() {
        let s = format!("{HDR}/dev/sda3\tpartition\t8000000\t0\t-2\n");
        assert!(can_hibernate_from(&s, MEM, "root=UUID=x resume=UUID=y quiet"));
    }
    #[test]
    fn several_swaps_add_up() {
        let s = format!("{HDR}/a\tpartition\t4000000\t0\t-2\n/b\tfile\t4000000\t0\t-3\n");
        assert!(can_hibernate_from(&s, MEM, "resume=/dev/a"));
    }
    #[test]
    fn unreadable_files_mean_no() {
        assert!(!can_hibernate_from("", "", ""));
    }
}
