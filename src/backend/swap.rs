//! Swap you can add after installing: compressed memory swap (zram) and a swap file.
//!
//! The installer only offers swap at install time, so a computer installed with "No swap" had no way to get any later
//! (and a 4 GB computer building a big program from the AUR ran out of memory). Everything that changes the system is
//! one shell script run with one administrator prompt (`pkexec sh -c`), built from fixed text and numbers only, never
//! from text the person typed.
//!
//! * zram: swap that lives compressed in RAM. Needs no disk space; the `zram-generator` package makes it at boot.
//! * Swap file: on Btrfs it must sit in its own subvolume (`@swap`, mounted at `/swap`) because Btrfs refuses to take a
//!   snapshot of a subvolume that holds a swap file in use, and Zohara snapshots `@` before every update. On other file
//!   systems it is a plain `/swapfile`.

use std::process::Command;

pub const ZRAM_CONF: &str = "/etc/systemd/zram-generator.conf.d/zohara.conf";
/// The swap file's two possible homes: Btrfs (own subvolume) and everything else.
pub const FILE_BTRFS: &str = "/swap/swapfile";
pub const FILE_PLAIN: &str = "/swapfile";
/// Disk space that must stay free after the swap file is made.
pub const KEEP_FREE_GIB: u64 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Partition,
    File,
    Zram,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub name: String,
    pub kind: Kind,
    pub size_kib: u64,
    pub used_kib: u64,
}

/// The swap in use, from the text of `/proc/swaps` (header line, then `name type size used priority`).
pub fn parse_swaps(text: &str) -> Vec<Device> {
    text.lines()
        .skip(1)
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let (name, ty, size, used) = (it.next()?, it.next()?, it.next()?, it.next()?);
            let kind = if name.contains("/zram") { Kind::Zram } else if ty == "file" { Kind::File } else { Kind::Partition };
            Some(Device { name: name.to_string(), kind, size_kib: size.parse().ok()?, used_kib: used.parse().ok()? })
        })
        .collect()
}

pub fn gib(kib: u64) -> String {
    let g = kib as f64 / 1024.0 / 1024.0;
    if g >= 10.0 { format!("{g:.0} GiB") } else { format!("{g:.1} GiB") }
}

/// One line per device for the page.
pub fn describe(d: &Device) -> String {
    let what = match d.kind {
        Kind::Zram => "Compressed memory (zram)",
        Kind::File => "Swap file",
        Kind::Partition => "Swap partition",
    };
    format!("{what}: {}, {} used", gib(d.size_kib), gib(d.used_kib))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub mem_kib: u64,
    pub devices: Vec<Device>,
    /// Zohara's zram setting is in place (it may not be running until the next start).
    pub zram_configured: bool,
    /// A swap file made here exists (in use or not).
    pub file_exists: bool,
    /// Free disk space on the root file system, in bytes.
    pub free_bytes: u64,
}

impl Status {
    pub fn swap_kib(&self) -> u64 {
        self.devices.iter().map(|d| d.size_kib).sum()
    }
    pub fn zram_active(&self) -> bool {
        self.devices.iter().any(|d| d.kind == Kind::Zram)
    }
    pub fn file_active(&self) -> Option<&Device> {
        self.devices.iter().find(|d| d.kind == Kind::File)
    }
    pub fn disk_swap_exists(&self) -> bool {
        self.devices.iter().any(|d| d.kind != Kind::Zram)
    }
}

fn mem_kib(meminfo: &str) -> u64 {
    meminfo.lines().find_map(|l| l.strip_prefix("MemTotal:")?.split_whitespace().next()?.parse().ok()).unwrap_or(0)
}

fn free_bytes_of_root() -> u64 {
    Command::new("df")
        .args(["-B1", "--output=avail", "/"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).lines().nth(1)?.trim().parse().ok())
        .unwrap_or(0)
}

pub fn status() -> Status {
    let read = |p: &str| std::fs::read_to_string(p).unwrap_or_default();
    let exists = |p: &str| std::path::Path::new(p).exists();
    Status {
        mem_kib: mem_kib(&read("/proc/meminfo")),
        devices: parse_swaps(&read("/proc/swaps")),
        zram_configured: exists(ZRAM_CONF),
        file_exists: exists(FILE_BTRFS) || exists(FILE_PLAIN),
        free_bytes: free_bytes_of_root(),
    }
}

/// Swap file sizes to offer, in GiB with a label. "As much as memory" is what Hibernate needs. Sizes the disk cannot hold
/// (keeping `KEEP_FREE_GIB` free) are left out; the file that already exists is not counted against the space.
pub fn sizes(mem_kib: u64, free_bytes: u64, existing_gib: u64) -> Vec<(String, u64)> {
    let ram_gib = mem_kib.div_ceil(1024 * 1024).max(1);
    let room = (free_bytes >> 30) + existing_gib;
    let mut out: Vec<(String, u64)> = Vec::new();
    for g in [1u64, 2, 4, 8, 16] {
        if g <= ram_gib * 2 && g + KEEP_FREE_GIB <= room && !out.iter().any(|o| o.1 == g) {
            out.push((format!("{g} GiB"), g));
        }
    }
    if ram_gib + KEEP_FREE_GIB <= room {
        out.retain(|o| o.1 != ram_gib);
        out.push((format!("{ram_gib} GiB, as much as memory (needed for Hibernate)"), ram_gib));
    }
    out.sort_by_key(|o| o.1);
    out
}

/// The size to suggest: half of memory, between 2 and 8 GiB, if it is on offer.
pub fn suggested(mem_kib: u64, options: &[(String, u64)]) -> Option<u64> {
    let want = (mem_kib / 1024 / 1024 / 2).clamp(2, 8);
    options.iter().map(|o| o.1).min_by_key(|g| g.abs_diff(want))
}

// ── Scripts ────────────────────────────────────────────────────────────────

/// Exit codes of the scripts that mean something to the person.
pub const EXIT_EXISTS: i32 = 3;
pub const EXIT_CANNOT_EMPTY: i32 = 4;
pub const EXIT_NO_ROOM: i32 = 5;

/// What an exit code of `pkexec sh -c <script>` means, in plain words; `None` when it was a success.
pub fn explain_exit(code: Option<i32>) -> Option<String> {
    match code {
        Some(0) => None,
        Some(126) | Some(127) => Some("The password prompt was closed, so nothing was changed.".into()),
        Some(EXIT_EXISTS) => Some("There is already a swap file. Remove it first, then make a new one.".into()),
        Some(EXIT_CANNOT_EMPTY) => Some("The swap file is in use and its contents don't fit in free memory right now. Close some apps and try again.".into()),
        Some(EXIT_NO_ROOM) => Some(format!("There isn't enough free disk space (at least {KEEP_FREE_GIB} GiB must stay free).")),
        _ => Some("It didn't work. Nothing is left half-done that would stop the computer from starting.".into()),
    }
}

/// Zswap (compressing in front of disk swap) is switched off while zram is on, and back on when zram goes away.
fn zswap_script(on: bool) -> String {
    let m = super::memory::Memory { zswap: on, ..super::memory::current() };
    super::memory::apply_script(m)
}

pub fn zram_enable_script() -> String {
    let mut s = String::from("set -e\npacman -S --needed --noconfirm zram-generator\n");
    s += &format!(
        "mkdir -p /etc/systemd/zram-generator.conf.d\nprintf '%s\\n' '# Written by Zohara Settings (Power & battery > Swap). Delete this file to turn it off.' '[zram0]' 'zram-size = min(ram / 2, 8192)' 'compression-algorithm = zstd' 'swap-priority = 100' > {ZRAM_CONF}\n"
    );
    s += &zswap_script(false);
    s += "systemctl daemon-reload\nsystemctl start systemd-zram-setup@zram0.service\n";
    s
}

pub fn zram_disable_script() -> String {
    let mut s = String::from("set -e\nsystemctl stop systemd-zram-setup@zram0.service 2>/dev/null || true\n");
    s += &format!("rm -f {ZRAM_CONF}\n");
    s += "systemctl daemon-reload\n";
    s += &zswap_script(true);
    s
}

/// Makes the swap file (`gib` GiB), mounts and registers it, and turns it on.
pub fn file_create_script(gib: u64) -> String {
    let gib = gib.clamp(1, 1024);
    format!(
        r#"set -e
SIZE={gib}
if [ -e {FILE_BTRFS} ] || [ -e {FILE_PLAIN} ]; then exit {EXIT_EXISTS}; fi
FS=$(findmnt -no FSTYPE /)
AVAIL=$(df -B1 --output=avail / | tail -1 | tr -d ' ')
if [ $(( AVAIL / 1073741824 )) -lt $(( SIZE + {KEEP_FREE_GIB} )) ]; then exit {EXIT_NO_ROOM}; fi
if [ "$FS" = btrfs ]; then
  DEV=$(findmnt -no SOURCE / | sed 's/\[.*//')
  UUID=$(blkid -s UUID -o value "$DEV")
  T=$(mktemp -d)
  mount -o subvolid=5 "$DEV" "$T"
  [ -e "$T/@swap" ] || btrfs subvolume create "$T/@swap"
  umount "$T"
  rmdir "$T"
  mkdir -p /swap
  grep -q '[[:space:]]/swap[[:space:]]' /etc/fstab || printf 'UUID=%s /swap btrfs subvol=/@swap,defaults,noatime,compress=zstd,discard=async 0 0\n' "$UUID" >> /etc/fstab
  mountpoint -q /swap || mount /swap
  btrfs filesystem mkswapfile --size "${{SIZE}}G" {FILE_BTRFS}
  F={FILE_BTRFS}
else
  mkswap -U clear --size "${{SIZE}}G" --file {FILE_PLAIN}
  F={FILE_PLAIN}
fi
grep -q "^$F[[:space:]]" /etc/fstab || printf '%s none swap defaults,pri=10 0 0\n' "$F" >> /etc/fstab
swapon -p 10 "$F"
"#
    )
}

/// Turns the swap file off, deletes it and takes it out of `/etc/fstab` (and removes the `@swap` subvolume).
pub fn file_remove_script() -> String {
    format!(
        r#"set -e
for F in {FILE_BTRFS} {FILE_PLAIN}; do
  if [ -e "$F" ]; then
    if grep -q "^$F " /proc/swaps; then swapoff "$F" || exit {EXIT_CANNOT_EMPTY}; fi
    rm -f "$F"
    sed -i "\#^$F[[:space:]]#d" /etc/fstab
  fi
done
if grep -q '[[:space:]]/swap[[:space:]]' /etc/fstab || mountpoint -q /swap || [ -d /swap ]; then
  mountpoint -q /swap && umount /swap
  FS=$(findmnt -no FSTYPE /)
  if [ "$FS" = btrfs ]; then
    DEV=$(findmnt -no SOURCE / | sed 's/\[.*//')
    T=$(mktemp -d)
    mount -o subvolid=5 "$DEV" "$T"
    [ -e "$T/@swap" ] && btrfs subvolume delete "$T/@swap" || true
    umount "$T"
    rmdir "$T"
  fi
  sed -i '\#[[:space:]]/swap[[:space:]]#d' /etc/fstab
  rmdir /swap 2>/dev/null || true
fi
"#
    )
}

/// Runs a script as administrator; `None` when it worked, otherwise what to tell the person.
pub fn run(script: &str) -> Option<String> {
    match Command::new("pkexec").args(["sh", "-c", script]).status() {
        Ok(s) => explain_exit(s.code()),
        Err(_) => Some("Couldn't ask for the password.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SWAPS: &str = "Filename\tType\tSize\tUsed\tPriority\n/dev/nvme0n1p3 partition\t9227260\t0\t-2\n/swap/swapfile  file\t4194300\t100\t10\n/dev/zram0 partition\t2097148\t5\t100\n";

    #[test]
    fn swap_in_use_is_read_and_told_apart() {
        let d = parse_swaps(SWAPS);
        assert_eq!(d.iter().map(|d| d.kind).collect::<Vec<_>>(), vec![Kind::Partition, Kind::File, Kind::Zram]);
        assert_eq!(d[1].size_kib, 4194300);
        assert_eq!(d[1].used_kib, 100);
        assert!(parse_swaps("Filename\tType\tSize\tUsed\tPriority\n").is_empty());
        assert!(parse_swaps("").is_empty());
    }

    #[test]
    fn the_page_text_is_plain() {
        let d = parse_swaps(SWAPS);
        assert_eq!(describe(&d[1]), "Swap file: 4.0 GiB, 0.0 GiB used");
        assert!(describe(&d[2]).starts_with("Compressed memory (zram)"));
    }

    #[test]
    fn sizes_fit_the_disk_and_the_memory() {
        let gb = |g: u64| g << 30;
        // 4 GiB memory, 100 GiB free: 1,2,4,8 and "as much as memory" (4) merged into one entry.
        let o = sizes(4 * 1024 * 1024, gb(100), 0);
        assert_eq!(o.iter().map(|o| o.1).collect::<Vec<_>>(), vec![1, 2, 4, 8]);
        assert!(o.iter().any(|o| o.1 == 4 && o.0.contains("Hibernate")));
        // Only 8 GiB free: 5 GiB must stay free, so 1, 2 and 3 fit but 4 does not... 1 and 2 only on offer.
        let o = sizes(4 * 1024 * 1024, gb(8), 0);
        assert_eq!(o.iter().map(|o| o.1).collect::<Vec<_>>(), vec![1, 2]);
        // No room at all.
        assert!(sizes(4 * 1024 * 1024, gb(3), 0).is_empty());
        // The existing file's own space is available again when it is replaced.
        assert!(sizes(4 * 1024 * 1024, gb(3), 4).iter().any(|o| o.1 == 2));
        // A big computer: memory-sized swap is offered, too.
        let o = sizes(32 * 1024 * 1024, gb(500), 0);
        assert!(o.iter().any(|o| o.1 == 32 && o.0.contains("Hibernate")));
    }

    #[test]
    fn a_sensible_size_is_suggested() {
        let o = sizes(16 * 1024 * 1024, 200 << 30, 0);
        assert_eq!(suggested(16 * 1024 * 1024, &o), Some(8));
        let o = sizes(4 * 1024 * 1024, 200 << 30, 0);
        assert_eq!(suggested(4 * 1024 * 1024, &o), Some(2));
        assert_eq!(suggested(4 * 1024 * 1024, &[]), None);
    }

    #[test]
    fn exit_codes_are_explained() {
        assert_eq!(explain_exit(Some(0)), None);
        assert!(explain_exit(Some(126)).unwrap().contains("password"));
        assert!(explain_exit(Some(EXIT_NO_ROOM)).unwrap().contains("disk space"));
        assert!(explain_exit(Some(EXIT_CANNOT_EMPTY)).unwrap().contains("memory"));
        assert!(explain_exit(None).is_some());
    }

    #[test]
    fn every_script_is_valid_shell() {
        let mut all = vec![zram_enable_script(), zram_disable_script(), file_remove_script()];
        for g in [0, 1, 2, 8, 4096] {
            all.push(file_create_script(g));
        }
        for s in all {
            let ok = Command::new("sh").args(["-n", "-c", &s]).status().unwrap().success();
            assert!(ok, "{s}");
        }
    }

    #[test]
    fn scripts_use_only_fixed_text_and_the_number() {
        let s = file_create_script(8);
        assert!(s.contains("SIZE=8\n"));
        assert!(s.contains("btrfs filesystem mkswapfile"));
        assert!(s.contains("subvol=/@swap"));
        assert!(file_create_script(99999).contains("SIZE=1024\n"), "kept in range");
        assert!(zram_enable_script().contains("pacman -S --needed --noconfirm zram-generator"));
        assert!(zram_enable_script().contains("zswap") || zram_enable_script().contains("vm.swappiness"));
    }

    #[test]
    fn the_live_system_status_is_readable() {
        let s = status();
        assert!(s.mem_kib > 0);
    }

    /// Writes the scripts to a folder (`ZOHARA_SWAP_DUMP`) so they can be run against a throwaway disk image.
    #[test]
    #[ignore = "writes files for a manual test"]
    fn dump_the_scripts() {
        let dir = std::env::var("ZOHARA_SWAP_DUMP").expect("set ZOHARA_SWAP_DUMP");
        for (name, text) in [("zram-enable.sh", zram_enable_script()), ("zram-disable.sh", zram_disable_script()), ("file-create-1.sh", file_create_script(1)), ("file-create-2.sh", file_create_script(2)), ("file-remove.sh", file_remove_script())] {
            std::fs::write(format!("{dir}/{name}"), text).unwrap();
        }
    }
}
