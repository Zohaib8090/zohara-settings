//! USB sticks, SD cards and external disks: what is plugged in, what may be changed, and the changes themselves
//! (format, erase the whole drive, rename, check and repair, safely remove, write a disk image to a stick).
//!
//! Safety first, because most of this destroys data:
//! * Only drives that are plugged in from outside (USB, SD, hot-plug) can be changed. Internal disks are listed as locked.
//! * A disk that holds anything the running system uses (`/`, `/boot`, `/home`, swap, the live image in `/run/archiso`...) is
//!   locked for good, in every mode.
//! * Before every destructive step the drive is looked up again and must still be the very same one the person confirmed
//!   (same path, size, model and serial number). If someone swaps drives while a dialog is open, nothing happens.
//!
//! The logic (reading `lsblk`, deciding what is allowed, label rules, which partition table to use, copying an image) is plain
//! code with unit tests. The changes go to UDisks2 over D-Bus, the same service the desktop uses, which asks PolicyKit for
//! permission when the change needs it, so Settings never handles a password. Live tests use loop devices (a file made to
//! look like a drive) and never touch real disks.

use serde_json::Value;
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

pub const LSBLK_COLUMNS: &str = "NAME,PATH,TYPE,SIZE,FSTYPE,LABEL,UUID,MOUNTPOINTS,RM,HOTPLUG,RO,ROTA,TRAN,MODEL,VENDOR,SERIAL,WWN,PTTYPE,FSSIZE,FSUSED";

// ── What a drive is ──────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A USB flash drive (a removable device behind USB).
    UsbStick,
    /// An external hard disk or SSD (fixed media behind USB).
    ExternalDisk,
    /// An SD or microSD card.
    SdCard,
    /// A disk inside the computer.
    Internal,
    /// A disk image attached as a drive (loop device). Only shown when asked for, for testing.
    Image,
}

impl Kind {
    pub fn title(self) -> &'static str {
        match self {
            Kind::UsbStick => "USB drive",
            Kind::ExternalDisk => "External disk",
            Kind::SdCard => "SD card",
            Kind::Internal => "Internal disk",
            Kind::Image => "Disk image (test)",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Kind::UsbStick => "drive-removable-media-usb-symbolic",
            Kind::ExternalDisk => "drive-harddisk-usb-symbolic",
            Kind::SdCard => "media-flash-sd-mmc-symbolic",
            Kind::Internal => "drive-harddisk-symbolic",
            Kind::Image => "media-optical-symbolic",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub path: String,
    pub label: String,
    pub fstype: String,
    pub size: u64,
    pub used: Option<u64>,
    pub mountpoints: Vec<String>,
}

impl Part {
    pub fn mounted(&self) -> bool {
        !self.mountpoints.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Drive {
    pub path: String,
    pub model: String,
    pub vendor: String,
    pub serial: String,
    pub wwn: String,
    pub size: u64,
    pub tran: String,
    pub removable: bool,
    pub hotplug: bool,
    pub read_only: bool,
    pub rotational: bool,
    pub pttype: String,
    pub kind: Kind,
    /// Something on this disk is used by the running system (or the live image): it can never be changed from here.
    pub system: bool,
    /// What lives on the disk itself when it has no partition table (a "superfloppy"): a filesystem written straight onto it.
    pub whole_fs: Option<Part>,
    pub parts: Vec<Part>,
}

impl Drive {
    /// The name shown to people: the model, or the kind when the drive does not say.
    pub fn title(&self) -> String {
        let model = self.model.trim();
        let vendor = self.vendor.trim();
        match (vendor.is_empty(), model.is_empty()) {
            (_, true) if vendor.is_empty() => self.kind.title().to_string(),
            (_, true) => vendor.to_string(),
            (true, false) => model.to_string(),
            (false, false) if model.to_lowercase().starts_with(&vendor.to_lowercase()) => model.to_string(),
            (false, false) => format!("{vendor} {model}"),
        }
    }

    /// Anything we may change: plugged in from outside, not used by the system, not write-protected.
    pub fn can_modify(&self) -> bool {
        !self.system && !self.read_only && matches!(self.kind, Kind::UsbStick | Kind::ExternalDisk | Kind::SdCard | Kind::Image)
    }

    /// Why it cannot be changed, in words for the person; `None` when it can.
    pub fn locked_reason(&self) -> Option<&'static str> {
        if self.system {
            Some("Zohara is running from this disk, so it can't be changed here.")
        } else if self.kind == Kind::Internal {
            Some("This disk is inside the computer. Settings only changes USB drives, SD cards and external disks.")
        } else if self.read_only {
            Some("This drive is write-protected. Slide the lock switch on the drive, or plug it in again.")
        } else {
            None
        }
    }

    /// Every filesystem on the drive, partitions or the whole disk.
    pub fn filesystems(&self) -> Vec<&Part> {
        match &self.whole_fs {
            Some(p) => vec![p],
            None => self.parts.iter().collect(),
        }
    }

    pub fn mounted_paths(&self) -> Vec<String> {
        self.filesystems().into_iter().filter(|p| p.mounted()).map(|p| p.path.clone()).collect()
    }

    /// The facts that must not change between "the person confirmed" and "we start".
    pub fn identity(&self) -> Identity {
        Identity {
            path: self.path.clone(),
            size: self.size,
            model: self.model.trim().to_string(),
            serial: self.serial.trim().to_string(),
            wwn: self.wwn.trim().to_string(),
        }
    }

    /// A short line like "29.8 GB · USB drive".
    pub fn summary(&self) -> String {
        format!("{} · {}", human(self.size), self.kind.title())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub path: String,
    pub size: u64,
    pub model: String,
    pub serial: String,
    pub wwn: String,
}

pub fn human(bytes: u64) -> String {
    let b = bytes as f64;
    match bytes {
        0..=1023 => format!("{bytes} B"),
        1024..=1_048_575 => format!("{:.0} KB", b / 1024.0),
        1_048_576..=1_073_741_823 => format!("{:.0} MB", b / 1_048_576.0),
        _ => format!("{:.1} GB", b / 1_073_741_824.0),
    }
}

// ── Reading lsblk ────────────────────────────────────────────────────────────

fn s(v: &Value) -> String {
    v.as_str().unwrap_or_default().trim().to_string()
}

fn n(v: &Value) -> u64 {
    v.as_u64().or_else(|| v.as_str().and_then(|x| x.parse().ok())).unwrap_or(0)
}

fn flag(v: &Value) -> bool {
    v.as_bool().unwrap_or_else(|| v.as_str() == Some("1") || v.as_u64() == Some(1))
}

fn mountpoints(v: &Value) -> Vec<String> {
    let mut out: Vec<String> = v["mountpoints"]
        .as_array()
        .map(|a| a.iter().filter_map(|m| m.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    if out.is_empty() {
        if let Some(m) = v["mountpoint"].as_str() {
            out.push(m.to_string());
        }
    }
    out
}

/// Where the running system keeps things. A disk with a partition mounted at or under one of these is the system's.
fn is_system_mount(m: &str) -> bool {
    m == "/"
        || m == "[SWAP]"
        || ["/boot", "/efi", "/home", "/var", "/usr", "/etc", "/opt", "/srv", "/nix", "/run/archiso", "/run/initramfs", "/mnt/zohara"]
            .iter()
            .any(|p| m == *p || m.starts_with(&format!("{p}/")))
}

/// All mount points of a block device and everything stacked on it (partitions, encrypted or LVM volumes).
fn all_mounts(v: &Value) -> Vec<String> {
    let mut out = mountpoints(v);
    for c in v["children"].as_array().into_iter().flatten() {
        out.extend(all_mounts(c));
    }
    out
}

fn part_from(v: &Value) -> Part {
    let size = n(&v["size"]);
    let used = if v["fsused"].is_null() { None } else { Some(n(&v["fsused"])) };
    Part { path: s(&v["path"]), label: s(&v["label"]), fstype: s(&v["fstype"]), size, used, mountpoints: mountpoints(v) }
}

fn classify(v: &Value, include_images: bool) -> Option<Kind> {
    let name = s(&v["name"]);
    let tran = s(&v["tran"]);
    let removable = flag(&v["rm"]);
    let hotplug = flag(&v["hotplug"]);
    match s(&v["type"]).as_str() {
        "disk" => {
            if name.starts_with("mmcblk") && (removable || hotplug) {
                Some(Kind::SdCard)
            } else if tran == "usb" || removable {
                // A flash stick reports itself as removable; an external hard disk or SSD does not.
                // "Hot-pluggable" alone is NOT enough: some computers report an internal SATA disk that way, and
                // an internal disk must never be offered for erasing.
                if removable { Some(Kind::UsbStick) } else { Some(Kind::ExternalDisk) }
            } else {
                Some(Kind::Internal)
            }
        }
        "loop" if include_images => Some(Kind::Image),
        _ => None,
    }
}

/// Turns `lsblk -J` output into drives. `include_images` also lists loop devices (used by the tests).
pub fn parse_lsblk(json: &str, include_images: bool) -> Vec<Drive> {
    let Ok(root) = serde_json::from_str::<Value>(json) else { return Vec::new() };
    let mut drives = Vec::new();
    for d in root["blockdevices"].as_array().into_iter().flatten() {
        let Some(kind) = classify(d, include_images) else { continue };
        // Optical drives and empty card readers show up as disks of size 0.
        let size = n(&d["size"]);
        if size == 0 {
            continue;
        }
        let children: Vec<&Value> = d["children"].as_array().map(|a| a.iter().collect()).unwrap_or_default();
        let parts: Vec<Part> = children
            .iter()
            .filter(|c| s(&c["type"]) == "part" || !c["fstype"].is_null())
            .filter(|c| !c["fstype"].is_null() && s(&c["fstype"]) != "swap")
            .map(|c| part_from(c))
            .collect();
        let whole_fs = if children.is_empty() && !d["fstype"].is_null() { Some(part_from(d)) } else { None };
        let system = all_mounts(d).iter().any(|m| is_system_mount(m));
        drives.push(Drive {
            path: s(&d["path"]),
            model: s(&d["model"]),
            vendor: s(&d["vendor"]),
            serial: s(&d["serial"]),
            wwn: s(&d["wwn"]),
            size,
            tran: s(&d["tran"]),
            removable: flag(&d["rm"]),
            hotplug: flag(&d["hotplug"]),
            read_only: flag(&d["ro"]),
            rotational: flag(&d["rota"]),
            pttype: s(&d["pttype"]),
            kind,
            system,
            whole_fs,
            parts,
        });
    }
    drives
}

/// Reads the drives from the system. Loop devices are listed only when `ZOHARA_DRIVES_SHOW_IMAGES=1` (for testing).
pub fn list() -> Vec<Drive> {
    let include_images = std::env::var("ZOHARA_DRIVES_SHOW_IMAGES").map(|v| v == "1").unwrap_or(false);
    let out = Command::new("lsblk").args(["-J", "-b", "-o", LSBLK_COLUMNS]).output();
    match out {
        Ok(o) => parse_lsblk(&String::from_utf8_lossy(&o.stdout), include_images),
        Err(_) => Vec::new(),
    }
}

pub fn find(path: &str) -> Option<Drive> {
    list().into_iter().find(|d| d.path == path)
}

/// The change must still be about the drive the person confirmed: the same one is there, and it is still allowed.
pub fn still_the_same(before: &Identity, now: &[Drive]) -> Result<Drive, String> {
    let Some(d) = now.iter().find(|d| d.path == before.path) else {
        return Err("The drive was removed. Nothing was changed.".into());
    };
    if d.identity() != *before {
        return Err("A different drive is now at that place. Nothing was changed. Plug the drive in again and start over.".into());
    }
    if !d.can_modify() {
        return Err(d.locked_reason().unwrap_or("This drive can't be changed.").to_string());
    }
    Ok(d.clone())
}

// ── Filesystems ──────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fs {
    /// The name UDisks and mkfs know.
    pub id: &'static str,
    pub title: &'static str,
    pub note: &'static str,
    pub max_label: usize,
    /// The formatting tool UDisks runs.
    pub tool: &'static str,
    /// The Windows-compatible filesystems get an MBR table on small drives (old devices read those).
    pub windows_style: bool,
}

pub const FILESYSTEMS: [Fs; 5] = [
    Fs { id: "exfat", title: "exFAT", note: "Windows, macOS and Linux. The usual choice for USB drives and SD cards.", max_label: 11, tool: "mkfs.exfat", windows_style: true },
    Fs { id: "vfat", title: "FAT32", note: "Works almost everywhere, even TVs and car stereos. A single file can't be over 4 GB.", max_label: 11, tool: "mkfs.vfat", windows_style: true },
    Fs { id: "ntfs", title: "NTFS", note: "Windows. Linux can read and write it too.", max_label: 32, tool: "mkfs.ntfs", windows_style: true },
    Fs { id: "ext4", title: "ext4", note: "Linux only. Fast and reliable for a Linux drive.", max_label: 16, tool: "mkfs.ext4", windows_style: false },
    Fs { id: "btrfs", title: "Btrfs", note: "Linux only. Snapshots and checksums.", max_label: 255, tool: "mkfs.btrfs", windows_style: false },
];

pub fn fs_by_id(id: &str) -> Option<&'static Fs> {
    FILESYSTEMS.iter().find(|f| f.id == id)
}

/// The filesystems whose formatting tool is installed.
pub fn available_filesystems() -> Vec<&'static Fs> {
    FILESYSTEMS.iter().filter(|f| tool_installed(f.tool)).collect()
}

fn tool_installed(tool: &str) -> bool {
    ["/usr/bin", "/usr/sbin", "/bin", "/sbin", "/usr/local/bin"].iter().any(|d| Path::new(d).join(tool).exists())
}

/// Checks and tidies a drive name for a filesystem. FAT drive names are upper case and have no `" * / : < > ? \ | + , ; = [ ]`.
pub fn normalize_label(fs: &str, input: &str) -> Result<String, String> {
    let f = fs_by_id(fs).ok_or_else(|| "That filesystem isn't supported.".to_string())?;
    let label = input.trim().to_string();
    if label.chars().any(|c| c.is_control()) {
        return Err("The name can't contain control characters.".into());
    }
    if f.id == "vfat" {
        if label.chars().any(|c| "\"*/:<>?\\|+,;=[]".contains(c)) {
            return Err("A FAT32 name can't contain \" * / : < > ? \\ | + , ; = [ ]".into());
        }
    } else if label.contains('/') {
        return Err("The name can't contain a slash.".into());
    }
    let label = if f.id == "vfat" { label.to_uppercase() } else { label };
    let len = if f.id == "ext4" || f.id == "btrfs" { label.len() } else { label.chars().count() };
    if len > f.max_label {
        return Err(format!("The name can have at most {} characters on {}.", f.max_label, f.title));
    }
    Ok(label)
}

/// The partition table for a whole-drive erase: the old style (MBR) for small drives with a Windows-style filesystem, because
/// old TVs, cameras and car radios only read those; GPT for everything else and for drives over 2 TB.
pub fn table_for(fs: &str, drive_size: u64) -> &'static str {
    const TWO_TB: u64 = 2 * 1024 * 1024 * 1024 * 1024;
    let windows = fs_by_id(fs).map(|f| f.windows_style).unwrap_or(false);
    if windows && drive_size < TWO_TB { "dos" } else { "gpt" }
}

/// The partition type code UDisks wants for the table style: a hex byte for MBR, a GUID for GPT.
pub fn partition_type(table: &str, fs: &str) -> &'static str {
    let windows = fs_by_id(fs).map(|f| f.windows_style).unwrap_or(false);
    match (table, windows, fs) {
        ("dos", true, "vfat") => "0x0c",
        ("dos", true, _) => "0x07",
        ("dos", false, _) => "0x83",
        (_, true, _) => "ebd0a0a2-b9e5-4433-87c0-68b6b72699c7",
        _ => "0fc63daf-8483-4772-8e79-3d69d8477de4",
    }
}

/// What gets lost: a short list for the confirmation ("Photos (exFAT, 12 GB used)").
pub fn what_will_be_lost(d: &Drive) -> Vec<String> {
    let fss = d.filesystems();
    if fss.is_empty() {
        return vec!["Nothing recognisable is on it".into()];
    }
    fss.iter()
        .map(|p| {
            let name = if p.label.is_empty() { "Unnamed".to_string() } else { p.label.clone() };
            let used = p.used.map(|u| format!(", {} used", human(u))).unwrap_or_default();
            let fs = fs_by_id(&p.fstype).map(|f| f.title.to_string()).unwrap_or_else(|| if p.fstype.is_empty() { "unknown".into() } else { p.fstype.clone() });
            format!("{name} ({fs}{used})")
        })
        .collect()
}

// ── Messages ─────────────────────────────────────────────────────────────────

/// UDisks errors in words a person can use.
pub fn friendly_error(raw: &str) -> String {
    let r = raw.to_lowercase();
    if r.contains("notauthorized") || r.contains("not authorized") || r.contains("dismissed") {
        "The change wasn't allowed. If a password window appeared, it was closed or the password was wrong.".into()
    } else if r.contains("busy") || r.contains("is mounted") || r.contains("in use") {
        "Something is still using this drive. Close any files or windows from it and try again.".into()
    } else if r.contains("read-only") || r.contains("write-protected") || r.contains("read only") {
        "The drive is write-protected.".into()
    } else if r.contains("no such file") || r.contains("unknownobject") || r.contains("unknown object") || r.contains("no such device") {
        "The drive isn't there any more.".into()
    } else if r.contains("timed out") || r.contains("timeout") {
        "The drive didn't answer in time. Plug it in again and try once more.".into()
    } else {
        // The tools' own message is the most useful thing left; keep it short.
        let t = raw.trim();
        let t = t.rsplit(": ").next().unwrap_or(t).trim();
        if t.is_empty() { "The change failed.".into() } else { format!("The change failed: {t}") }
    }
}

// ── Disk images ──────────────────────────────────────────────────────────────

/// Is this file something that can be written to a stick: a regular file, not empty. Returns its size.
pub fn check_image_file(path: &Path) -> Result<u64, String> {
    let meta = std::fs::metadata(path).map_err(|_| "That file can't be opened.".to_string())?;
    if !meta.is_file() {
        return Err("That isn't a file.".into());
    }
    if meta.len() == 0 {
        return Err("That file is empty.".into());
    }
    Ok(meta.len())
}

/// Does the file look like a bootable ISO (the `CD001` mark of an ISO 9660 image)? Plain disk images (`.img`) don't have it.
pub fn looks_like_iso(path: &Path) -> bool {
    let Ok(mut f) = std::fs::File::open(path) else { return false };
    let mut mark = [0u8; 5];
    f.seek(SeekFrom::Start(0x8001)).is_ok() && f.read_exact(&mut mark).is_ok() && &mark == b"CD001"
}

/// The image has to fit, and must not be stored on the drive that is about to be erased.
pub fn check_image_fits(image: &Path, image_size: u64, drive: &Drive) -> Result<(), String> {
    if image_size > drive.size {
        return Err(format!("The image is {} but the drive holds only {}.", human(image_size), human(drive.size)));
    }
    let image = std::fs::canonicalize(image).unwrap_or_else(|_| image.to_path_buf());
    for p in drive.filesystems() {
        for m in &p.mountpoints {
            if image.starts_with(m) {
                return Err("The image file is stored on this drive. Choose another drive, or copy the file to your computer first.".into());
            }
        }
    }
    Ok(())
}

/// Progress of a long job, shared with the window.
#[derive(Default)]
pub struct Progress {
    pub phase: AtomicU64, // 0 preparing, 1 writing, 2 checking, 3 done
    pub done: AtomicU64,
    pub total: AtomicU64,
    pub cancel: AtomicBool,
}

impl Progress {
    pub fn new() -> Arc<Progress> {
        Arc::new(Progress::default())
    }
}

pub const CHUNK: usize = 4 * 1024 * 1024;

/// Copies `src` to `dst` in chunks (the heart of "make a bootable USB drive"), then flushes it to the drive.
pub fn copy_image(src: &mut impl Read, dst: &mut (impl Write + SyncAll), total: u64, progress: &Progress) -> Result<(), String> {
    progress.phase.store(1, Ordering::SeqCst);
    progress.total.store(total, Ordering::SeqCst);
    progress.done.store(0, Ordering::SeqCst);
    let mut buf = vec![0u8; CHUNK];
    let mut done = 0u64;
    while done < total {
        if progress.cancel.load(Ordering::SeqCst) {
            return Err("Cancelled. The drive now holds a partly written image and needs to be formatted again.".into());
        }
        let want = ((total - done) as usize).min(CHUNK);
        src.read_exact(&mut buf[..want]).map_err(|e| format!("Couldn't read the image: {e}"))?;
        dst.write_all(&buf[..want]).map_err(|e| format!("Couldn't write to the drive: {e}"))?;
        done += want as u64;
        progress.done.store(done, Ordering::SeqCst);
    }
    dst.sync_all_data().map_err(|e| format!("Couldn't finish writing to the drive: {e}"))?;
    Ok(())
}

pub trait SyncAll {
    fn sync_all_data(&mut self) -> std::io::Result<()>;
}

impl SyncAll for std::fs::File {
    fn sync_all_data(&mut self) -> std::io::Result<()> {
        self.sync_all()
    }
}

impl SyncAll for Vec<u8> {
    fn sync_all_data(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Reads both back and compares them, the first difference ends it.
pub fn verify_image(image: &mut impl Read, drive: &mut impl Read, total: u64, progress: &Progress) -> Result<(), String> {
    progress.phase.store(2, Ordering::SeqCst);
    progress.total.store(total, Ordering::SeqCst);
    progress.done.store(0, Ordering::SeqCst);
    let mut a = vec![0u8; CHUNK];
    let mut b = vec![0u8; CHUNK];
    let mut done = 0u64;
    while done < total {
        if progress.cancel.load(Ordering::SeqCst) {
            return Err("Cancelled while checking.".into());
        }
        let want = ((total - done) as usize).min(CHUNK);
        image.read_exact(&mut a[..want]).map_err(|e| format!("Couldn't read the image: {e}"))?;
        drive.read_exact(&mut b[..want]).map_err(|e| format!("Couldn't read the drive back: {e}"))?;
        if a[..want] != b[..want] {
            return Err(format!("The drive does not match the image (first difference near {}). Try again, or use another drive.", human(done)));
        }
        done += want as u64;
        progress.done.store(done, Ordering::SeqCst);
    }
    Ok(())
}

// ── Talking to UDisks2 ───────────────────────────────────────────────────────

const UDISKS: &str = "org.freedesktop.UDisks2";
const BLOCKS: &str = "/org/freedesktop/UDisks2/block_devices/";

/// `/dev/sdb1` → the D-Bus path of its block device. Only plain device names are accepted.
pub fn block_object_path(dev: &str) -> Result<String, String> {
    let name = dev.strip_prefix("/dev/").ok_or_else(|| "That isn't a device.".to_string())?;
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err("That device name isn't supported.".into());
    }
    Ok(format!("{BLOCKS}{name}"))
}

/// Runs async D-Bus work to the end on a small private runtime (these calls come from worker threads).
fn run<T>(fut: impl std::future::Future<Output = Result<T, String>>) -> Result<T, String> {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    rt.block_on(async {
        match tokio::time::timeout(std::time::Duration::from_secs(60 * 60 * 6), fut).await {
            Ok(r) => r,
            Err(_) => Err("timed out".into()),
        }
    })
}

type Opts<'a> = HashMap<&'a str, zbus::zvariant::Value<'a>>;

/// A UDisks2 session. `interactive` lets PolicyKit show a password window when a change needs one (the app does; tests don't).
#[derive(Clone, Copy)]
pub struct Udisks {
    pub interactive: bool,
}

impl Udisks {
    pub fn new() -> Udisks {
        Udisks { interactive: true }
    }

    fn opts<'a>(&self) -> Opts<'a> {
        let mut o: Opts = HashMap::new();
        if !self.interactive {
            o.insert("auth.no_user_interaction", true.into());
        }
        o
    }

    async fn call<B: serde::ser::Serialize + zbus::zvariant::DynamicType>(
        &self,
        conn: &zbus::Connection,
        path: &str,
        iface: &str,
        method: &str,
        body: &B,
    ) -> Result<zbus::Message, String> {
        conn.call_method(Some(UDISKS), path, Some(iface), method, body).await.map_err(|e| friendly_error(&e.to_string()))
    }

    /// Writes a new filesystem (`exfat`, `vfat`, ...) or partition table (`dos`, `gpt`, `empty`) onto a device.
    pub fn format(&self, dev: &str, kind: &str, label: &str, zero_fill: bool) -> Result<(), String> {
        let path = block_object_path(dev)?;
        let this = *self;
        let (kind, label) = (kind.to_string(), label.to_string());
        run(async move {
            let conn = zbus::Connection::system().await.map_err(|e| e.to_string())?;
            let mut o = this.opts();
            if !label.is_empty() {
                o.insert("label", label.as_str().into());
            }
            if zero_fill {
                o.insert("erase", "zero".into());
            }
            this.call(&conn, &path, "org.freedesktop.UDisks2.Block", "Format", &(kind.as_str(), o)).await.map(|_| ())
        })
    }

    /// Makes one partition from 1 MiB to the end of a drive that already has a table, and formats it.
    pub fn create_partition_and_format(&self, dev: &str, ptype: &str, fs: &str, label: &str) -> Result<String, String> {
        let path = block_object_path(dev)?;
        let this = *self;
        let (ptype, fs, label) = (ptype.to_string(), fs.to_string(), label.to_string());
        run(async move {
            let conn = zbus::Connection::system().await.map_err(|e| e.to_string())?;
            let mut fo = this.opts();
            if !label.is_empty() {
                fo.insert("label", label.as_str().into());
            }
            let msg = this
                .call(
                    &conn,
                    &path,
                    "org.freedesktop.UDisks2.PartitionTable",
                    "CreatePartitionAndFormat",
                    &(1u64 << 20, 0u64, ptype.as_str(), "", this.opts(), fs.as_str(), fo),
                )
                .await?;
            let p: zbus::zvariant::OwnedObjectPath = msg.body().deserialize().map_err(|e| e.to_string())?;
            Ok(p.to_string())
        })
    }

    pub fn set_label(&self, dev: &str, label: &str) -> Result<(), String> {
        let path = block_object_path(dev)?;
        let this = *self;
        let label = label.to_string();
        run(async move {
            let conn = zbus::Connection::system().await.map_err(|e| e.to_string())?;
            this.call(&conn, &path, "org.freedesktop.UDisks2.Filesystem", "SetLabel", &(label.as_str(), this.opts())).await.map(|_| ())
        })
    }

    /// `true`: no problems found (or repaired).
    pub fn check(&self, dev: &str, repair: bool) -> Result<bool, String> {
        let path = block_object_path(dev)?;
        let this = *self;
        run(async move {
            let conn = zbus::Connection::system().await.map_err(|e| e.to_string())?;
            let m = if repair { "Repair" } else { "Check" };
            let msg = this.call(&conn, &path, "org.freedesktop.UDisks2.Filesystem", m, &(this.opts(),)).await?;
            msg.body().deserialize::<bool>().map_err(|e| e.to_string())
        })
    }

    pub fn mount(&self, dev: &str) -> Result<String, String> {
        let path = block_object_path(dev)?;
        let this = *self;
        run(async move {
            let conn = zbus::Connection::system().await.map_err(|e| e.to_string())?;
            let msg = this.call(&conn, &path, "org.freedesktop.UDisks2.Filesystem", "Mount", &(this.opts(),)).await?;
            msg.body().deserialize::<String>().map_err(|e| e.to_string())
        })
    }

    /// Unmounts; a device that is not mounted counts as done.
    pub fn unmount(&self, dev: &str) -> Result<(), String> {
        let path = block_object_path(dev)?;
        let this = *self;
        run(async move {
            let conn = zbus::Connection::system().await.map_err(|e| e.to_string())?;
            let r = conn.call_method(Some(UDISKS), path.as_str(), Some("org.freedesktop.UDisks2.Filesystem"), "Unmount", &(this.opts(),)).await;
            match r {
                Ok(_) => Ok(()),
                Err(e) if e.to_string().to_lowercase().contains("not mounted") => Ok(()),
                Err(e) => Err(friendly_error(&e.to_string())),
            }
        })
    }

    pub fn unmount_all(&self, drive: &Drive) -> Result<(), String> {
        for p in drive.mounted_paths() {
            self.unmount(&p)?;
        }
        Ok(())
    }

    /// Unmounts everything on the drive and, when the hardware can, powers it off so it can be pulled out.
    pub fn safely_remove(&self, drive: &Drive) -> Result<(), String> {
        self.unmount_all(drive)?;
        let path = block_object_path(&drive.path)?;
        let this = *self;
        run(async move {
            let conn = zbus::Connection::system().await.map_err(|e| e.to_string())?;
            let msg = this.call(&conn, &path, "org.freedesktop.DBus.Properties", "Get", &("org.freedesktop.UDisks2.Block", "Drive")).await?;
            let v: zbus::zvariant::OwnedValue = msg.body().deserialize().map_err(|e| e.to_string())?;
            let drive_path: zbus::zvariant::OwnedObjectPath = v.try_into().map_err(|_| "no drive".to_string())?;
            if drive_path.as_str() == "/" {
                return Ok(()); // a loop device or similar: nothing to power off
            }
            // Eject first (media cards), power off after (USB devices). Either may be refused for this hardware.
            let _ = conn.call_method(Some(UDISKS), drive_path.as_str(), Some("org.freedesktop.UDisks2.Drive"), "Eject", &(this.opts(),)).await;
            let r = conn.call_method(Some(UDISKS), drive_path.as_str(), Some("org.freedesktop.UDisks2.Drive"), "PowerOff", &(this.opts(),)).await;
            match r {
                Ok(_) => Ok(()),
                Err(e) if e.to_string().to_lowercase().contains("not supported") => Ok(()),
                Err(e) => Err(friendly_error(&e.to_string())),
            }
        })
    }

    /// Opens the whole drive for writing (`write`) or reading and hands back the file. No root needed: UDisks passes the file over.
    pub fn open_device(&self, dev: &str, write: bool) -> Result<std::fs::File, String> {
        let path = block_object_path(dev)?;
        let this = *self;
        run(async move {
            let conn = zbus::Connection::system().await.map_err(|e| e.to_string())?;
            let mut o = this.opts();
            // O_EXCL: refuse when something else (a mounted filesystem) holds the device. O_SYNC is left off; we flush at the end.
            o.insert("flags", (if write { 0o200i32 } else { 0i32 }).into());
            let mode = if write { "w" } else { "r" };
            let msg = this.call(&conn, &path, "org.freedesktop.UDisks2.Block", "OpenDevice", &(mode, o)).await?;
            let fd: zbus::zvariant::OwnedFd = msg.body().deserialize().map_err(|e| e.to_string())?;
            Ok(std::fs::File::from(std::os::fd::OwnedFd::from(fd)))
        })
    }

    /// Asks the system to look at the partitions again after the whole drive was overwritten.
    pub fn rescan(&self, dev: &str) {
        if let Ok(path) = block_object_path(dev) {
            let this = *self;
            let _ = run(async move {
                let conn = zbus::Connection::system().await.map_err(|e| e.to_string())?;
                let _ = this.call(&conn, &path, "org.freedesktop.UDisks2.Block", "Rescan", &(this.opts(),)).await;
                Ok(())
            });
        }
    }
}

// ── The jobs the window starts ───────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct FormatPlan {
    pub fs: String,
    pub label: String,
    /// Overwrite with zeros first (slow, makes the old data unrecoverable) instead of the quick format.
    pub zero_fill: bool,
}

/// Formats one partition (or the whole disk when it has no partitions) of a confirmed drive.
pub fn format_partition(ud: Udisks, confirmed: &Identity, target: &str, plan: &FormatPlan) -> Result<(), String> {
    let drive = still_the_same(confirmed, &list())?;
    if !drive.filesystems().iter().any(|p| p.path == target) && drive.path != target {
        return Err("That part of the drive isn't there any more.".into());
    }
    let label = normalize_label(&plan.fs, &plan.label)?;
    fs_by_id(&plan.fs).filter(|f| tool_installed(f.tool)).ok_or_else(|| "The tool for that filesystem isn't installed.".to_string())?;
    ud.unmount_all(&drive)?;
    ud.format(target, &plan.fs, &label, plan.zero_fill)
}

/// Erases the whole drive: a new partition table, one partition, formatted.
pub fn erase_drive(ud: Udisks, confirmed: &Identity, plan: &FormatPlan) -> Result<(), String> {
    let drive = still_the_same(confirmed, &list())?;
    let label = normalize_label(&plan.fs, &plan.label)?;
    fs_by_id(&plan.fs).filter(|f| tool_installed(f.tool)).ok_or_else(|| "The tool for that filesystem isn't installed.".to_string())?;
    ud.unmount_all(&drive)?;
    if plan.zero_fill {
        // Overwrite everything first, then start from an empty drive.
        ud.format(&drive.path, "empty", "", true)?;
    }
    let table = table_for(&plan.fs, drive.size);
    ud.format(&drive.path, table, "", false)?;
    std::thread::sleep(std::time::Duration::from_millis(600));
    ud.create_partition_and_format(&drive.path, partition_type(table, &plan.fs), &plan.fs, &label).map(|_| ())
}

/// Writes a disk image (an ISO, an `.img`) to the whole drive and checks it.
pub fn write_image(ud: Udisks, confirmed: &Identity, image: &Path, verify: bool, progress: &Progress) -> Result<(), String> {
    let drive = still_the_same(confirmed, &list())?;
    let size = check_image_file(image)?;
    check_image_fits(image, size, &drive)?;
    progress.phase.store(0, Ordering::SeqCst);
    ud.unmount_all(&drive)?;
    let mut src = std::fs::File::open(image).map_err(|_| "The image can't be opened.".to_string())?;
    {
        // The drive may only be touched through the file UDisks gave for exactly the confirmed device.
        let still = still_the_same(confirmed, &list())?;
        debug_assert_eq!(still.path, drive.path);
        let mut dst = ud.open_device(&drive.path, true)?;
        copy_image(&mut src, &mut dst, size, progress)?;
    }
    ud.rescan(&drive.path);
    if verify {
        let mut again = std::fs::File::open(image).map_err(|_| "The image can't be opened.".to_string())?;
        let mut back = ud.open_device(&drive.path, false)?;
        verify_image(&mut again, &mut back, size, progress)?;
    }
    progress.phase.store(3, Ordering::SeqCst);
    Ok(())
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> String {
        r#"{"blockdevices":[
         {"name":"nvme0n1","path":"/dev/nvme0n1","type":"disk","size":512110190592,"fstype":null,"label":null,"mountpoints":[null],"rm":false,"hotplug":false,"ro":false,"rota":false,"tran":"nvme","model":"AirDisk 512GB SSD","vendor":null,"serial":"S1","wwn":"eui.1","pttype":"gpt",
          "children":[
           {"name":"nvme0n1p1","path":"/dev/nvme0n1p1","type":"part","size":536870912,"fstype":"vfat","label":null,"mountpoints":["/boot/efi"]},
           {"name":"nvme0n1p2","path":"/dev/nvme0n1p2","type":"part","size":511000000000,"fstype":"btrfs","label":null,"mountpoints":["/","/home","/var/log"]}]},
         {"name":"sda","path":"/dev/sda","type":"disk","size":31914983424,"fstype":null,"mountpoints":[null],"rm":true,"hotplug":true,"ro":false,"rota":false,"tran":"usb","model":"Ultra Fit","vendor":"SanDisk ","serial":"4C53","wwn":null,"pttype":"dos",
          "children":[
           {"name":"sda1","path":"/dev/sda1","type":"part","size":31913934848,"fstype":"exfat","label":"PHOTOS","mountpoints":["/run/media/u/PHOTOS"],"fsused":12000000000}]},
         {"name":"sdb","path":"/dev/sdb","type":"disk","size":2000398934016,"fstype":null,"mountpoints":[null],"rm":false,"hotplug":true,"ro":false,"rota":true,"tran":"usb","model":"My Passport","vendor":"WD","serial":"WX1","wwn":"0x1","pttype":"gpt",
          "children":[
           {"name":"sdb1","path":"/dev/sdb1","type":"part","size":2000397795328,"fstype":"ntfs","label":"Backup","mountpoints":[null]}]},
         {"name":"mmcblk0","path":"/dev/mmcblk0","type":"disk","size":63864569856,"fstype":"exfat","label":"CARD","mountpoints":[null],"rm":true,"hotplug":true,"ro":false,"rota":false,"tran":null,"model":null,"vendor":null,"serial":null,"wwn":null,"pttype":null},
         {"name":"sr0","path":"/dev/sr0","type":"rom","size":1073741312,"fstype":null,"mountpoints":[null],"rm":true,"hotplug":true,"ro":false,"tran":"sata"},
         {"name":"sdc","path":"/dev/sdc","type":"disk","size":0,"fstype":null,"mountpoints":[null],"rm":true,"hotplug":true,"ro":false,"tran":"usb"},
         {"name":"sdd","path":"/dev/sdd","type":"disk","size":8000000000,"fstype":"iso9660","label":"ZOHARA_ISO","mountpoints":["/run/archiso/bootmnt"],"rm":true,"hotplug":true,"ro":true,"rota":false,"tran":"usb","model":"Flash","vendor":null,"serial":"L1","wwn":null,"pttype":"dos"},
         {"name":"loop0","path":"/dev/loop0","type":"loop","size":67108864,"fstype":null,"mountpoints":[null],"rm":false,"hotplug":false,"ro":false,"tran":null}
        ]}"#
        .to_string()
    }

    fn find<'a>(d: &'a [Drive], path: &str) -> &'a Drive {
        d.iter().find(|d| d.path == path).unwrap_or_else(|| panic!("no {path}"))
    }

    #[test]
    fn kinds_are_told_apart() {
        let d = parse_lsblk(&sample(), false);
        assert_eq!(find(&d, "/dev/nvme0n1").kind, Kind::Internal);
        assert_eq!(find(&d, "/dev/sda").kind, Kind::UsbStick, "a removable device behind USB is a stick");
        assert_eq!(find(&d, "/dev/sdb").kind, Kind::ExternalDisk, "a fixed disk behind USB is an external disk");
        assert_eq!(find(&d, "/dev/mmcblk0").kind, Kind::SdCard);
        assert!(d.iter().all(|d| d.path != "/dev/sr0"), "optical drives are not listed");
        assert!(d.iter().all(|d| d.path != "/dev/sdc"), "an empty card reader is not listed");
        assert!(d.iter().all(|d| d.kind != Kind::Image), "loop devices stay hidden unless asked for");
    }

    #[test]
    fn loop_devices_show_only_when_asked() {
        let d = parse_lsblk(&sample(), true);
        assert_eq!(find(&d, "/dev/loop0").kind, Kind::Image);
        assert!(find(&d, "/dev/loop0").can_modify());
    }

    #[test]
    fn the_system_disk_is_locked_for_good() {
        let d = parse_lsblk(&sample(), false);
        let sys = find(&d, "/dev/nvme0n1");
        assert!(sys.system && !sys.can_modify());
        assert!(sys.locked_reason().unwrap().contains("running from this disk"));
    }

    #[test]
    fn the_live_usb_stick_is_locked_too() {
        // The stick Zohara was started from holds the live image: formatting it would pull the system out from under itself.
        let d = parse_lsblk(&sample(), false);
        let live = find(&d, "/dev/sdd");
        assert!(live.system && !live.can_modify());
    }

    #[test]
    fn ordinary_external_drives_may_be_changed() {
        let d = parse_lsblk(&sample(), false);
        for p in ["/dev/sda", "/dev/sdb", "/dev/mmcblk0"] {
            assert!(find(&d, p).can_modify(), "{p}");
            assert!(find(&d, p).locked_reason().is_none());
        }
    }

    #[test]
    fn an_internal_data_disk_is_not_offered_but_has_a_reason() {
        let json = r#"{"blockdevices":[{"name":"sdz","path":"/dev/sdz","type":"disk","size":1000000000000,"fstype":null,"mountpoints":[null],"rm":false,"hotplug":false,"ro":false,"tran":"sata","model":"Data HDD"}]}"#;
        let d = parse_lsblk(json, false);
        assert_eq!(d[0].kind, Kind::Internal);
        assert!(!d[0].can_modify());
        assert!(d[0].locked_reason().unwrap().contains("inside the computer"));
    }

    #[test]
    fn a_hot_pluggable_internal_disk_is_not_offered() {
        // Some boards report internal SATA disks as hot-pluggable. Only USB, removable media and SD cards count as outside.
        let json = r#"{"blockdevices":[{"name":"sdx","path":"/dev/sdx","type":"disk","size":2000000000000,"fstype":null,"mountpoints":[null],"rm":false,"hotplug":true,"ro":false,"rota":true,"tran":"sata","model":"Data HDD"}]}"#;
        let d = parse_lsblk(json, false);
        assert_eq!(d[0].kind, Kind::Internal);
        assert!(!d[0].can_modify());
    }

    #[test]
    fn a_write_protected_stick_says_so() {
        let json = r#"{"blockdevices":[{"name":"sde","path":"/dev/sde","type":"disk","size":4000000000,"fstype":null,"mountpoints":[null],"rm":true,"hotplug":true,"ro":true,"tran":"usb"}]}"#;
        let d = parse_lsblk(json, false);
        assert!(!d[0].can_modify());
        assert!(d[0].locked_reason().unwrap().contains("write-protected"));
    }

    #[test]
    fn system_mounts_are_recognised() {
        for m in ["/", "/boot", "/boot/efi", "/home", "/home/u", "/var/log", "/usr/share", "[SWAP]", "/run/archiso/bootmnt", "/efi"] {
            assert!(is_system_mount(m), "{m}");
        }
        for m in ["/run/media/u/PHOTOS", "/mnt/backup", "/media/usb", "/homework", "/variable"] {
            assert!(!is_system_mount(m), "{m}");
        }
    }

    #[test]
    fn partitions_and_sizes_are_read() {
        let d = parse_lsblk(&sample(), false);
        let s = find(&d, "/dev/sda");
        assert_eq!(s.parts.len(), 1);
        assert_eq!(s.parts[0].label, "PHOTOS");
        assert_eq!(s.parts[0].fstype, "exfat");
        assert_eq!(s.parts[0].used, Some(12_000_000_000));
        assert!(s.parts[0].mounted());
        assert_eq!(s.mounted_paths(), vec!["/dev/sda1"]);
        assert!(!find(&d, "/dev/sdb").parts[0].mounted(), "a null mount point is not a mount");
    }

    #[test]
    fn a_filesystem_straight_on_the_disk_counts() {
        let d = parse_lsblk(&sample(), false);
        let card = find(&d, "/dev/mmcblk0");
        assert!(card.parts.is_empty());
        assert_eq!(card.whole_fs.as_ref().map(|p| p.fstype.as_str()), Some("exfat"));
        assert_eq!(card.filesystems().len(), 1);
    }

    #[test]
    fn titles_read_well() {
        let d = parse_lsblk(&sample(), false);
        assert_eq!(find(&d, "/dev/sda").title(), "SanDisk Ultra Fit");
        assert_eq!(find(&d, "/dev/sdb").title(), "WD My Passport");
        assert_eq!(find(&d, "/dev/mmcblk0").title(), "SD card");
        assert_eq!(find(&d, "/dev/sda").summary(), "29.7 GB · USB drive");
    }

    #[test]
    fn human_sizes() {
        assert_eq!(human(0), "0 B");
        assert_eq!(human(1500), "1 KB");
        assert_eq!(human(5 * 1024 * 1024), "5 MB");
        assert_eq!(human(3_221_225_472), "3.0 GB");
    }

    #[test]
    fn labels_follow_each_filesystem() {
        assert_eq!(normalize_label("vfat", " my files ").unwrap(), "MY FILES");
        assert!(normalize_label("vfat", "a:b").is_err());
        assert!(normalize_label("vfat", "TWELVE_CHARS").is_err(), "FAT32 names stop at 11");
        assert_eq!(normalize_label("exfat", "Holiday 26").unwrap(), "Holiday 26");
        assert!(normalize_label("exfat", "0123456789a").is_ok());
        assert!(normalize_label("exfat", "0123456789ab").is_err(), "exFAT names stop at 11 (UDisks refuses 14 characters: seen on a real run)");
        assert!(normalize_label("ext4", "0123456789abcdef").is_ok());
        assert!(normalize_label("ext4", "0123456789abcdefg").is_err(), "ext4 names stop at 16");
        assert!(normalize_label("ext4", "a/b").is_err());
        assert_eq!(normalize_label("btrfs", "").unwrap(), "");
        assert!(normalize_label("exfat", "a\nb").is_err());
        assert!(normalize_label("zfs", "x").is_err());
        assert!(normalize_label("ext4", "Ünï").is_ok());
    }

    #[test]
    fn partition_tables_suit_the_drive() {
        assert_eq!(table_for("exfat", 32_000_000_000), "dos", "small Windows-style drives get the old table");
        assert_eq!(table_for("vfat", 16_000_000_000), "dos");
        assert_eq!(table_for("exfat", 4_000_000_000_000), "gpt", "over 2 TB the old table can't address it");
        assert_eq!(table_for("ext4", 8_000_000_000), "gpt");
        assert_eq!(partition_type("dos", "vfat"), "0x0c");
        assert_eq!(partition_type("dos", "exfat"), "0x07");
        assert_eq!(partition_type("dos", "ext4"), "0x83");
        assert_eq!(partition_type("gpt", "exfat"), "ebd0a0a2-b9e5-4433-87c0-68b6b72699c7");
        assert_eq!(partition_type("gpt", "ext4"), "0fc63daf-8483-4772-8e79-3d69d8477de4");
    }

    #[test]
    fn what_is_lost_is_listed_in_words() {
        let d = parse_lsblk(&sample(), false);
        assert_eq!(what_will_be_lost(find(&d, "/dev/sda")), vec!["PHOTOS (exFAT, 11.2 GB used)"]);
        assert_eq!(what_will_be_lost(find(&d, "/dev/sdb")), vec!["Backup (NTFS)"]);
    }

    #[test]
    fn a_swapped_drive_stops_the_change() {
        let d = parse_lsblk(&sample(), false);
        let sda = find(&d, "/dev/sda").clone();
        let id = sda.identity();
        assert!(still_the_same(&id, &d).is_ok());
        // gone
        let without: Vec<Drive> = d.iter().filter(|x| x.path != "/dev/sda").cloned().collect();
        assert!(still_the_same(&id, &without).unwrap_err().contains("removed"));
        // another drive in the same place
        let mut other = d.clone();
        for x in other.iter_mut() {
            if x.path == "/dev/sda" {
                x.serial = "OTHER".into();
            }
        }
        assert!(still_the_same(&id, &other).unwrap_err().contains("different drive"));
        let mut bigger = d.clone();
        for x in bigger.iter_mut() {
            if x.path == "/dev/sda" {
                x.size *= 2;
            }
        }
        assert!(still_the_same(&id, &bigger).is_err());
    }

    #[test]
    fn a_drive_that_became_the_system_stops_the_change() {
        let d = parse_lsblk(&sample(), false);
        let id = find(&d, "/dev/sda").identity();
        let mut now = d.clone();
        for x in now.iter_mut() {
            if x.path == "/dev/sda" {
                x.system = true;
            }
        }
        assert!(still_the_same(&id, &now).is_err());
        // and the real system disk can never be confirmed
        let sysid = find(&d, "/dev/nvme0n1").identity();
        assert!(still_the_same(&sysid, &d).is_err());
    }

    #[test]
    fn device_paths_are_checked_before_they_reach_dbus() {
        assert_eq!(block_object_path("/dev/sdb1").unwrap(), "/org/freedesktop/UDisks2/block_devices/sdb1");
        assert_eq!(block_object_path("/dev/nvme0n1p2").unwrap(), "/org/freedesktop/UDisks2/block_devices/nvme0n1p2");
        assert_eq!(block_object_path("/dev/mmcblk0p1").unwrap(), "/org/freedesktop/UDisks2/block_devices/mmcblk0p1");
        for bad in ["sdb", "/dev/", "/dev/../etc", "/dev/sd b", "/dev/sdb;rm", "/etc/passwd", "/dev/disk/by-id/x"] {
            assert!(block_object_path(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn errors_are_put_in_plain_words() {
        assert!(friendly_error("GDBus.Error:org.freedesktop.UDisks2.Error.NotAuthorizedCanObtain: not authorized").contains("wasn't allowed"));
        assert!(friendly_error("Error mounting: target is busy").contains("still using"));
        assert!(friendly_error("Device is mounted").contains("still using"));
        assert!(friendly_error("write-protected").contains("write-protected"));
        assert!(friendly_error("Error: mkfs failed: bad block").starts_with("The change failed"));
    }

    fn temp(name: &str, data: &[u8]) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("zohara-drives-test-{}-{name}", std::process::id()));
        std::fs::write(&p, data).unwrap();
        p
    }

    #[test]
    fn image_files_are_checked() {
        let empty = temp("empty", b"");
        assert!(check_image_file(&empty).is_err());
        assert!(check_image_file(Path::new("/nonexistent/x.iso")).is_err());
        assert!(check_image_file(Path::new("/tmp")).is_err());
        let mut iso = vec![0u8; 0x8001 + 5 + 100];
        iso[0x8001..0x8001 + 5].copy_from_slice(b"CD001");
        let good = temp("good.iso", &iso);
        assert_eq!(check_image_file(&good).unwrap(), iso.len() as u64);
        assert!(looks_like_iso(&good));
        let plain = temp("plain.img", &vec![1u8; 70000]);
        assert!(!looks_like_iso(&plain));
        for p in [empty, good, plain] {
            let _ = std::fs::remove_file(p);
        }
    }

    #[test]
    fn an_image_must_fit_and_not_live_on_the_drive() {
        let d = parse_lsblk(&sample(), false);
        let sda = find(&d, "/dev/sda");
        let p = temp("fit.iso", b"x");
        assert!(check_image_fits(&p, 1000, sda).is_ok());
        assert!(check_image_fits(&p, sda.size + 1, sda).unwrap_err().contains("holds only"));
        // an image stored on the stick itself
        let mut on_stick = sda.clone();
        on_stick.parts[0].mountpoints = vec![std::env::temp_dir().to_string_lossy().to_string()];
        assert!(check_image_fits(&p, 1, &on_stick).unwrap_err().contains("stored on this drive"));
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn copying_an_image_reports_progress_and_can_be_verified() {
        let data: Vec<u8> = (0..(CHUNK * 2 + 1234)).map(|i| (i % 251) as u8).collect();
        let progress = Progress::new();
        let mut dst: Vec<u8> = Vec::new();
        copy_image(&mut &data[..], &mut dst, data.len() as u64, &progress).unwrap();
        assert_eq!(dst, data);
        assert_eq!(progress.done.load(Ordering::SeqCst), data.len() as u64);
        assert_eq!(progress.phase.load(Ordering::SeqCst), 1);
        verify_image(&mut &data[..], &mut &dst[..], data.len() as u64, &progress).unwrap();
        assert_eq!(progress.phase.load(Ordering::SeqCst), 2);
        let mut bad = dst.clone();
        bad[CHUNK + 5] ^= 0xff;
        let err = verify_image(&mut &data[..], &mut &bad[..], data.len() as u64, &progress).unwrap_err();
        assert!(err.contains("does not match"), "{err}");
    }

    #[test]
    fn copying_can_be_cancelled() {
        let data = vec![7u8; CHUNK * 3];
        let progress = Progress::new();
        progress.cancel.store(true, Ordering::SeqCst);
        let mut dst: Vec<u8> = Vec::new();
        let err = copy_image(&mut &data[..], &mut dst, data.len() as u64, &progress).unwrap_err();
        assert!(err.contains("Cancelled"));
        assert!(dst.is_empty());
    }

    #[test]
    fn a_short_image_is_reported_not_ignored() {
        let progress = Progress::new();
        let mut dst: Vec<u8> = Vec::new();
        let short = vec![1u8; 10];
        assert!(copy_image(&mut &short[..], &mut dst, 20, &progress).is_err());
    }

    #[test]
    fn the_filesystem_list_is_complete() {
        for id in ["exfat", "vfat", "ntfs", "ext4", "btrfs"] {
            assert!(fs_by_id(id).is_some(), "{id}");
        }
        assert!(fs_by_id("xfs").is_none());
        // every offered filesystem must have a name rule that accepts an empty name
        for f in FILESYSTEMS {
            assert_eq!(normalize_label(f.id, "").unwrap(), "");
        }
    }
}

// ── Live tests: a file made to look like a drive (a loop device); never a real disk ─────────────

#[cfg(test)]
mod live {
    use super::*;

    /// A 64 MB loop device. Removed again when dropped.
    struct Fake {
        dev: String,
        image: std::path::PathBuf,
    }

    impl Fake {
        fn new(tag: &str) -> Fake {
            let image = std::env::temp_dir().join(format!("zohara-fake-drive-{}-{tag}.img", std::process::id()));
            let f = std::fs::File::create(&image).unwrap();
            f.set_len(64 * 1024 * 1024).unwrap();
            let out = Command::new("udisksctl").args(["loop-setup", "-f"]).arg(&image).arg("--no-user-interaction").output().unwrap();
            let text = String::from_utf8_lossy(&out.stdout).to_string();
            let dev = text.trim().trim_end_matches('.').rsplit(' ').next().unwrap().to_string();
            assert!(dev.starts_with("/dev/loop"), "loop-setup said: {text} {}", String::from_utf8_lossy(&out.stderr));
            Fake { dev, image }
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            let _ = Command::new("udisksctl").args(["loop-delete", "-b", &self.dev, "--no-user-interaction"]).output();
            let _ = std::fs::remove_file(&self.image);
        }
    }

    fn quiet() -> Udisks {
        Udisks { interactive: false }
    }

    fn listed(dev: &str) -> Drive {
        std::env::set_var("ZOHARA_DRIVES_SHOW_IMAGES", "1");
        std::thread::sleep(std::time::Duration::from_millis(700));
        find(dev).unwrap_or_else(|| panic!("{dev} not listed"))
    }

    #[test]
    #[ignore]
    fn live_a_fake_drive_is_listed_and_can_be_formatted_renamed_checked_and_erased() {
        let fake = Fake::new("all");
        let d = listed(&fake.dev);
        assert_eq!(d.kind, Kind::Image);
        assert!(d.can_modify());
        let id = d.identity();

        // format the whole (partition-less) disk as exFAT
        let plan = FormatPlan { fs: "exfat".into(), label: "Ztest".into(), zero_fill: false };
        format_partition(quiet(), &id, &fake.dev, &plan).unwrap();
        let d = listed(&fake.dev);
        assert_eq!(d.whole_fs.as_ref().map(|p| (p.fstype.as_str(), p.label.as_str())), Some(("exfat", "Ztest")), "{d:?}");

        // rename
        quiet().set_label(&fake.dev, "Renamed").unwrap();
        assert_eq!(listed(&fake.dev).whole_fs.unwrap().label, "Renamed");

        // check (unmounted)
        assert_eq!(quiet().check(&fake.dev, false), Ok(true));

        // erase the whole drive: new table, one partition
        let plan = FormatPlan { fs: "ext4".into(), label: "Linux".into(), zero_fill: false };
        erase_drive(quiet(), &id, &plan).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(800));
        let d = listed(&fake.dev);
        assert_eq!(d.pttype, "gpt", "a Linux filesystem gets a GPT table: {d:?}");
        assert_eq!(d.parts.len(), 1, "{d:?}");
        assert_eq!((d.parts[0].fstype.as_str(), d.parts[0].label.as_str()), ("ext4", "Linux"));

        // a small exFAT drive gets the old table
        let plan = FormatPlan { fs: "vfat".into(), label: "stick".into(), zero_fill: false };
        erase_drive(quiet(), &id, &plan).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(800));
        let d = listed(&fake.dev);
        assert_eq!(d.pttype, "dos", "{d:?}");
        assert_eq!((d.parts[0].fstype.as_str(), d.parts[0].label.as_str()), ("vfat", "STICK"), "FAT32 names are upper case: {d:?}");
    }

    #[test]
    #[ignore]
    fn live_the_wrong_drive_is_refused_before_anything_is_written() {
        let fake = Fake::new("guard");
        let d = listed(&fake.dev);
        let mut id = d.identity();
        id.serial = "somebody-elses-serial".into();
        let plan = FormatPlan { fs: "exfat".into(), label: "X".into(), zero_fill: false };
        let err = format_partition(quiet(), &id, &fake.dev, &plan).unwrap_err();
        assert!(err.contains("different drive"), "{err}");
        assert!(listed(&fake.dev).whole_fs.is_none(), "nothing was written");
    }

    #[test]
    #[ignore]
    fn live_the_running_systems_disk_can_never_be_confirmed() {
        // The disk behind "/" is on this very machine. It must be locked and refused, whatever is asked. Nothing is written.
        std::env::remove_var("ZOHARA_DRIVES_SHOW_IMAGES");
        let sys: Vec<Drive> = list().into_iter().filter(|d| d.system).collect();
        assert!(!sys.is_empty(), "the disk holding / must be recognised as the system's");
        for d in &sys {
            let plan = FormatPlan { fs: "exfat".into(), label: "NO".into(), zero_fill: false };
            let err = erase_drive(quiet(), &d.identity(), &plan).unwrap_err();
            assert!(err.contains("running from this disk") || err.contains("inside the computer"), "{err}");
        }
    }

    /// Writing a whole drive needs the administrator password (PolicyKit action `open-device`). Unattended, that must be refused
    /// and nothing written. With `ZOHARA_LIVE_INTERACTIVE=1` a password window appears and the image is really written and checked.
    #[test]
    #[ignore]
    fn live_an_image_is_written_to_a_fake_drive_and_checked() {
        let fake = Fake::new("write");
        let d = listed(&fake.dev);
        let id = d.identity();
        let interactive = std::env::var("ZOHARA_LIVE_INTERACTIVE").map(|v| v == "1").unwrap_or(false);
        // a 20 MB "ISO"
        let mut data: Vec<u8> = (0..(20 * 1024 * 1024)).map(|i| (i * 31 % 253) as u8).collect();
        data[0x8001..0x8001 + 5].copy_from_slice(b"CD001");
        let iso = std::env::temp_dir().join(format!("zohara-fake-{}.iso", std::process::id()));
        std::fs::write(&iso, &data).unwrap();
        let progress = Progress::new();
        let result = write_image(Udisks { interactive }, &id, &iso, true, &progress);
        if !interactive {
            let err = result.unwrap_err();
            assert!(err.contains("wasn't allowed"), "unattended writing must be refused: {err}");
            // nothing was written: the backing file is still all zeros at the start
            let mut head = vec![0u8; 4096];
            std::fs::File::open(&fake.image).unwrap().read_exact(&mut head).unwrap();
            assert!(head.iter().all(|b| *b == 0), "nothing was written");
        } else {
            result.unwrap();
            assert_eq!(progress.phase.load(Ordering::SeqCst), 3);
            let mut back = std::fs::File::open(&fake.image).unwrap();
            let mut got = vec![0u8; data.len()];
            back.read_exact(&mut got).unwrap();
            assert!(got == data, "the drive holds exactly the image");
        }
        let _ = std::fs::remove_file(iso);
    }

    #[test]
    #[ignore]
    fn live_an_image_that_does_not_fit_is_refused() {
        let fake = Fake::new("big");
        let d = listed(&fake.dev);
        let big = std::env::temp_dir().join(format!("zohara-fake-big-{}.img", std::process::id()));
        let f = std::fs::File::create(&big).unwrap();
        f.set_len(100 * 1024 * 1024).unwrap();
        let err = write_image(quiet(), &d.identity(), &big, false, &Progress::new()).unwrap_err();
        assert!(err.contains("holds only"), "{err}");
        let _ = std::fs::remove_file(big);
    }
}
