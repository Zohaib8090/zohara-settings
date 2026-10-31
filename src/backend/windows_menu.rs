//! Menu entries for programs a Windows installer put into the shared Windows folders (Proton's and plain Wine's).
//!
//! An installer makes its shortcuts inside the Windows folder (Start Menu and Desktop `.lnk` files), and Proton turns off
//! Wine's own menu builder, so nothing reached the Linux app menu and search never found the program. This reads those
//! Start Menu shortcuts, works out which program each one starts, and writes a desktop entry
//! (`zohara-win-<name>.desktop`) that runs it through the same double-click handler. An entry is removed again when
//! its program is gone (uninstalled).

use super::game_launch::exec_quote;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// What a Windows shortcut (`.lnk`) points to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lnk {
    /// The Windows path of the program, for example `C:\users\x\AppData\Local\Programs\Kimi\Kimi.exe`.
    pub target: String,
    pub args: String,
}

fn u16_at(b: &[u8], at: usize) -> Option<usize> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?) as usize)
}

fn u32_at(b: &[u8], at: usize) -> Option<usize> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?) as usize)
}

fn c_string(b: &[u8], at: usize) -> Option<String> {
    let rest = b.get(at..)?;
    let end = rest.iter().position(|c| *c == 0)?;
    Some(String::from_utf8_lossy(&rest[..end]).to_string())
}

/// Reads the target and the arguments of a shortcut (the MS-SHLLINK layout). None when it is not a shortcut or has no
/// local target.
pub fn parse_lnk(b: &[u8]) -> Option<Lnk> {
    if u32_at(b, 0)? != 0x4c || b.get(4..20)? != [0x01, 0x14, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0xc0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46] {
        return None;
    }
    let flags = u32_at(b, 0x14)?;
    let (has_id_list, has_info, has_name, has_rel, has_dir, has_args, unicode) =
        (flags & 1 != 0, flags & 2 != 0, flags & 4 != 0, flags & 8 != 0, flags & 0x10 != 0, flags & 0x20 != 0, flags & 0x80 != 0);
    let mut at = 0x4c;
    if has_id_list {
        at += 2 + u16_at(b, at)?;
    }
    let mut target = String::new();
    if has_info {
        let info = at;
        let size = u32_at(b, info)?;
        let info_flags = u32_at(b, info + 8)?;
        if info_flags & 1 != 0 {
            let base = c_string(b, info + u32_at(b, info + 16)?)?;
            let suffix = c_string(b, info + u32_at(b, info + 24)?).unwrap_or_default();
            target = base + &suffix;
        }
        at += size;
    }
    let mut strings: Vec<String> = Vec::new();
    for present in [has_name, has_rel, has_dir, has_args] {
        if !present {
            continue;
        }
        let n = u16_at(b, at)?;
        at += 2;
        if unicode {
            let raw = b.get(at..at + n * 2)?;
            strings.push(String::from_utf16_lossy(&raw.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>()));
            at += n * 2;
        } else {
            strings.push(String::from_utf8_lossy(b.get(at..at + n)?).to_string());
            at += n;
        }
    }
    // Windows writes an empty string as a single NUL character
    let args = if has_args { strings.pop().unwrap_or_default().replace('\0', "").trim().to_string() } else { String::new() };
    (!target.is_empty()).then_some(Lnk { target, args })
}

/// `rel` below `base`, matching each folder and file name without caring about upper and lower case (Windows does not).
fn resolve(base: &Path, rel: &str) -> Option<PathBuf> {
    let mut cur = base.to_path_buf();
    for part in rel.split('/').filter(|p| !p.is_empty()) {
        let exact = cur.join(part);
        if exact.exists() {
            cur = exact;
            continue;
        }
        let found = std::fs::read_dir(&cur).ok()?.flatten().find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(part))?;
        cur = found.path();
    }
    Some(cur)
}

/// A Windows path inside the Windows folder (`C:\...`, or `Z:\...` for the Linux root) as a Linux path.
pub fn to_unix(drive_c: &Path, win: &str) -> Option<PathBuf> {
    let w = win.replace('\\', "/");
    let lower = w.to_lowercase();
    if lower.starts_with("c:/") {
        resolve(drive_c, &w[3..])
    } else if lower.starts_with("z:/") {
        Some(PathBuf::from(&w[2..]))
    } else {
        None
    }
}

/// Start Menu folders that never hold a program the person installed.
const SKIP_FOLDERS: [&str; 6] = ["startup", "administrative tools", "accessories", "system tools", "wine", "windows accessories"];

/// Shortcuts that belong to a program but are not the program: its uninstaller, help and web links.
fn is_noise(name: &str) -> bool {
    let n = name.to_lowercase();
    ["uninstall", "unins", "readme", "read me", "help", "manual", "licen", "website", "documentation", "release notes"].iter().any(|w| n.contains(w))
}

fn lnk_files(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if p.is_dir() {
            if depth > 0 && !SKIP_FOLDERS.contains(&name.to_lowercase().as_str()) {
                lnk_files(&p, depth - 1, out);
            }
        } else if name.to_lowercase().ends_with(".lnk") {
            out.push(p);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    pub name: String,
    /// What to run: the program itself, or its shortcut when the shortcut passes arguments.
    pub run: PathBuf,
    pub exe: PathBuf,
}

/// The programs in one Windows folder's Start Menu (`drive_c` is its C: drive).
pub fn scan(drive_c: &Path) -> Vec<Program> {
    let mut menus: Vec<PathBuf> = vec![drive_c.join("ProgramData/Microsoft/Windows/Start Menu/Programs")];
    if let Ok(users) = std::fs::read_dir(drive_c.join("users")) {
        for u in users.flatten() {
            menus.push(u.path().join("AppData/Roaming/Microsoft/Windows/Start Menu/Programs"));
        }
    }
    let mut lnks = Vec::new();
    for m in menus {
        lnk_files(&m, 3, &mut lnks);
    }
    lnks.sort();
    let mut out: Vec<Program> = Vec::new();
    for l in lnks {
        let name = l.file_stem().map(|s| s.to_string_lossy().trim().to_string()).unwrap_or_default();
        if name.is_empty() || is_noise(&name) {
            continue;
        }
        let Some(lnk) = std::fs::read(&l).ok().and_then(|b| parse_lnk(&b)) else { continue };
        if !lnk.target.to_lowercase().ends_with(".exe") {
            continue;
        }
        let Some(exe) = to_unix(drive_c, &lnk.target).filter(|p| p.is_file()) else { continue };
        // Windows' own programs (notepad, the .NET tools) are not something that was installed
        if exe.starts_with(drive_c.join("windows")) || exe.to_string_lossy().to_lowercase().contains("/windows/system32/") {
            continue;
        }
        if out.iter().any(|p| p.exe == exe) {
            continue;
        }
        let run = if lnk.args.trim().is_empty() { exe.clone() } else { l.clone() };
        out.push(Program { name, run, exe });
    }
    out
}

fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

const MARK: &str = "X-Zohara-Windows-Exe=";

fn entry_text(p: &Program, me: &str, icon: &str) -> String {
    let clean = |t: &str| t.replace(['\n', '\r'], " ");
    format!(
        "[Desktop Entry]\nType=Application\nName={}\nComment=Windows program (runs with Proton-GE or Wine)\nExec={} --run-windows {}\nIcon={}\nTerminal=false\nCategories=Utility;\n{}{}\n",
        clean(&p.name),
        exec_quote(me),
        exec_quote(&p.run.to_string_lossy()),
        icon,
        MARK,
        clean(&p.exe.to_string_lossy())
    )
}

/// Makes the menu entries for every program found in the Windows folders under `data`, and removes the entries whose
/// program is gone. `skip` are program files that already have an entry of their own (games added in My games).
/// Returns how many entries were added or removed.
pub fn sync_in(data: &Path, me: &str, skip: &[String]) -> usize {
    let apps = data.join("applications");
    let mut changed = 0;
    // entries whose program was uninstalled
    if let Ok(rd) = std::fs::read_dir(&apps) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.starts_with("zohara-win-") || !name.ends_with(".desktop") {
                continue;
            }
            let exe = std::fs::read_to_string(e.path()).ok().and_then(|t| t.lines().find_map(|l| l.strip_prefix(MARK).map(str::to_string)));
            if exe.map(|x| !Path::new(&x).exists()).unwrap_or(false) && std::fs::remove_file(e.path()).is_ok() {
                changed += 1;
            }
        }
    }
    let mut programs = scan(&data.join("zohara/windows/pfx/drive_c"));
    programs.extend(scan(&data.join("zohara/windows-wine/drive_c")));
    for p in programs {
        let id = slug(&p.name);
        if id.is_empty() || skip.iter().any(|s| Path::new(s) == p.exe) {
            continue;
        }
        let file = apps.join(format!("zohara-win-{id}.desktop"));
        if file.exists() {
            continue;
        }
        let png = data.join("zohara/icons").join(format!("win-{id}.png"));
        let icon = match super::pe_icon::extract_png(&p.exe, &png) {
            Ok(()) => png.to_string_lossy().to_string(),
            Err(_) => "application-x-ms-dos-executable".to_string(),
        };
        if std::fs::create_dir_all(&apps).is_ok() && std::fs::write(&file, entry_text(&p, me, &icon)).is_ok() {
            changed += 1;
        }
    }
    changed
}

fn data_home() -> PathBuf {
    std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share"))
}

/// The real thing: the user's own folders, and the games already added in My games are left alone.
pub fn sync() -> usize {
    let Ok(me) = std::env::current_exe() else { return 0 };
    let data = data_home();
    let skip: Vec<String> = super::windows_apps::known_games(&data.join("applications")).into_iter().map(|g| g.path).collect();
    let n = sync_in(&data, &me.to_string_lossy(), &skip);
    if n > 0 {
        let _ = Command::new("update-desktop-database").arg(data.join("applications")).status();
        let _ = Command::new("kbuildsycoca6").stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
    n
}

/// `zohara-settings --sync-windows-menu [--wait PID]`: after the installer (PID) has ended, add the new programs to the
/// menu. A second look a little later catches an installer that finished writing its shortcuts after it closed.
pub fn run_after(pid: Option<u32>) -> i32 {
    if let Some(pid) = pid {
        let proc = PathBuf::from(format!("/proc/{pid}"));
        let started = std::time::Instant::now();
        while proc.exists() && started.elapsed().as_secs() < 4 * 3600 {
            std::thread::sleep(std::time::Duration::from_secs(3));
        }
    }
    sync();
    std::thread::sleep(std::time::Duration::from_secs(20));
    sync();
    0
}

/// Starts the watcher for a program that was just started (the installer), in its own process.
pub fn watch(pid: u32) {
    if let Ok(me) = std::env::current_exe() {
        let _ = Command::new(me)
            .args(["--sync-windows-menu", "--wait", &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A shortcut like the one the Kimi installer made: a local target, no arguments.
    fn make_lnk(target: &str, args: Option<&str>) -> Vec<u8> {
        let mut b = vec![0u8; 0x4c];
        b[0] = 0x4c;
        b[4..20].copy_from_slice(&[0x01, 0x14, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0xc0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46]);
        let mut flags: u32 = 2 | 0x80; // LinkInfo + unicode strings
        if args.is_some() {
            flags |= 0x20;
        }
        b[0x14..0x18].copy_from_slice(&flags.to_le_bytes());
        // LinkInfo: header 0x1c, then the local base path
        let base = [target.as_bytes(), &[0]].concat();
        let suffix = [0u8];
        let size = 0x1c + 4 + base.len() + suffix.len();
        let mut info = vec![0u8; 0x1c];
        info[0..4].copy_from_slice(&(size as u32).to_le_bytes());
        info[4..8].copy_from_slice(&0x1cu32.to_le_bytes());
        info[8..12].copy_from_slice(&1u32.to_le_bytes());
        info[12..16].copy_from_slice(&0x1cu32.to_le_bytes()); // volume id (a dummy 4 bytes)
        info[16..20].copy_from_slice(&((0x1c + 4) as u32).to_le_bytes());
        info[24..28].copy_from_slice(&((0x1c + 4 + base.len()) as u32).to_le_bytes());
        info.extend_from_slice(&[0, 0, 0, 0]);
        info.extend_from_slice(&base);
        info.extend_from_slice(&suffix);
        b.extend_from_slice(&info);
        if let Some(a) = args {
            let units: Vec<u16> = a.encode_utf16().collect();
            b.extend_from_slice(&(units.len() as u16).to_le_bytes());
            for u in units {
                b.extend_from_slice(&u.to_le_bytes());
            }
        }
        b
    }

    #[test]
    fn a_shortcut_gives_its_target_and_arguments() {
        let l = parse_lnk(&make_lnk(r"C:\users\steamuser\AppData\Local\Programs\Kimi\Kimi.exe", None)).unwrap();
        assert_eq!(l.target, r"C:\users\steamuser\AppData\Local\Programs\Kimi\Kimi.exe");
        assert_eq!(l.args, "");
        let l = parse_lnk(&make_lnk(r"C:\Games\Tool.exe", Some("--fast"))).unwrap();
        assert_eq!((l.target.as_str(), l.args.as_str()), (r"C:\Games\Tool.exe", "--fast"));
        // an empty argument string the way Windows writes it (one NUL character)
        let mut real = make_lnk(r"C:\Games\Tool.exe", Some("\0"));
        assert_eq!(parse_lnk(&real).unwrap().args, "");
        real.clear();
        assert!(parse_lnk(b"not a shortcut").is_none());
        assert!(parse_lnk(&[]).is_none());
    }

    #[test]
    fn windows_paths_become_linux_paths_whatever_the_case() {
        let base = std::env::temp_dir().join(format!("zs-winmenu-path-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("users/steamuser/AppData/Local/Programs/Kimi")).unwrap();
        std::fs::write(base.join("users/steamuser/AppData/Local/Programs/Kimi/Kimi.exe"), b"x").unwrap();
        let got = to_unix(&base, r"C:\Users\SteamUser\appdata\local\programs\Kimi\KIMI.EXE").unwrap();
        assert_eq!(got, base.join("users/steamuser/AppData/Local/Programs/Kimi/Kimi.exe"));
        assert_eq!(to_unix(&base, r"Z:\usr\bin\wine"), Some(PathBuf::from("/usr/bin/wine")));
        assert!(to_unix(&base, r"D:\x.exe").is_none());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn an_installed_program_gets_a_menu_entry_and_loses_it_when_uninstalled() {
        let data = std::env::temp_dir().join(format!("zs-winmenu-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        let c = data.join("zohara/windows/pfx/drive_c");
        let exe = c.join("users/steamuser/AppData/Local/Programs/Kimi/Kimi.exe");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, b"MZ").unwrap();
        let menu = c.join("users/steamuser/AppData/Roaming/Microsoft/Windows/Start Menu/Programs");
        std::fs::create_dir_all(&menu).unwrap();
        std::fs::write(menu.join("Kimi.lnk"), make_lnk(r"C:\users\steamuser\AppData\Local\Programs\Kimi\Kimi.exe", None)).unwrap();
        std::fs::write(menu.join("Uninstall Kimi.lnk"), make_lnk(r"C:\users\steamuser\AppData\Local\Programs\Kimi\Kimi.exe", None)).unwrap();
        std::fs::create_dir_all(menu.join("StartUp")).unwrap();
        std::fs::write(menu.join("StartUp/Other.lnk"), make_lnk(r"C:\users\steamuser\AppData\Local\Programs\Kimi\Kimi.exe", None)).unwrap();

        assert_eq!(scan(&c).iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), vec!["Kimi"]);
        assert_eq!(sync_in(&data, "/usr/bin/zohara-settings", &[]), 1);
        let entry = std::fs::read_to_string(data.join("applications/zohara-win-kimi.desktop")).unwrap();
        assert!(entry.contains("Name=Kimi\n"));
        assert!(entry.contains(&format!("--run-windows \"{}\"", exe.to_string_lossy())), "{entry}");
        // a second look changes nothing
        assert_eq!(sync_in(&data, "/usr/bin/zohara-settings", &[]), 0);
        // a game added in My games is not listed twice
        std::fs::remove_file(data.join("applications/zohara-win-kimi.desktop")).unwrap();
        assert_eq!(sync_in(&data, "/usr/bin/zohara-settings", &[exe.to_string_lossy().to_string()]), 0);
        // uninstalled: the entry goes
        assert_eq!(sync_in(&data, "/usr/bin/zohara-settings", &[]), 1);
        std::fs::remove_file(&exe).unwrap();
        assert_eq!(sync_in(&data, "/usr/bin/zohara-settings", &[]), 1);
        assert!(!data.join("applications/zohara-win-kimi.desktop").exists());
        let _ = std::fs::remove_dir_all(&data);
    }
}
