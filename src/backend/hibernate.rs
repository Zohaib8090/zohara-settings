//! Can this computer hibernate? Hibernate writes RAM to swap and reads it back at boot, so it needs a `resume=` kernel option and
//! **the swap that `resume=` names** has to be at least as big as the RAM. The boot code only looks at that one swap, so other
//! swap does not count: zram is memory (an image kept there is gone at power-off), and a swap file the boot code is not told about
//! is never looked at (found 2026-10-20: the check used to add up every swap, so a laptop with an 8.8 GiB resume partition, 7.7 GiB
//! of zram and a 16 GiB swap file was told it could hibernate 15 GiB of RAM). Settings hides the Hibernate choices unless it holds.

use std::fs;

/// Swap list from the text of `/proc/swaps` (header line, then `name type size used priority`): (name, type, KiB).
fn swap_entries(swaps: &str) -> Vec<(String, String, u64)> {
    swaps
        .lines()
        .skip(1)
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let name = f.next()?.to_string();
            let kind = f.next()?.to_string();
            let kib = f.next()?.parse::<u64>().ok()?;
            Some((name, kind, kib))
        })
        .collect()
}

/// `MemTotal` in KiB from the text of `/proc/meminfo`.
fn mem_kib(meminfo: &str) -> u64 {
    meminfo
        .lines()
        .find_map(|l| l.strip_prefix("MemTotal:"))
        .and_then(|v| v.split_whitespace().next()?.parse().ok())
        .unwrap_or(0)
}

/// What the kernel line says to resume from: the device named by `resume=` (`UUID=...`, `/dev/...` and so on, left as written) and
/// whether `resume_offset=` is there (then the swap is a file inside that device's file system).
fn resume_option(cmdline: &str) -> Option<(String, bool)> {
    let spec = cmdline.split_whitespace().find_map(|w| w.strip_prefix("resume="))?.to_string();
    if spec.is_empty() {
        return None;
    }
    let offset = cmdline.split_whitespace().any(|w| w.strip_prefix("resume_offset=").map(|v| !v.is_empty()).unwrap_or(false));
    Some((spec, offset))
}

/// The size in KiB of the swap `resume=` names, or `None` when there is no such swap in use. `resolve` turns what `resume=` says
/// (`UUID=...`, `/dev/sda3`) and each swap's name into one device path so they can be compared.
fn resume_swap_kib(swaps: &str, cmdline: &str, resolve: &dyn Fn(&str) -> Option<String>) -> Option<u64> {
    let (spec, has_offset) = resume_option(cmdline)?;
    let entries = swap_entries(swaps);
    if has_offset {
        // a swap file: the kernel line names the device the file lives on and the offset of the file; count the files, never zram
        let total: u64 = entries.iter().filter(|(n, k, _)| k == "file" && !n.contains("/zram")).map(|(_, _, s)| *s).sum();
        return (total > 0).then_some(total);
    }
    let target = resolve(&spec)?;
    entries.iter().find(|(n, _, _)| !n.contains("/zram") && resolve(n).as_deref() == Some(target.as_str())).map(|(_, _, s)| *s)
}

/// Pure decision: the swap `resume=` names is at least as big as the RAM.
pub fn can_hibernate_from(swaps: &str, meminfo: &str, cmdline: &str, resolve: &dyn Fn(&str) -> Option<String>) -> bool {
    let mem = mem_kib(meminfo);
    mem > 0 && resume_swap_kib(swaps, cmdline, resolve).map(|s| s >= mem).unwrap_or(false)
}

/// Turns `UUID=...`, `PARTUUID=...`, `LABEL=...` or a device path into the real device path (symbolic links followed).
fn resolve_device(spec: &str) -> Option<String> {
    let path = if let Some(v) = spec.strip_prefix("UUID=") {
        format!("/dev/disk/by-uuid/{v}")
    } else if let Some(v) = spec.strip_prefix("PARTUUID=") {
        format!("/dev/disk/by-partuuid/{v}")
    } else if let Some(v) = spec.strip_prefix("LABEL=") {
        format!("/dev/disk/by-label/{v}")
    } else if spec.starts_with('/') {
        spec.to_string()
    } else {
        return None;
    };
    fs::canonicalize(path).ok().map(|p| p.to_string_lossy().to_string())
}

pub fn can_hibernate() -> bool {
    let read = |p: &str| fs::read_to_string(p).unwrap_or_default();
    can_hibernate_from(&read("/proc/swaps"), &read("/proc/meminfo"), &read("/proc/cmdline"), &resolve_device)
}

#[cfg(test)]
mod tests {
    use super::*;
    const HDR: &str = "Filename\tType\tSize\tUsed\tPriority\n";
    const MEM8: &str = "MemTotal:        8000000 kB\nMemFree: 1 kB\n";
    /// A fake `resolve`: UUID=y is /dev/sda3 (as are a few spellings of it), everything starting with /dev/ is itself.
    fn resolve(spec: &str) -> Option<String> {
        match spec {
            "UUID=y" => Some("/dev/sda3".into()),
            s if s.starts_with("/dev/") => Some(s.to_string()),
            _ => None,
        }
    }

    #[test]
    fn no_swap_cannot_hibernate() {
        assert!(!can_hibernate_from(HDR, MEM8, "root=UUID=x resume=UUID=y", &resolve));
    }
    #[test]
    fn small_resume_swap_cannot_hibernate() {
        let s = format!("{HDR}/dev/sda3\tpartition\t4000000\t0\t-2\n");
        assert!(!can_hibernate_from(&s, MEM8, "resume=UUID=y", &resolve));
    }
    #[test]
    fn big_resume_swap_with_resume_can() {
        let s = format!("{HDR}/dev/sda3\tpartition\t8000000\t0\t-2\n");
        assert!(can_hibernate_from(&s, MEM8, "root=UUID=x resume=UUID=y quiet", &resolve));
        assert!(can_hibernate_from(&s, MEM8, "resume=/dev/sda3", &resolve));
    }
    #[test]
    fn big_swap_without_resume_cannot() {
        let s = format!("{HDR}/dev/sda3\tpartition\t8000000\t0\t-2\n");
        assert!(!can_hibernate_from(&s, MEM8, "root=UUID=x quiet", &resolve));
        assert!(!can_hibernate_from(&s, MEM8, "resume=", &resolve));
    }
    #[test]
    fn zram_never_counts() {
        // a lot of zram, a small resume partition: the old check said yes
        let s = format!("{HDR}/dev/zram0\tpartition\t8000000\t0\t100\n/dev/sda3\tpartition\t4000000\t0\t-2\n");
        assert!(!can_hibernate_from(&s, MEM8, "resume=UUID=y", &resolve));
        // resume pointing at zram (a mistake): no
        let z = format!("{HDR}/dev/zram0\tpartition\t8000000\t0\t100\n");
        assert!(!can_hibernate_from(&z, MEM8, "resume=/dev/zram0", &resolve));
    }
    #[test]
    fn other_swap_does_not_add_up() {
        // the owner's laptop on 2026-10-20: 15.3 GiB RAM, resume partition 8.8 GiB, zram 7.7 GiB, swap file 16 GiB
        let mem = "MemTotal:       16000000 kB\n";
        let s = format!("{HDR}/dev/zram0\tpartition\t8080000\t0\t100\n/dev/sda3\tpartition\t9227000\t0\t-2\n/swap/swapfile\tfile\t16777212\t0\t10\n");
        assert!(!can_hibernate_from(&s, mem, "resume=UUID=y", &resolve));
    }
    #[test]
    fn resume_swap_must_be_in_use() {
        // resume names a device that is not an active swap
        let s = format!("{HDR}/dev/sdb1\tpartition\t9000000\t0\t-2\n");
        assert!(!can_hibernate_from(&s, MEM8, "resume=UUID=y", &resolve));
        // an unknown spelling cannot be resolved
        assert!(!can_hibernate_from(&s, MEM8, "resume=whatever", &resolve));
    }
    #[test]
    fn a_swap_file_with_an_offset_counts_files_only() {
        let s = format!("{HDR}/dev/zram0\tpartition\t9000000\t0\t100\n/swap/swapfile\tfile\t8500000\t0\t10\n");
        assert!(can_hibernate_from(&s, MEM8, "resume=UUID=y resume_offset=12345", &resolve));
        let small = format!("{HDR}/dev/zram0\tpartition\t9000000\t0\t100\n/swap/swapfile\tfile\t3000000\t0\t10\n");
        assert!(!can_hibernate_from(&small, MEM8, "resume=UUID=y resume_offset=12345", &resolve));
        // an offset but no swap file in use
        let none = format!("{HDR}/dev/zram0\tpartition\t9000000\t0\t100\n");
        assert!(!can_hibernate_from(&none, MEM8, "resume=UUID=y resume_offset=12345", &resolve));
    }
    #[test]
    fn unreadable_files_mean_no() {
        assert!(!can_hibernate_from("", "", "", &resolve));
    }

    /// Prints the answer for the machine the tests run on: `cargo test real_system -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_system_answer() {
        println!("can_hibernate() on this machine: {}", can_hibernate());
    }
}
