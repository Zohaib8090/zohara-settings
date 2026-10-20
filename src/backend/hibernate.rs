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

/// The file the installer writes for "Swap (no Hibernate)": it makes the sign-in screen and power menu hide Hibernate.
pub const BLOCK_FILE: &str = "/etc/systemd/sleep.conf.d/10-zohara-no-hibernate.conf";
pub const GRUB_DEFAULT: &str = "/etc/default/grub";
const UNBLOCK_UNIT: &str = "/etc/systemd/system/zohara-hibernate-unblock.service";

/// Exit codes of `enable_script` that mean something to the person.
pub const EXIT_TOO_SMALL: i32 = 6;
pub const EXIT_NO_GRUB: i32 = 7;
pub const EXIT_NO_UUID: i32 = 8;
pub const EXIT_NO_OFFSET: i32 = 9;

/// Where this computer stands on Hibernate, for the Swap page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setup {
    /// Works as far as the settings go (still test it once).
    Ready,
    /// The start-up setting is written, but this start-up does not have it yet: restart once.
    RestartNeeded,
    /// There is enough swap; only the start-up setting (and maybe the installer's block file) is missing.
    CanSetUp,
    /// There is disk swap, but smaller than memory.
    TooSmall,
    NoSwap,
}

/// `GRUB_CMDLINE_LINUX_DEFAULT` in `/etc/default/grub` already says `resume=`.
fn grub_has_resume(grub_default: &str) -> bool {
    grub_default.lines().any(|l| l.trim_start().starts_with("GRUB_CMDLINE_LINUX_DEFAULT=") && l.contains("resume="))
}

/// Pure decision behind the Swap page's Hibernate row.
pub fn setup_state_from(swaps: &str, meminfo: &str, cmdline: &str, grub_default: &str, blocked: bool, resolve: &dyn Fn(&str) -> Option<String>) -> Setup {
    if can_hibernate_from(swaps, meminfo, cmdline, resolve) {
        // everything the running system needs is there; only the installer's block file can still be in the way
        return if blocked { Setup::CanSetUp } else { Setup::Ready };
    }
    let mem = mem_kib(meminfo);
    let disk: Vec<_> = swap_entries(swaps).into_iter().filter(|(n, _, _)| !n.contains("/zram")).collect();
    if disk.is_empty() {
        return Setup::NoSwap;
    }
    if mem == 0 || !disk.iter().any(|(_, _, s)| *s >= mem) {
        return Setup::TooSmall;
    }
    if grub_has_resume(grub_default) && resume_option(cmdline).is_none() {
        Setup::RestartNeeded
    } else {
        Setup::CanSetUp
    }
}

pub fn setup_state() -> Setup {
    let read = |p: &str| fs::read_to_string(p).unwrap_or_default();
    setup_state_from(
        &read("/proc/swaps"),
        &read("/proc/meminfo"),
        &read("/proc/cmdline"),
        &read(GRUB_DEFAULT),
        std::path::Path::new(BLOCK_FILE).exists(),
        &resolve_device,
    )
}

/// Script (run as administrator with `pkexec sh -c`) that sets Hibernate up on the swap that is big enough: writes `resume=`
/// (and `resume_offset=` for a swap file) into the GRUB settings, makes sure the boot image has the `resume` step, rebuilds the
/// boot menu and removes the installer's block file. The block file stays until the next start if this start-up lacks `resume=`:
/// the running kernel would write an image it could not read back, so the Hibernate buttons must not show before a restart.
/// Fixed text only, nothing the person typed. If the boot menu cannot be rebuilt, the GRUB settings are put back.
pub fn enable_script() -> String {
    ENABLE_SCRIPT.replace("@BLOCK@", BLOCK_FILE).replace("@UNIT@", UNBLOCK_UNIT)
}

const ENABLE_SCRIPT: &str = r#"set -e
MEM=$(awk '/^MemTotal:/ {print $2}' /proc/meminfo)
PART=$(awk -v m="$MEM" 'NR>1 && $2=="partition" && $1 !~ /zram/ && $3+0>=m+0 {print $1; exit}' /proc/swaps)
FILE=$(awk -v m="$MEM" 'NR>1 && $2=="file" && $3+0>=m+0 {print $1; exit}' /proc/swaps)
[ -f /etc/default/grub ] || exit 7
if [ -n "$PART" ]; then
  UUID=$(blkid -s UUID -o value "$PART" || true)
  [ -n "$UUID" ] || exit 8
  ARGS="resume=UUID=$UUID"
elif [ -n "$FILE" ]; then
  DEV=$(findmnt -no SOURCE -T "$FILE" | sed 's/\[.*//')
  UUID=$(blkid -s UUID -o value "$DEV" || true)
  [ -n "$UUID" ] || exit 8
  if [ "$(findmnt -no FSTYPE -T "$FILE")" = btrfs ]; then
    OFF=$(btrfs inspect-internal map-swapfile -r "$FILE" || true)
  else
    OFF=$(filefrag -v "$FILE" | awk '$1=="0:" {gsub(/\./,"",$4); print $4; exit}' || true)
  fi
  [ -n "$OFF" ] || exit 9
  ARGS="resume=UUID=$UUID resume_offset=$OFF"
else
  exit 6
fi
REBUILD=0
if ! grep -q '^HOOKS=.*\bsystemd\b' /etc/mkinitcpio.conf && ! grep -q '^HOOKS=.*\bresume\b' /etc/mkinitcpio.conf; then
  sed -i -E '/^HOOKS=/ s/\bfilesystems\b/resume filesystems/' /etc/mkinitcpio.conf
  REBUILD=1
fi
BAK=$(mktemp)
cp /etc/default/grub "$BAK"
cp -n /etc/default/grub /etc/default/grub.zohara-before-hibernate 2>/dev/null || true
trap 'rc=$?; if [ $rc -ne 0 ]; then cp "$BAK" /etc/default/grub; fi; rm -f "$BAK"; exit $rc' EXIT
sed -i -E "/^GRUB_CMDLINE_LINUX_DEFAULT=/ { s/[[:space:]]+resume(_offset)?=[^[:space:]\"']*//g ; s/([\"'])resume(_offset)?=[^[:space:]\"']*[[:space:]]*/\1/ ; s|([\"'])\$| $ARGS\\1| }" /etc/default/grub
grep -q "^GRUB_CMDLINE_LINUX_DEFAULT=.*resume=" /etc/default/grub
grub-mkconfig -o /boot/grub/grub.cfg >/dev/null 2>&1
[ "$REBUILD" = 1 ] && mkinitcpio -P >/dev/null 2>&1
NEED=0
for a in $ARGS; do grep -qw -- "$a" /proc/cmdline || NEED=1; done
if [ -e @BLOCK@ ]; then
  if [ "$NEED" = 0 ]; then
    rm -f @BLOCK@
  else
    cat > @UNIT@ <<'EOF2'
[Unit]
Description=Allow Hibernate now that the start-up setting is active (Zohara Settings)
ConditionKernelCommandLine=resume
ConditionPathExists=@BLOCK@

[Service]
Type=oneshot
ExecStart=/usr/bin/rm -f @BLOCK@
ExecStartPost=-/usr/bin/systemctl disable zohara-hibernate-unblock.service

[Install]
WantedBy=multi-user.target
EOF2
    systemctl daemon-reload
    systemctl enable zohara-hibernate-unblock.service
  fi
fi
exit 0
"#;

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

    const GRUB_NO: &str = "GRUB_TIMEOUT=5\nGRUB_CMDLINE_LINUX_DEFAULT='quiet splash loglevel=3'\n";
    const GRUB_YES: &str = "GRUB_CMDLINE_LINUX_DEFAULT='quiet splash resume=UUID=y loglevel=3'\n";

    #[test]
    fn setup_ready_only_when_nothing_blocks() {
        let s = format!("{HDR}/dev/sda3\tpartition\t8000000\t0\t-2\n");
        assert_eq!(setup_state_from(&s, MEM8, "resume=UUID=y", GRUB_YES, false, &resolve), Setup::Ready);
        // the installer's block file is still there: it can be set up (the script removes it)
        assert_eq!(setup_state_from(&s, MEM8, "resume=UUID=y", GRUB_YES, true, &resolve), Setup::CanSetUp);
    }
    #[test]
    fn setup_can_be_done_when_swap_is_big_enough() {
        let s = format!("{HDR}/dev/zram0\tpartition\t2000000\t0\t100\n/dev/sda3\tpartition\t8000000\t0\t-2\n");
        assert_eq!(setup_state_from(&s, MEM8, "quiet splash", GRUB_NO, true, &resolve), Setup::CanSetUp);
        let f = format!("{HDR}/swap/swapfile\tfile\t8500000\t0\t10\n");
        assert_eq!(setup_state_from(&f, MEM8, "quiet", GRUB_NO, false, &resolve), Setup::CanSetUp);
    }
    #[test]
    fn setup_waits_for_a_restart_once_grub_says_resume() {
        let s = format!("{HDR}/dev/sda3\tpartition\t8000000\t0\t-2\n");
        assert_eq!(setup_state_from(&s, MEM8, "quiet splash", GRUB_YES, true, &resolve), Setup::RestartNeeded);
        // a resume= that points at nothing usable is not "done": set it up again
        assert_eq!(setup_state_from(&s, MEM8, "resume=UUID=other", GRUB_YES, false, &resolve), Setup::CanSetUp);
    }
    #[test]
    fn setup_not_possible_without_big_enough_disk_swap() {
        let small = format!("{HDR}/dev/zram0\tpartition\t9000000\t0\t100\n/dev/sda3\tpartition\t4000000\t0\t-2\n");
        assert_eq!(setup_state_from(&small, MEM8, "quiet", GRUB_NO, false, &resolve), Setup::TooSmall);
        let only_zram = format!("{HDR}/dev/zram0\tpartition\t9000000\t0\t100\n");
        assert_eq!(setup_state_from(&only_zram, MEM8, "quiet", GRUB_NO, false, &resolve), Setup::NoSwap);
        assert_eq!(setup_state_from(HDR, MEM8, "quiet", GRUB_NO, false, &resolve), Setup::NoSwap);
    }
    #[test]
    fn grub_resume_is_read_from_the_default_line_only() {
        assert!(grub_has_resume(GRUB_YES));
        assert!(!grub_has_resume(GRUB_NO));
        assert!(!grub_has_resume("# GRUB_CMDLINE_LINUX_DEFAULT='resume=x'\nGRUB_CMDLINE_LINUX='resume=y'\n"));
    }
    #[test]
    fn the_script_is_valid_shell_with_the_real_paths() {
        let sc = enable_script();
        assert!(!sc.contains('@'), "placeholders left in the script");
        assert!(sc.contains(BLOCK_FILE));
        assert!(sc.contains("grub-mkconfig"));
        let ok = std::process::Command::new("bash").args(["-n", "-c", &sc]).status().map(|s| s.success()).unwrap_or(false);
        assert!(ok, "bash -n rejected the script");
    }
    #[test]
    fn the_grub_edit_replaces_old_resume_and_keeps_the_rest() {
        // the same sed the script uses, run on sample lines
        let sed = r#"/^GRUB_CMDLINE_LINUX_DEFAULT=/ { s/[[:space:]]+resume(_offset)?=[^[:space:]"']*//g ; s/(["'])resume(_offset)?=[^[:space:]"']*[[:space:]]*/\1/ ; s|(["'])$| resume=UUID=NEW resume_offset=7\1| }"#;
        let run = |line: &str| -> String {
            let mut c = std::process::Command::new("sed").args(["-E", sed]).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).spawn().unwrap();
            use std::io::Write;
            c.stdin.take().unwrap().write_all(line.as_bytes()).unwrap();
            String::from_utf8(c.wait_with_output().unwrap().stdout).unwrap()
        };
        assert_eq!(run("GRUB_CMDLINE_LINUX_DEFAULT='quiet splash loglevel=3'\n"), "GRUB_CMDLINE_LINUX_DEFAULT='quiet splash loglevel=3 resume=UUID=NEW resume_offset=7'\n");
        assert_eq!(run("GRUB_CMDLINE_LINUX_DEFAULT='quiet resume=UUID=OLD loglevel=3'\n"), "GRUB_CMDLINE_LINUX_DEFAULT='quiet loglevel=3 resume=UUID=NEW resume_offset=7'\n");
        assert_eq!(run("GRUB_CMDLINE_LINUX_DEFAULT=\"resume=UUID=OLD resume_offset=1 quiet\"\n"), "GRUB_CMDLINE_LINUX_DEFAULT=\"quiet resume=UUID=NEW resume_offset=7\"\n");
        assert_eq!(run("GRUB_TIMEOUT=5\n"), "GRUB_TIMEOUT=5\n");
    }
}
