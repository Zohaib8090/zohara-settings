//! Mac app checker: look at a `.app` folder, an iPhone `.ipa`, a `.dmg` or a bare Mach-O file and say, in plain words, whether
//! Zohara has any chance of running it. Reads only what the app declares about itself (`Info.plist` and the Mach-O load commands:
//! which CPU it is built for and which Apple libraries it asks for). It never runs the app and never touches Apple's code.
//!
//! The answer is only "might run" at best: macOS apps need Darling (a macOS translation layer) and Zohara has **not measured**
//! Darling yet (see `docs/RUN-ANYTHING-MACOS-IOS-2026-10-19.md` in the `zohara` repo). The framework lists below come from general
//! knowledge of what Darling covers, not from a test here, and the text says so. iPhone apps cannot run: there is no UIKit or Metal
//! for Linux, and App Store apps are encrypted (removing that protection is not something Zohara builds).

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::process::{Command, Stdio};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Only classic Apple libraries: worth trying under Darling.
    Might,
    /// Needs something Darling is not known to have.
    Unlikely,
    /// Cannot run here (iPhone app, Apple-silicon-only build).
    Cannot,
    /// Not enough to say (a disk image, an unreadable file).
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    MacApp,
    IosApp,
    MachO,
    DiskImage,
    Other,
}

#[derive(Clone, Debug)]
pub struct Report {
    pub name: String,
    pub kind: Kind,
    pub verdict: Verdict,
    /// Plain sentences, most important first.
    pub reasons: Vec<String>,
    /// Apple libraries the app asks for (`Foundation`, `AppKit`, ...).
    pub apple_frameworks: Vec<String>,
    /// CPU types found, e.g. `x86_64`, `arm64`.
    pub archs: Vec<String>,
}

// ── Mach-O ─────────────────────────────────────────────────────────────────

const CPU_X86: u32 = 7;
const CPU_X86_64: u32 = 0x0100_0007;
const CPU_ARM64: u32 = 0x0100_000C;

const LC_LOAD_DYLIB: u32 = 0x0C;
const LC_LOAD_WEAK_DYLIB: u32 = 0x8000_0018;
const LC_REEXPORT_DYLIB: u32 = 0x8000_001F;
const LC_LOAD_UPWARD_DYLIB: u32 = 0x8000_0023;
const LC_VERSION_MIN_MACOSX: u32 = 0x24;
const LC_VERSION_MIN_IPHONEOS: u32 = 0x25;
const LC_VERSION_MIN_TVOS: u32 = 0x2F;
const LC_VERSION_MIN_WATCHOS: u32 = 0x30;
const LC_BUILD_VERSION: u32 = 0x32;
const LC_ENCRYPTION_INFO: u32 = 0x21;
const LC_ENCRYPTION_INFO_64: u32 = 0x2C;

/// What one CPU slice of a Mach-O file says about itself.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Slice {
    pub cpu: u32,
    pub dylibs: Vec<String>,
    /// 1 macOS, 2 iOS, 3 tvOS, 4 watchOS, ... (the numbering of `LC_BUILD_VERSION`).
    pub platform: Option<u32>,
    /// App Store encryption (`cryptid` 1).
    pub encrypted: bool,
}

fn rd32(b: &[u8], off: usize, le: bool) -> Option<u32> {
    let s: [u8; 4] = b.get(off..off + 4)?.try_into().ok()?;
    Some(if le { u32::from_le_bytes(s) } else { u32::from_be_bytes(s) })
}

fn rd64(b: &[u8], off: usize, le: bool) -> Option<u64> {
    let s: [u8; 8] = b.get(off..off + 8)?.try_into().ok()?;
    Some(if le { u64::from_le_bytes(s) } else { u64::from_be_bytes(s) })
}

type ReadAt<'a> = &'a dyn Fn(u64, usize) -> Option<Vec<u8>>;

/// One thin Mach-O starting at `base`.
fn parse_slice(read: ReadAt, base: u64) -> Option<Slice> {
    let head = read(base, 32)?;
    let magic = rd32(&head, 0, true)?;
    let hdr = match magic {
        0xFEED_FACF => 32usize,
        0xFEED_FACE => 28usize,
        _ => return None,
    };
    let cpu = rd32(&head, 4, true)?;
    let ncmds = rd32(&head, 16, true)? as usize;
    let size = rd32(&head, 20, true)? as usize;
    if size > 4 * 1024 * 1024 || ncmds > 4096 {
        return None;
    }
    let cmds = read(base + hdr as u64, size)?;
    let mut s = Slice { cpu, ..Slice::default() };
    let mut at = 0usize;
    for _ in 0..ncmds {
        let cmd = rd32(&cmds, at, true)?;
        let csize = rd32(&cmds, at + 4, true)? as usize;
        if csize < 8 || at + csize > cmds.len() {
            return None;
        }
        let body = &cmds[at..at + csize];
        match cmd {
            LC_LOAD_DYLIB | LC_LOAD_WEAK_DYLIB | LC_REEXPORT_DYLIB | LC_LOAD_UPWARD_DYLIB => {
                let off = rd32(body, 8, true)? as usize;
                let raw = body.get(off..)?;
                let end = raw.iter().position(|b| *b == 0).unwrap_or(raw.len());
                s.dylibs.push(String::from_utf8_lossy(&raw[..end]).to_string());
            }
            LC_BUILD_VERSION => s.platform = rd32(body, 8, true),
            LC_VERSION_MIN_MACOSX => s.platform = Some(1),
            LC_VERSION_MIN_IPHONEOS => s.platform = Some(2),
            LC_VERSION_MIN_TVOS => s.platform = Some(3),
            LC_VERSION_MIN_WATCHOS => s.platform = Some(4),
            LC_ENCRYPTION_INFO | LC_ENCRYPTION_INFO_64 => s.encrypted = rd32(body, 16, true).unwrap_or(0) != 0,
            _ => {}
        }
        at += csize;
    }
    Some(s)
}

/// Every CPU slice of a Mach-O file (thin or "fat"); `None` when it is not a Mach-O file.
pub fn parse_macho(read: ReadAt) -> Option<Vec<Slice>> {
    let head = read(0, 8)?;
    match rd32(&head, 0, false)? {
        0xCAFE_BABE | 0xCAFE_BABF => {
            let wide = rd32(&head, 0, false)? == 0xCAFE_BABF;
            let n = rd32(&head, 4, false)? as usize;
            if n == 0 || n > 16 {
                return None; // also rejects Java class files, which share the first four bytes
            }
            let entry = if wide { 32 } else { 20 };
            let table = read(8, n * entry)?;
            let mut out = Vec::new();
            for i in 0..n {
                let e = i * entry;
                let off = if wide { rd64(&table, e + 8, false)? } else { rd32(&table, e + 8, false)? as u64 };
                if let Some(s) = parse_slice(read, off) {
                    out.push(s);
                }
            }
            (!out.is_empty()).then_some(out)
        }
        _ => parse_slice(read, 0).map(|s| vec![s]),
    }
}

fn read_at_file(path: &Path) -> impl Fn(u64, usize) -> Option<Vec<u8>> {
    let path = path.to_path_buf();
    move |off, len| {
        let mut f = File::open(&path).ok()?;
        f.seek(SeekFrom::Start(off)).ok()?;
        let mut buf = vec![0u8; len];
        let mut got = 0;
        while got < len {
            let n = f.read(&mut buf[got..]).ok()?;
            if n == 0 {
                break;
            }
            got += n;
        }
        buf.truncate(got);
        (got > 0).then_some(buf)
    }
}

fn cpu_name(cpu: u32) -> String {
    match cpu {
        CPU_X86_64 => "x86_64".into(),
        CPU_ARM64 => "arm64".into(),
        CPU_X86 => "i386 (32-bit Intel)".into(),
        other => format!("cpu {other:#x}"),
    }
}

// ── Info.plist (XML and binary), only the string values ────────────────────

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// `<key>K</key><string>V</string>` pairs of an XML plist.
fn plist_xml(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut rest = text;
    while let Some(i) = rest.find("<key>") {
        rest = &rest[i + 5..];
        let Some(j) = rest.find("</key>") else { break };
        let key = unescape(&rest[..j]);
        rest = &rest[j + 6..];
        let t = rest.trim_start();
        if let Some(v) = t.strip_prefix("<string>") {
            if let Some(k) = v.find("</string>") {
                out.insert(key, unescape(&v[..k]));
            }
        }
    }
    out
}

/// Size and offset helpers of a binary plist (`bplist00`).
fn bp_int(b: &[u8], off: usize, n: usize) -> Option<usize> {
    let s = b.get(off..off + n)?;
    Some(s.iter().fold(0usize, |a, x| (a << 8) | *x as usize))
}

fn bp_len(b: &[u8], off: usize) -> Option<(usize, usize)> {
    // (length, bytes the length itself takes after the marker)
    let low = (*b.get(off)? & 0x0F) as usize;
    if low != 0x0F {
        return Some((low, 0));
    }
    let m = *b.get(off + 1)?;
    if m >> 4 != 1 {
        return None;
    }
    let n = 1usize << (m & 0x0F);
    Some((bp_int(b, off + 2, n)?, 1 + n))
}

fn bp_string(b: &[u8], off: usize) -> Option<String> {
    let t = *b.get(off)? >> 4;
    let (len, extra) = bp_len(b, off)?;
    let start = off + 1 + extra;
    match t {
        0x5 => Some(String::from_utf8_lossy(b.get(start..start + len)?).to_string()),
        0x6 => {
            let raw = b.get(start..start + len * 2)?;
            let units: Vec<u16> = raw.chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
            Some(String::from_utf16_lossy(&units))
        }
        _ => None,
    }
}

fn plist_binary(b: &[u8]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let _ = (|| -> Option<()> {
        if b.len() < 40 || !b.starts_with(b"bplist00") {
            return None;
        }
        let t = &b[b.len() - 32..];
        let (osz, rsz) = (t[6] as usize, t[7] as usize);
        let nobj = bp_int(t, 8, 8)?;
        let top = bp_int(t, 16, 8)?;
        let table = bp_int(t, 24, 8)?;
        if osz == 0 || rsz == 0 || nobj > 1_000_000 {
            return None;
        }
        let obj_off = |i: usize| -> Option<usize> { bp_int(b, table + i * osz, osz) };
        let top_off = obj_off(top)?;
        if *b.get(top_off)? >> 4 != 0xD {
            return None;
        }
        let (n, extra) = bp_len(b, top_off)?;
        let refs = top_off + 1 + extra;
        for i in 0..n {
            let k = bp_int(b, refs + i * rsz, rsz)?;
            let v = bp_int(b, refs + (n + i) * rsz, rsz)?;
            if let (Some(ks), Some(vs)) = (bp_string(b, obj_off(k)?), bp_string(b, obj_off(v)?)) {
                out.insert(ks, vs);
            }
        }
        Some(())
    })();
    out
}

/// The string values of an `Info.plist`, XML or binary; empty when it cannot be read.
pub fn plist_strings(data: &[u8]) -> HashMap<String, String> {
    if data.starts_with(b"bplist00") {
        plist_binary(data)
    } else {
        plist_xml(&String::from_utf8_lossy(data))
    }
}

// ── What the libraries mean ────────────────────────────────────────────────

/// `/System/Library/Frameworks/AppKit.framework/Versions/C/AppKit` -> `AppKit`.
fn framework_name(dylib: &str) -> Option<String> {
    let rest = dylib.strip_prefix("/System/Library/Frameworks/").or_else(|| dylib.strip_prefix("/System/Library/PrivateFrameworks/"))?;
    Some(rest.split(".framework").next()?.to_string())
}

/// Libraries of the classic Mac API that a macOS translation layer is built around.
const CLASSIC: &[&str] = &[
    "Foundation", "CoreFoundation", "AppKit", "Cocoa", "Carbon", "ApplicationServices", "CoreServices", "CoreGraphics", "CoreText",
    "QuartzCore", "OpenGL", "Security", "SystemConfiguration", "IOKit", "AudioToolbox", "CoreAudio", "AudioUnit", "DiskArbitration",
    "CFNetwork", "ImageIO", "Accelerate", "CoreData", "AddressBook", "ServiceManagement", "Quartz",
];

/// Libraries that need Apple's newer closed parts (graphics stack, Swift UI, cloud, machine learning): not known to exist for Linux.
const HARD: &[&str] = &[
    "Metal", "MetalKit", "MetalPerformanceShaders", "SwiftUI", "RealityKit", "ARKit", "StoreKit", "CloudKit", "CoreML", "CreateML", "Vision",
    "WebKit", "AppIntents", "WidgetKit", "ScreenCaptureKit", "PassKit", "HealthKit", "AuthenticationServices", "GameKit", "SpriteKit", "SceneKit",
    "AVKit", "UIKit", "CoreHaptics",
];

fn darling_installed() -> bool {
    ["/usr/bin/darling", "/usr/local/bin/darling"].iter().any(|p| Path::new(p).exists())
}

/// Turns the slices of an executable into the answer.
fn judge(slices: &[Slice], kind: Kind) -> (Verdict, Vec<String>, Vec<String>, Vec<String>) {
    let mut reasons: Vec<String> = Vec::new();
    let archs: Vec<String> = slices.iter().map(|s| cpu_name(s.cpu)).collect();
    let mut frameworks: Vec<String> = Vec::new();
    for s in slices {
        for d in &s.dylibs {
            if let Some(f) = framework_name(d) {
                if !frameworks.contains(&f) {
                    frameworks.push(f);
                }
            }
        }
    }
    frameworks.sort();

    let ios = kind == Kind::IosApp || slices.iter().any(|s| matches!(s.platform, Some(2) | Some(3) | Some(4) | Some(7) | Some(11))) || frameworks.iter().any(|f| f == "UIKit");
    if ios {
        reasons.push("This is an iPhone or iPad app. It is built on Apple's phone libraries (UIKit and Metal), which do not exist for Linux, so it cannot run on Zohara.".into());
        if slices.iter().any(|s| s.encrypted) {
            reasons.push("It is also locked by the App Store (encrypted). Zohara will not remove that protection.".into());
        }
        reasons.push("Look for the Android, web or Linux version of the app instead.".into());
        return (Verdict::Cannot, reasons, frameworks, archs);
    }

    let has_x86 = slices.iter().any(|s| s.cpu == CPU_X86_64);
    if !has_x86 {
        if slices.iter().any(|s| s.cpu == CPU_ARM64) {
            reasons.push("It is built only for Apple's own chips (arm64). Darling runs Intel programs (x86_64) only, so this one cannot run.".into());
            reasons.push("Ask the maker for an Intel (universal) build.".into());
        } else if slices.iter().any(|s| s.cpu == CPU_X86) {
            reasons.push("It is a 32-bit Intel program. macOS dropped those years ago, and Darling does not run them well, so it is very unlikely to work.".into());
        } else {
            reasons.push("It is not built for a CPU Zohara can use.".into());
        }
        return (Verdict::Cannot, reasons, frameworks, archs);
    }

    let hard: Vec<&String> = frameworks.iter().filter(|f| HARD.contains(&f.as_str())).collect();
    let swift = slices.iter().any(|s| s.dylibs.iter().any(|d| d.contains("libswift")));
    let unknown: Vec<&String> = frameworks.iter().filter(|f| !CLASSIC.contains(&f.as_str()) && !HARD.contains(&f.as_str())).collect();
    let mut verdict = Verdict::Might;
    if !hard.is_empty() {
        verdict = Verdict::Unlikely;
        let names = hard.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ");
        reasons.push(format!("It uses Apple libraries that are not known to exist for Linux: {names}. It will most likely not start."));
    }
    if swift {
        verdict = Verdict::Unlikely;
        reasons.push("It is written in Swift and needs Apple's Swift libraries, which Darling is not known to provide.".into());
    }
    if verdict == Verdict::Might {
        reasons.push("It only asks for the classic Mac libraries, so it is worth trying under Darling.".into());
        if !unknown.is_empty() {
            let names = unknown.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ");
            reasons.push(format!("It also uses libraries we have no opinion on yet: {names}."));
        }
    }
    if !darling_installed() {
        reasons.push("Darling, the program that would run it, is not installed on this computer.".into());
    }
    reasons.push("Zohara has not measured Darling yet: this is a guess from the list of libraries, not a test.".into());
    (verdict, reasons, frameworks, archs)
}

// ── Entry point ────────────────────────────────────────────────────────────

fn blank(name: &str, kind: Kind, verdict: Verdict, reason: &str) -> Report {
    Report { name: name.to_string(), kind, verdict, reasons: vec![reason.to_string()], apple_frameworks: vec![], archs: vec![] }
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| p.to_string_lossy().to_string())
}

/// Reads the first few MiB of one file inside a zip (`unzip -p`), as the `.ipa` check needs only the Mach-O header.
fn unzip_prefix(zip: &Path, inner: &str, max: usize) -> Option<Vec<u8>> {
    let mut child = Command::new("unzip").arg("-p").arg(zip).arg(inner).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let mut out = Vec::new();
    let mut so = child.stdout.take()?;
    let _ = (&mut so).take(max as u64).read_to_end(&mut out);
    let _ = child.kill();
    let _ = child.wait();
    (!out.is_empty()).then_some(out)
}

fn check_ipa(path: &Path) -> Report {
    let name = file_name(path);
    let list = Command::new("unzip").arg("-Z1").arg(path).stderr(Stdio::null()).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
    let app_dir = list.lines().find_map(|l| {
        let rest = l.strip_prefix("Payload/")?;
        let dir = rest.split('/').next()?;
        dir.ends_with(".app").then(|| format!("Payload/{dir}"))
    });
    let mut display = name.clone();
    let mut slices: Vec<Slice> = Vec::new();
    if let Some(dir) = app_dir {
        let plist = unzip_prefix(path, &format!("{dir}/Info.plist"), 4 * 1024 * 1024).map(|d| plist_strings(&d)).unwrap_or_default();
        if let Some(n) = plist.get("CFBundleDisplayName").or_else(|| plist.get("CFBundleName")) {
            display = n.clone();
        }
        if let Some(exe) = plist.get("CFBundleExecutable") {
            if let Some(bytes) = unzip_prefix(path, &format!("{dir}/{exe}"), 8 * 1024 * 1024) {
                let read = move |off: u64, len: usize| -> Option<Vec<u8>> {
                    let o = off as usize;
                    (o < bytes.len()).then(|| bytes[o..(o + len).min(bytes.len())].to_vec())
                };
                slices = parse_macho(&read).unwrap_or_default();
            }
        }
    }
    let (verdict, reasons, apple_frameworks, archs) = judge(&slices, Kind::IosApp);
    Report { name: display, kind: Kind::IosApp, verdict, reasons, apple_frameworks, archs }
}

/// Checks `path` (a `.app` folder, `.ipa`, `.dmg` or a Mach-O file) and answers in plain words. Never runs the app.
pub fn check(path: &Path) -> Report {
    let name = file_name(path);
    let lower = name.to_lowercase();
    if path.is_dir() {
        if !lower.ends_with(".app") {
            return blank(&name, Kind::Other, Verdict::Unknown, "This folder is not a Mac app (its name does not end in .app).");
        }
        let plist = std::fs::read(path.join("Contents/Info.plist")).map(|d| plist_strings(&d)).unwrap_or_default();
        let display = plist.get("CFBundleDisplayName").or_else(|| plist.get("CFBundleName")).cloned().unwrap_or_else(|| name.trim_end_matches(".app").to_string());
        let Some(exe) = plist.get("CFBundleExecutable") else {
            // an app folder with no readable Info.plist or no Contents/MacOS is not a program (a data folder, say)
            let has_macos = path.join("Contents/MacOS").is_dir();
            return blank(
                &display,
                Kind::MacApp,
                Verdict::Unknown,
                if has_macos { "The app's Info.plist could not be read, so Zohara cannot tell what it needs." } else { "This .app folder holds no program (there is no Contents/MacOS). It looks like a data folder." },
            );
        };
        let exe_path = path.join("Contents/MacOS").join(exe);
        let read = read_at_file(&exe_path);
        let Some(slices) = parse_macho(&read) else {
            return blank(&display, Kind::MacApp, Verdict::Unknown, "The program inside could not be read as a Mac program.");
        };
        let (verdict, reasons, apple_frameworks, archs) = judge(&slices, Kind::MacApp);
        return Report { name: display, kind: if verdict == Verdict::Cannot && slices.iter().any(|s| matches!(s.platform, Some(2))) { Kind::IosApp } else { Kind::MacApp }, verdict, reasons, apple_frameworks, archs };
    }
    if lower.ends_with(".ipa") {
        return check_ipa(path);
    }
    if lower.ends_with(".dmg") {
        return blank(&name, Kind::DiskImage, Verdict::Unknown, "This is a Mac disk image. Open it first (right-click > Open with a file archiver such as Ark), then check the .app inside.");
    }
    let read = read_at_file(path);
    match parse_macho(&read) {
        Some(slices) => {
            let (verdict, reasons, apple_frameworks, archs) = judge(&slices, Kind::MachO);
            Report { name, kind: Kind::MachO, verdict, reasons, apple_frameworks, archs }
        }
        None => blank(&name, Kind::Other, Verdict::Unknown, "This is not a Mac app, an iPhone app, a disk image or a Mac program file."),
    }
}

/// One-line headline for the dialog title.
pub fn headline(r: &Report) -> String {
    let what = match r.verdict {
        Verdict::Might => "might run",
        Verdict::Unlikely => "will most likely not run",
        Verdict::Cannot => "cannot run",
        Verdict::Unknown => "cannot be checked",
    };
    format!("{} {what}", r.name)
}

/// The text for the dialog body (and for `--check-mac-app`).
pub fn explain(r: &Report) -> String {
    let mut s = r.reasons.join("\n\n");
    if !r.archs.is_empty() {
        s.push_str(&format!("\n\nBuilt for: {}", r.archs.join(", ")));
    }
    if !r.apple_frameworks.is_empty() {
        s.push_str(&format!("\nApple libraries it asks for: {}", r.apple_frameworks.join(", ")));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u32le(v: u32) -> Vec<u8> {
        v.to_le_bytes().to_vec()
    }

    fn dylib_cmd(name: &str) -> Vec<u8> {
        let mut c = Vec::new();
        let mut n = name.as_bytes().to_vec();
        n.push(0);
        while (24 + n.len()) % 8 != 0 {
            n.push(0);
        }
        c.extend(u32le(LC_LOAD_DYLIB));
        c.extend(u32le((24 + n.len()) as u32));
        c.extend(u32le(24)); // name offset
        c.extend(u32le(2)); // timestamp
        c.extend(u32le(0)); // current version
        c.extend(u32le(0)); // compat version
        c.extend(n);
        c
    }

    fn build_version(platform: u32) -> Vec<u8> {
        let mut c = Vec::new();
        c.extend(u32le(LC_BUILD_VERSION));
        c.extend(u32le(24));
        c.extend(u32le(platform));
        c.extend(u32le(0x000A_0F00));
        c.extend(u32le(0x000A_0F00));
        c.extend(u32le(0)); // ntools
        c
    }

    fn encryption(cryptid: u32) -> Vec<u8> {
        let mut c = Vec::new();
        c.extend(u32le(LC_ENCRYPTION_INFO_64));
        c.extend(u32le(24));
        c.extend(u32le(0x4000));
        c.extend(u32le(0x1000));
        c.extend(u32le(cryptid));
        c.extend(u32le(0));
        c
    }

    fn macho64(cpu: u32, cmds: &[Vec<u8>]) -> Vec<u8> {
        let body: Vec<u8> = cmds.concat();
        let mut m = Vec::new();
        m.extend(u32le(0xFEED_FACF));
        m.extend(u32le(cpu));
        m.extend(u32le(3));
        m.extend(u32le(2)); // MH_EXECUTE
        m.extend(u32le(cmds.len() as u32));
        m.extend(u32le(body.len() as u32));
        m.extend(u32le(0));
        m.extend(u32le(0));
        m.extend(body);
        m
    }

    fn fat(slices: &[(u32, Vec<u8>)]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend(0xCAFE_BABEu32.to_be_bytes());
        out.extend((slices.len() as u32).to_be_bytes());
        let mut off = 8 + slices.len() * 20;
        let mut data = Vec::new();
        for (cpu, s) in slices {
            out.extend(cpu.to_be_bytes());
            out.extend(3u32.to_be_bytes());
            out.extend((off as u32).to_be_bytes());
            out.extend((s.len() as u32).to_be_bytes());
            out.extend(0u32.to_be_bytes());
            data.extend(s.clone());
            off += s.len();
        }
        out.extend(data);
        out
    }

    fn read_of(bytes: Vec<u8>) -> impl Fn(u64, usize) -> Option<Vec<u8>> {
        move |off, len| {
            let o = off as usize;
            (o < bytes.len()).then(|| bytes[o..(o + len).min(bytes.len())].to_vec())
        }
    }

    const FND: &str = "/System/Library/Frameworks/Foundation.framework/Versions/C/Foundation";
    const APPKIT: &str = "/System/Library/Frameworks/AppKit.framework/Versions/C/AppKit";
    const METAL: &str = "/System/Library/Frameworks/Metal.framework/Versions/A/Metal";
    const UIKIT: &str = "/System/Library/Frameworks/UIKit.framework/UIKit";

    #[test]
    fn a_thin_file_gives_its_cpu_and_libraries() {
        let m = macho64(CPU_X86_64, &[dylib_cmd(FND), dylib_cmd("/usr/lib/libSystem.B.dylib"), dylib_cmd("@rpath/libfoo.dylib"), build_version(1)]);
        let s = parse_macho(&read_of(m)).unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].cpu, CPU_X86_64);
        assert_eq!(s[0].dylibs, vec![FND, "/usr/lib/libSystem.B.dylib", "@rpath/libfoo.dylib"]);
        assert_eq!(s[0].platform, Some(1));
        assert!(!s[0].encrypted);
    }

    #[test]
    fn a_fat_file_gives_every_slice() {
        let a = macho64(CPU_X86_64, &[dylib_cmd(FND)]);
        let b = macho64(CPU_ARM64, &[dylib_cmd(APPKIT)]);
        let s = parse_macho(&read_of(fat(&[(CPU_X86_64, a), (CPU_ARM64, b)]))).unwrap();
        assert_eq!(s.iter().map(|x| x.cpu).collect::<Vec<_>>(), vec![CPU_X86_64, CPU_ARM64]);
        assert_eq!(s[1].dylibs, vec![APPKIT]);
    }

    #[test]
    fn other_files_are_not_mach_o() {
        assert!(parse_macho(&read_of(b"#!/bin/sh\necho hi\n".to_vec())).is_none());
        assert!(parse_macho(&read_of(vec![])).is_none());
        // a Java class file starts with the same four bytes as a fat Mach-O
        let mut java = 0xCAFE_BABEu32.to_be_bytes().to_vec();
        java.extend(0x0000_0034u32.to_be_bytes());
        java.extend(vec![0u8; 64]);
        assert!(parse_macho(&read_of(java)).is_none());
        // a truncated file must not panic
        let m = macho64(CPU_X86_64, &[dylib_cmd(FND)]);
        for cut in [4, 20, 40, m.len() - 5] {
            let _ = parse_macho(&read_of(m[..cut].to_vec()));
        }
    }

    #[test]
    fn classic_intel_app_might_run() {
        let m = macho64(CPU_X86_64, &[dylib_cmd(FND), dylib_cmd(APPKIT), dylib_cmd("/usr/lib/libSystem.B.dylib"), build_version(1)]);
        let (v, reasons, fw, archs) = judge(&parse_macho(&read_of(m)).unwrap(), Kind::MacApp);
        assert_eq!(v, Verdict::Might);
        assert_eq!(fw, vec!["AppKit", "Foundation"]);
        assert_eq!(archs, vec!["x86_64"]);
        assert!(reasons.iter().any(|r| r.contains("not measured")));
    }

    #[test]
    fn metal_and_swift_make_it_unlikely() {
        let m = macho64(CPU_X86_64, &[dylib_cmd(FND), dylib_cmd(METAL)]);
        let (v, reasons, ..) = judge(&parse_macho(&read_of(m)).unwrap(), Kind::MacApp);
        assert_eq!(v, Verdict::Unlikely);
        assert!(reasons[0].contains("Metal"));
        let sw = macho64(CPU_X86_64, &[dylib_cmd(FND), dylib_cmd("/usr/lib/swift/libswiftCore.dylib")]);
        assert_eq!(judge(&parse_macho(&read_of(sw)).unwrap(), Kind::MacApp).0, Verdict::Unlikely);
    }

    #[test]
    fn apple_silicon_only_cannot_run() {
        let m = macho64(CPU_ARM64, &[dylib_cmd(FND)]);
        let (v, reasons, ..) = judge(&parse_macho(&read_of(m)).unwrap(), Kind::MacApp);
        assert_eq!(v, Verdict::Cannot);
        assert!(reasons[0].contains("arm64"));
        // a universal file with an Intel slice is fine
        let u = fat(&[(CPU_X86_64, macho64(CPU_X86_64, &[dylib_cmd(FND)])), (CPU_ARM64, macho64(CPU_ARM64, &[dylib_cmd(FND)]))]);
        assert_eq!(judge(&parse_macho(&read_of(u)).unwrap(), Kind::MacApp).0, Verdict::Might);
    }

    #[test]
    fn iphone_apps_cannot_run_and_encryption_is_named() {
        let m = macho64(CPU_ARM64, &[dylib_cmd(UIKIT), dylib_cmd(FND), build_version(2), encryption(1)]);
        let (v, reasons, ..) = judge(&parse_macho(&read_of(m)).unwrap(), Kind::IosApp);
        assert_eq!(v, Verdict::Cannot);
        assert!(reasons[0].contains("iPhone"));
        assert!(reasons.iter().any(|r| r.contains("encrypted")));
        // UIKit alone is enough, even on a file that claims macOS
        let m = macho64(CPU_X86_64, &[dylib_cmd(UIKIT)]);
        assert_eq!(judge(&parse_macho(&read_of(m)).unwrap(), Kind::MacApp).0, Verdict::Cannot);
        // an unencrypted one says nothing about encryption
        let m = macho64(CPU_ARM64, &[dylib_cmd(UIKIT), build_version(2), encryption(0)]);
        assert!(!judge(&parse_macho(&read_of(m)).unwrap(), Kind::IosApp).1.iter().any(|r| r.contains("encrypted")));
    }

    #[test]
    fn xml_plist_strings_are_read() {
        let x = "<?xml version=\"1.0\"?><plist><dict><key>CFBundleName</key><string>Hello &amp; Co</string>\n<key>CFBundleExecutable</key>\n  <string>hello</string><key>LSUIElement</key><true/></dict></plist>";
        let p = plist_strings(x.as_bytes());
        assert_eq!(p.get("CFBundleName").unwrap(), "Hello & Co");
        assert_eq!(p.get("CFBundleExecutable").unwrap(), "hello");
        assert!(!p.contains_key("LSUIElement"));
        assert!(plist_strings(b"garbage").is_empty());
    }

    /// The marker of an ASCII string: the length is in the marker below 15, otherwise in a following one-byte integer.
    fn bp_str_head(len: usize) -> Vec<u8> {
        if len < 15 {
            vec![0x50 | len as u8]
        } else {
            vec![0x5F, 0x10, len as u8]
        }
    }

    /// A small binary plist (`bplist00`): a dict of ASCII string keys and values, one-byte offsets and references.
    fn bplist(pairs: &[(&str, &str)]) -> Vec<u8> {
        let mut objs: Vec<Vec<u8>> = Vec::new();
        let n = pairs.len();
        // object 0 is the dict; then keys, then values
        let mut dict = vec![0xD0 | n as u8];
        for i in 0..n {
            dict.push((1 + i) as u8);
        }
        for i in 0..n {
            dict.push((1 + n + i) as u8);
        }
        objs.push(dict);
        for (k, _) in pairs {
            let mut o = bp_str_head(k.len());
            o.extend(k.as_bytes());
            objs.push(o);
        }
        for (_, v) in pairs {
            let mut o = bp_str_head(v.len());
            o.extend(v.as_bytes());
            objs.push(o);
        }
        let mut out = b"bplist00".to_vec();
        let mut offs = Vec::new();
        for o in &objs {
            offs.push(out.len() as u8);
            out.extend(o);
        }
        let table = out.len();
        out.extend(offs);
        let mut trailer = vec![0u8; 6];
        trailer.push(1); // offset size
        trailer.push(1); // ref size
        trailer.extend((objs.len() as u64).to_be_bytes());
        trailer.extend(0u64.to_be_bytes()); // top object
        trailer.extend((table as u64).to_be_bytes());
        out.extend(trailer);
        out
    }

    #[test]
    fn binary_plist_strings_are_read() {
        let b = bplist(&[("CFBundleName", "Hello"), ("CFBundleExecutable", "hello-bin")]);
        let p = plist_strings(&b);
        assert_eq!(p.get("CFBundleName").unwrap(), "Hello");
        assert_eq!(p.get("CFBundleExecutable").unwrap(), "hello-bin");
        // truncated or corrupt input never panics
        for cut in [0, 7, 12, b.len() - 3] {
            let _ = plist_strings(&b[..cut]);
        }
    }

    #[test]
    fn app_folders_and_files_are_told_apart() {
        let dir = std::env::temp_dir().join(format!("zohara-mac-check-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // a real-looking app
        let app = dir.join("Hello.app");
        std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        std::fs::write(app.join("Contents/Info.plist"), "<plist><dict><key>CFBundleName</key><string>Hello</string><key>CFBundleExecutable</key><string>hello</string></dict></plist>").unwrap();
        std::fs::write(app.join("Contents/MacOS/hello"), macho64(CPU_X86_64, &[dylib_cmd(FND), dylib_cmd(APPKIT)])).unwrap();
        let r = check(&app);
        assert_eq!((r.kind, r.verdict, r.name.as_str()), (Kind::MacApp, Verdict::Might, "Hello"));
        assert!(headline(&r).contains("might run"));
        assert!(explain(&r).contains("AppKit, Foundation"));
        // a data folder that only has the .app name (like Cities.app in the owner's download)
        let data = dir.join("Cities.app");
        std::fs::create_dir_all(data.join("Contents/Resources")).unwrap();
        let r = check(&data);
        assert_eq!(r.verdict, Verdict::Unknown);
        assert!(r.reasons[0].contains("data folder"));
        // a disk image and a text file
        std::fs::write(dir.join("a.dmg"), b"x").unwrap();
        assert_eq!(check(&dir.join("a.dmg")).kind, Kind::DiskImage);
        std::fs::write(dir.join("notes.txt"), b"hello").unwrap();
        assert_eq!(check(&dir.join("notes.txt")).kind, Kind::Other);
        // a bare Mach-O library
        std::fs::write(dir.join("libx.dylib"), macho64(CPU_X86_64, &[dylib_cmd(FND)])).unwrap();
        assert_eq!(check(&dir.join("libx.dylib")).kind, Kind::MachO);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
