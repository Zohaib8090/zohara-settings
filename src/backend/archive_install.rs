//! Install an app from an archive (`.tar.gz`, `.tar.xz`, `.tar.bz2`, `.tar.zst`).
//!
//! An archive is not an installer, so the first job is to look inside (without extracting) and say what it is:
//! * **a ready-to-run app** (a program with the "executable" flag, such as Firefox or Blender): extracted to its own folder under
//!   `~/.local/share/zohara/archive-apps/<name>/`, with a Start menu entry, and removable again;
//! * **source code** (has `configure`, `CMakeLists.txt`, `Cargo.toml` ...): not installed here, because a hand-built program is
//!   invisible to pacman and cannot be removed cleanly; the person is pointed to the Store or the AUR;
//! * **an Arch recipe** (`PKGBUILD`) or **an Arch package** (`.pkg.tar.zst`): handled by the package tools, not by this;
//! * **anything else**: ordinary files.
//!
//! Safety: the listing is checked before anything is written. Absolute paths, `..`, links that point outside the folder, and
//! absurd sizes refuse the archive. Nothing is run during install, and no part of the system folders is touched.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    PortableApp,
    SourceCode,
    BuildRecipe,
    PacmanPackage,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Dir,
    Symlink,
    HardLink,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    pub kind: EntryKind,
    /// Any "execute" permission bit is set.
    pub executable: bool,
    /// Where a link points (symbolic or hard).
    pub link: Option<String>,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inspection {
    pub kind: Kind,
    /// A name for the Start menu, made from the archive's name.
    pub suggested_name: String,
    /// The one folder everything sits in, when there is one (stripped on install).
    pub top_dir: Option<String>,
    /// Programs that can start the app, best guess first, as paths inside the app folder (after the top folder is stripped).
    pub candidates: Vec<String>,
    /// An icon image inside the app, when one looks right.
    pub icon: Option<String>,
    pub total_size: u64,
    pub entries: usize,
    /// Why the archive must not be unpacked, when it must not.
    pub problem: Option<String>,
}

/// Biggest archive content accepted: a game launcher can be several GB, nothing legitimate here is bigger than this.
pub const MAX_TOTAL_BYTES: u64 = 16 * 1024 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 400_000;

// ── Reading the listing ─────────────────────────────────────────────────────────────────────────────────────────

/// The first `n` whitespace-separated fields of `line` and the rest of it (with its own spaces kept).
fn split_fields(line: &str, n: usize) -> Option<(Vec<&str>, &str)> {
    let mut fields = Vec::with_capacity(n);
    let mut rest = line.trim_start();
    for _ in 0..n {
        let end = rest.find(char::is_whitespace)?;
        fields.push(&rest[..end]);
        rest = rest[end..].trim_start();
    }
    Some((fields, rest))
}

/// One line of `tar -tvf`: `-rwxr-xr-x user/group 1234 2026-10-19 12:34 dir/file` (links add ` -> target` or ` link to target`).
pub fn parse_listing_line(line: &str) -> Option<Entry> {
    let (f, rest) = split_fields(line, 5)?;
    let mode = f[0];
    let first = mode.chars().next()?;
    let kind = match first {
        '-' => EntryKind::File,
        'd' => EntryKind::Dir,
        'l' => EntryKind::Symlink,
        'h' => EntryKind::HardLink,
        _ => EntryKind::Other,
    };
    let executable = mode.chars().skip(1).enumerate().any(|(i, c)| matches!(i, 2 | 5 | 8) && matches!(c, 'x' | 's' | 't'));
    let size = f[2].parse().unwrap_or(0);
    let (path, link) = match kind {
        EntryKind::Symlink => match rest.split_once(" -> ") {
            Some((p, t)) => (p, Some(t.to_string())),
            None => (rest, None),
        },
        EntryKind::HardLink => match rest.split_once(" link to ") {
            Some((p, t)) => (p, Some(t.to_string())),
            None => (rest, None),
        },
        _ => (rest, None),
    };
    let path = path.trim_end_matches('/').to_string();
    if path.is_empty() {
        return None;
    }
    Some(Entry { path, kind, executable, link, size })
}

pub fn parse_listing(text: &str) -> Vec<Entry> {
    text.lines().filter_map(parse_listing_line).collect()
}

/// `tar -tvf`: works for every compression `tar` can read (gzip, xz, bzip2, zstd).
pub fn list_archive(path: &Path) -> Result<Vec<Entry>, String> {
    let out = Command::new("tar")
        .arg("-tvf")
        .arg(path)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("tar could not be started ({e})"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let last = err.lines().map(str::trim).filter(|l| !l.is_empty()).last().unwrap_or("it could not be read");
        return Err(format!("This is not a readable archive: {last}"));
    }
    Ok(parse_listing(&String::from_utf8_lossy(&out.stdout)))
}

// ── Safety ──────────────────────────────────────────────────────────────────────────────────────────────────────

fn norm(path: &str) -> &str {
    let mut p = path;
    while let Some(r) = p.strip_prefix("./") {
        p = r;
    }
    p
}

/// A link inside `link_path` pointing at `target` would end up outside the folder the archive is unpacked into.
pub fn escapes_root(link_path: &str, target: &str) -> bool {
    if target.starts_with('/') {
        return true;
    }
    let mut depth: i64 = norm(link_path).split('/').filter(|c| !c.is_empty()).count() as i64 - 1;
    for c in target.split('/') {
        match c {
            "" | "." => {}
            ".." => {
                depth -= 1;
                if depth < 0 {
                    return true;
                }
            }
            _ => depth += 1,
        }
    }
    false
}

/// A reason the archive must not be unpacked, or `None`.
pub fn unsafe_reason(entries: &[Entry]) -> Option<String> {
    if entries.is_empty() {
        return Some("The archive is empty.".into());
    }
    if entries.len() > MAX_ENTRIES {
        return Some("The archive holds an unreasonable number of files.".into());
    }
    if entries.iter().map(|e| e.size).sum::<u64>() > MAX_TOTAL_BYTES {
        return Some("The archive is larger than 16 GB when unpacked.".into());
    }
    for e in entries {
        let p = norm(&e.path);
        if p.starts_with('/') || p.split('/').any(|c| c == "..") {
            return Some(format!("The archive tries to write outside its own folder ({}).", e.path));
        }
        if let Some(t) = &e.link {
            let escapes = match e.kind {
                EntryKind::Symlink => escapes_root(p, t),
                // a hard link target is a path inside the archive
                _ => t.starts_with('/') || t.split('/').any(|c| c == ".."),
            };
            if escapes {
                return Some(format!("The archive contains a link that points outside its own folder ({}).", e.path));
            }
        }
    }
    None
}

// ── What is it? ─────────────────────────────────────────────────────────────────────────────────────────────────

/// The single folder all entries sit in, if there is exactly one.
pub fn common_top(entries: &[Entry]) -> Option<String> {
    let mut top: Option<&str> = None;
    let mut deeper = false;
    for e in entries {
        let p = norm(&e.path);
        let first = p.split('/').next().unwrap_or("");
        if first.is_empty() || first == "." {
            continue;
        }
        match top {
            None => top = Some(first),
            Some(t) if t == first => {}
            Some(_) => return None,
        }
        if p.contains('/') || e.kind == EntryKind::Dir {
            deeper = true;
        }
    }
    let t = top?;
    // A single file at the top is not a folder.
    let single_file = entries.iter().all(|e| norm(&e.path) == t && e.kind != EntryKind::Dir);
    (deeper && !single_file).then(|| t.to_string())
}

fn strip_top<'a>(path: &'a str, top: &Option<String>) -> &'a str {
    let p = norm(path);
    match top {
        Some(t) => p.strip_prefix(t.as_str()).map(|r| r.trim_start_matches('/')).unwrap_or(p),
        None => p,
    }
}

const ARCHIVE_SUFFIXES: &[&str] = &[".tar.gz", ".tar.xz", ".tar.bz2", ".tar.zst", ".tar.lz", ".tar.lzma", ".tgz", ".txz", ".tbz2", ".tbz", ".tar"];

fn without_archive_suffix(name: &str) -> &str {
    let low = name.to_lowercase();
    for s in ARCHIVE_SUFFIXES {
        if low.ends_with(s) {
            return &name[..name.len() - s.len()];
        }
    }
    name
}

/// "blender-4.2.0-linux-x64.tar.xz" becomes "Blender".
pub fn suggest_name(file_name: &str, top_dir: Option<&str>) -> String {
    let base = top_dir.map(str::to_string).unwrap_or_else(|| without_archive_suffix(file_name).to_string());
    const NOISE: &[&str] = &[
        "linux", "x86", "x86_64", "x64", "amd64", "x8664", "64bit", "64", "32bit", "portable", "bin", "binary", "release", "stable", "generic", "gnu", "glibc", "unknown", "static", "dist",
    ];
    let words: Vec<&str> = base
        .split(|c| c == '-' || c == '_' || c == ' ')
        .filter(|w| !w.is_empty())
        .filter(|w| {
            let l = w.to_lowercase();
            let looks_like_version = l.starts_with(|c: char| c.is_ascii_digit()) || (l.starts_with('v') && l[1..].starts_with(|c: char| c.is_ascii_digit()));
            !looks_like_version && !NOISE.contains(&l.as_str())
        })
        .collect();
    let name = if words.is_empty() { base.replace(['-', '_'], " ") } else { words.join(" ") };
    let mut chars = name.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => "App".to_string(),
    }
}

/// A short, safe folder and file name: lowercase letters, digits and dashes.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "app".into()
    } else {
        out
    }
}

const BUILD_FILES: &[&str] = &["configure", "configure.ac", "autogen.sh", "CMakeLists.txt", "meson.build", "Cargo.toml", "Makefile", "makefile", "setup.py", "pyproject.toml", "go.mod"];
const SOURCE_EXT: &[&str] = &[".c", ".cc", ".cpp", ".cxx", ".h", ".hpp", ".rs", ".go", ".m", ".zig"];
const NOT_PROGRAMS: &[&str] = &["install", "uninstall", "update", "updater", "setup", "configure", "autogen", "build", "test", "crashpad", "sandbox", "chrome-sandbox", "postinst", "prerm"];

fn is_library_like(name: &str) -> bool {
    let l = name.to_lowercase();
    l.contains(".so") || l.ends_with(".dll") || l.ends_with(".dylib") || l.ends_with(".a") || l.ends_with(".o") || l.ends_with(".py") || l.ends_with(".pl")
}

/// Rank the executables: the one named like the app, at the top, or in `bin/`, comes first.
fn rank_candidates(entries: &[Entry], top: &Option<String>, app_name: &str) -> Vec<String> {
    let want = slug(app_name).replace('-', "");
    let mut scored: Vec<(i32, String)> = Vec::new();
    for e in entries {
        if e.kind != EntryKind::File || !e.executable {
            continue;
        }
        let rel = strip_top(&e.path, top);
        if rel.is_empty() {
            continue;
        }
        let parts: Vec<&str> = rel.split('/').collect();
        if parts.len() > 3 {
            continue;
        }
        let file = parts[parts.len() - 1];
        if is_library_like(file) {
            continue;
        }
        let fl = file.to_lowercase();
        let stem = fl.trim_end_matches(".sh").trim_end_matches(".appimage").to_string();
        if NOT_PROGRAMS.iter().any(|n| stem.contains(n)) {
            continue;
        }
        let compact = stem.replace(['-', '_', ' '], "");
        let mut score = 0;
        if !want.is_empty() && compact == want {
            score += 60;
        } else if !want.is_empty() && (compact.contains(&want) || want.contains(&compact)) && compact.len() >= 3 {
            score += 35;
        }
        score += match parts.len() {
            1 => 25,
            _ if parts[0] == "bin" => 20,
            _ => 5,
        };
        if fl.ends_with(".appimage") {
            score += 30;
        }
        if fl.ends_with(".sh") {
            score -= 10;
        }
        scored.push((score, rel.to_string()));
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.len().cmp(&b.1.len())).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, p)| p).collect()
}

fn pick_icon(entries: &[Entry], top: &Option<String>, app_name: &str) -> Option<String> {
    let want = slug(app_name).replace('-', "");
    let mut best: Option<(i32, String)> = None;
    for e in entries {
        if e.kind != EntryKind::File {
            continue;
        }
        let rel = strip_top(&e.path, top);
        let l = rel.to_lowercase();
        let ext_score = if l.ends_with(".svg") {
            30
        } else if l.ends_with(".png") {
            20
        } else {
            continue;
        };
        let depth = rel.split('/').count();
        if depth > 5 {
            continue;
        }
        let file = l.rsplit('/').next().unwrap_or("");
        let stem = file.rsplit_once('.').map(|(s, _)| s).unwrap_or(file).replace(['-', '_'], "");
        let mut score = ext_score - depth as i32;
        if !want.is_empty() && stem == want {
            score += 40;
        } else if stem == "icon" || stem == "logo" || stem.contains("icon") && depth <= 2 {
            score += 25;
        } else if !want.is_empty() && stem.contains(&want) {
            score += 15;
        } else {
            continue;
        }
        // bigger pictures first (a 512 px icon over a 16 px one), by file size
        score += (e.size / 4096).min(20) as i32;
        if best.as_ref().map(|(s, _)| score > *s).unwrap_or(true) {
            best = Some((score, rel.to_string()));
        }
    }
    best.map(|(_, p)| p)
}

pub fn classify(file_name: &str, entries: &[Entry]) -> Inspection {
    let problem = unsafe_reason(entries);
    let total_size = entries.iter().map(|e| e.size).sum();
    let top_dir = common_top(entries);
    let suggested_name = suggest_name(file_name, top_dir.as_deref());
    let low = file_name.to_lowercase();
    let mut inspection = Inspection { kind: Kind::Other, suggested_name, top_dir: top_dir.clone(), candidates: Vec::new(), icon: None, total_size, entries: entries.len(), problem };

    let rels: Vec<&str> = entries.iter().map(|e| strip_top(&e.path, &top_dir)).collect();
    let root: BTreeSet<&str> = rels.iter().filter(|r| !r.contains('/') && !r.is_empty()).copied().collect();

    if low.contains(".pkg.tar") || rels.contains(&".PKGINFO") {
        inspection.kind = Kind::PacmanPackage;
        return inspection;
    }
    if root.contains("PKGBUILD") {
        inspection.kind = Kind::BuildRecipe;
        return inspection;
    }
    let candidates = rank_candidates(entries, &top_dir, &inspection.suggested_name);
    let has_build_file = BUILD_FILES.iter().any(|b| root.contains(b));
    let source_files = rels.iter().filter(|r| SOURCE_EXT.iter().any(|x| r.to_lowercase().ends_with(x))).count();
    if has_build_file && (source_files >= 5 || candidates.is_empty()) {
        inspection.kind = Kind::SourceCode;
        return inspection;
    }
    if !candidates.is_empty() {
        inspection.kind = Kind::PortableApp;
        inspection.icon = pick_icon(entries, &top_dir, &inspection.suggested_name);
        inspection.candidates = candidates;
    }
    inspection
}

pub fn inspect(path: &Path) -> Result<Inspection, String> {
    let entries = list_archive(path)?;
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    Ok(classify(&name, &entries))
}

/// What to tell the person in plain words, for kinds that are not installed here.
pub fn explain(i: &Inspection) -> String {
    match i.kind {
        Kind::PortableApp => format!("This is a ready-to-run app. Zohara can unpack it into its own folder, add {} to the Start menu, and remove it again later.", i.suggested_name),
        Kind::SourceCode => "This is source code, not a program: it has to be built first, and a hand-built program is invisible to the package manager, so it cannot be updated or removed cleanly. Look for the app in the Store (Flathub or the AUR) instead.".into(),
        Kind::BuildRecipe => "This is an Arch build recipe (a PKGBUILD). The AUR section of the Store builds and installs these properly, and the package manager keeps track of them.".into(),
        Kind::PacmanPackage => "This is an Arch package. Installing one by hand skips the update checks; the Store installs the same program from the repositories.".into(),
        Kind::Other => "This looks like ordinary files, not an app (no program inside to start). Open it with the archive manager to see what is in it.".into(),
    }
}

// ── Install, list, remove ───────────────────────────────────────────────────────────────────────────────────────

/// Where apps and menu entries go. A struct so tests use a throwaway folder.
#[derive(Clone, Debug)]
pub struct Dirs {
    /// `~/.local/share/zohara/archive-apps`
    pub apps: PathBuf,
    /// `~/.local/share/applications`
    pub menu: PathBuf,
}

impl Dirs {
    pub fn real() -> Dirs {
        let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
        let data = std::env::var("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|_| home.join(".local/share"));
        Dirs { apps: data.join("zohara/archive-apps"), menu: data.join("applications") }
    }
}

const META: &str = ".zohara-archive.json";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    pub name: String,
    pub slug: String,
    pub dir: PathBuf,
    pub exec: PathBuf,
    pub desktop_file: PathBuf,
}

/// A value for the `Exec=` line of a desktop file: quoted, with the characters the format treats specially escaped.
pub fn exec_quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' | '`' | '$' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            '%' => out.push_str("%%"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

pub fn desktop_text(name: &str, exec: &Path, work_dir: &Path, icon: &str) -> String {
    let clean = |s: &str| s.replace(['\n', '\r'], " ");
    format!(
        "[Desktop Entry]\nType=Application\nName={}\nComment=Installed from an archive with Zohara\nExec={}\nPath={}\nIcon={}\nTerminal=false\nCategories=Utility;\nX-Zohara-ArchiveApp=true\n",
        clean(name),
        exec_quote(&exec.to_string_lossy()),
        clean(&work_dir.to_string_lossy()),
        clean(icon)
    )
}

/// The start of a file tells if it can be started: a program (ELF), a script (`#!`), or an AppImage (an ELF).
fn looks_startable(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 4];
    match std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut head)) {
        Ok(()) => head == [0x7f, b'E', b'L', b'F'] || head[..2] == *b"#!",
        Err(_) => false,
    }
}

pub fn install(archive: &Path, name: &str, exec_rel: &str, dirs: &Dirs) -> Result<Installed, String> {
    let info = inspect(archive)?;
    if let Some(p) = &info.problem {
        return Err(p.clone());
    }
    if info.kind != Kind::PortableApp {
        return Err(explain(&info));
    }
    if !info.candidates.iter().any(|c| c == exec_rel) {
        return Err("That program is not in the archive.".into());
    }
    let name = name.trim();
    if name.is_empty() {
        return Err("Give the app a name.".into());
    }
    let slug = slug(name);
    let dest = dirs.apps.join(&slug);
    let meta = dest.join(META);
    if dest.exists() && !meta.exists() {
        return Err(format!("There is already a folder called {slug} that Zohara did not create; choose another name."));
    }
    std::fs::create_dir_all(&dirs.apps).map_err(|e| format!("Could not create the apps folder: {e}"))?;
    let staging = dirs.apps.join(format!(".staging-{slug}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    let mut cmd = Command::new("tar");
    cmd.arg("-xf").arg(archive).arg("-C").arg(&staging).args(["--no-same-owner", "--no-same-permissions"]);
    if info.top_dir.is_some() {
        cmd.arg("--strip-components=1");
    }
    let out = cmd.stdin(Stdio::null()).output().map_err(|e| format!("tar could not be started ({e})"))?;
    if !out.status.success() {
        let _ = std::fs::remove_dir_all(&staging);
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("Unpacking failed: {}", err.lines().last().unwrap_or("unknown error")));
    }
    let exec_in_staging = staging.join(exec_rel);
    if !looks_startable(&exec_in_staging) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(format!("{exec_rel} is not a program that can be started."));
    }
    // Keep the executable bit even if the umask removed it.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(m) = std::fs::metadata(&exec_in_staging) {
            let mut p = m.permissions();
            p.set_mode(p.mode() | 0o755);
            let _ = std::fs::set_permissions(&exec_in_staging, p);
        }
    }
    if dest.exists() {
        std::fs::remove_dir_all(&dest).map_err(|e| format!("Could not replace the old copy: {e}"))?;
    }
    std::fs::rename(&staging, &dest).map_err(|e| format!("Could not finish installing: {e}"))?;

    let exec = dest.join(exec_rel);
    let icon = info.icon.as_ref().map(|i| dest.join(i).to_string_lossy().to_string()).unwrap_or_else(|| "application-x-executable".to_string());
    let desktop_file = dirs.menu.join(format!("zohara-archive-{slug}.desktop"));
    std::fs::create_dir_all(&dirs.menu).map_err(|e| e.to_string())?;
    std::fs::write(&desktop_file, desktop_text(name, &exec, &dest, &icon)).map_err(|e| format!("Could not add it to the Start menu: {e}"))?;
    let meta_json = serde_json::json!({ "name": name, "slug": slug, "exec": exec_rel, "desktop_file": desktop_file.to_string_lossy(), "from": archive.to_string_lossy() });
    let _ = std::fs::write(&meta, meta_json.to_string());
    Ok(Installed { name: name.to_string(), slug, dir: dest, exec, desktop_file })
}

/// Apps installed from archives, newest name order.
pub fn list_installed(dirs: &Dirs) -> Vec<Installed> {
    let Ok(rd) = std::fs::read_dir(&dirs.apps) else { return Vec::new() };
    let mut out: Vec<Installed> = rd
        .flatten()
        .filter_map(|e| {
            let dir = e.path();
            let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join(META)).ok()?).ok()?;
            let slug = v["slug"].as_str()?.to_string();
            Some(Installed {
                name: v["name"].as_str()?.to_string(),
                exec: dir.join(v["exec"].as_str()?),
                desktop_file: dirs.menu.join(format!("zohara-archive-{slug}.desktop")),
                slug,
                dir,
            })
        })
        .collect();
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

/// Removes the app's folder and its Start menu entry. Only things Zohara created can be removed.
pub fn remove(slug: &str, dirs: &Dirs) -> Result<(), String> {
    if slug.is_empty() || slug.contains('/') || slug.starts_with('.') {
        return Err("Not a valid app name.".into());
    }
    let dir = dirs.apps.join(slug);
    if !dir.join(META).exists() {
        return Err("This app was not installed from an archive by Zohara.".into());
    }
    let _ = std::fs::remove_file(dirs.menu.join(format!("zohara-archive-{slug}.desktop")));
    std::fs::remove_dir_all(&dir).map_err(|e| format!("Could not remove the folder: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(path: &str, exec: bool) -> Entry {
        Entry { path: path.into(), kind: EntryKind::File, executable: exec, link: None, size: 100 }
    }
    fn d(path: &str) -> Entry {
        Entry { path: path.into(), kind: EntryKind::Dir, executable: true, link: None, size: 0 }
    }

    #[test]
    fn listing_lines_are_read() {
        let f = parse_listing_line("-rwxr-xr-x user/users 1234 2026-10-19 12:34 firefox/firefox").unwrap();
        assert_eq!((f.path.as_str(), f.kind, f.executable, f.size), ("firefox/firefox", EntryKind::File, true, 1234));
        let g = parse_listing_line("-rw-r--r-- user/users      42 2026-10-19 12:34 firefox/my file.txt").unwrap();
        assert_eq!(g.path, "firefox/my file.txt");
        assert!(!g.executable);
        let dir = parse_listing_line("drwxr-xr-x user/users 0 2026-10-19 12:34 firefox/").unwrap();
        assert_eq!((dir.path.as_str(), dir.kind), ("firefox", EntryKind::Dir));
        let l = parse_listing_line("lrwxrwxrwx user/users 0 2026-10-19 12:34 app/lib.so -> lib.so.1").unwrap();
        assert_eq!((l.kind, l.link.as_deref()), (EntryKind::Symlink, Some("lib.so.1")));
        let h = parse_listing_line("hrwxr-xr-x user/users 0 2026-10-19 12:34 app/b link to app/a").unwrap();
        assert_eq!((h.kind, h.link.as_deref()), (EntryKind::HardLink, Some("app/a")));
        assert!(parse_listing_line("garbage").is_none());
    }

    #[test]
    fn links_that_leave_the_folder_are_caught() {
        assert!(escapes_root("app/lib", "/etc/passwd"));
        assert!(escapes_root("app/lib", "../../x"));
        assert!(escapes_root("lib", "../x"));
        assert!(!escapes_root("app/lib/a.so", "../b.so"));
        assert!(!escapes_root("app/lib/a.so", "a.so.1"));
        assert!(!escapes_root("./app/bin/tool", "../lib/tool"));
    }

    #[test]
    fn unsafe_archives_are_refused() {
        assert!(unsafe_reason(&[e("/etc/passwd", false)]).is_some());
        assert!(unsafe_reason(&[e("app/../../x", false)]).is_some());
        assert!(unsafe_reason(&[]).is_some());
        let mut big = e("app/huge", false);
        big.size = MAX_TOTAL_BYTES + 1;
        assert!(unsafe_reason(&[big]).is_some());
        let mut link = e("app/l", false);
        link.kind = EntryKind::Symlink;
        link.link = Some("/usr/bin/sh".into());
        assert!(unsafe_reason(&[link]).is_some());
        assert!(unsafe_reason(&[d("app"), e("app/run", true), e("./app/readme", false)]).is_none());
    }

    #[test]
    fn names_come_from_the_archive_name() {
        assert_eq!(suggest_name("blender-4.2.0-linux-x64.tar.xz", None), "Blender");
        assert_eq!(suggest_name("firefox-130.0.tar.bz2", None), "Firefox");
        assert_eq!(suggest_name("Discord.tar.gz", Some("Discord")), "Discord");
        assert_eq!(suggest_name("my_cool_tool-v2.1_amd64.tgz", None), "My cool tool");
        assert_eq!(suggest_name("1.2.3.tar.gz", None), "1.2.3");
        assert_eq!(slug("My Cool Tool!"), "my-cool-tool");
        assert_eq!(slug("..//"), "app");
    }

    #[test]
    fn a_ready_to_run_app_is_found_with_its_main_program() {
        let entries = vec![
            d("blender-4.2"),
            e("blender-4.2/blender", true),
            e("blender-4.2/blender-launcher", true),
            e("blender-4.2/lib/libfoo.so.1", true),
            e("blender-4.2/4.2/scripts/x.py", false),
            e("blender-4.2/blender.svg", false),
            e("blender-4.2/readme.txt", false),
        ];
        let i = classify("blender-4.2.0-linux-x64.tar.xz", &entries);
        assert_eq!(i.kind, Kind::PortableApp);
        assert_eq!(i.top_dir.as_deref(), Some("blender-4.2"));
        assert_eq!(i.suggested_name, "Blender");
        assert_eq!(i.candidates[0], "blender");
        assert!(!i.candidates.iter().any(|c| c.contains("libfoo")));
        assert_eq!(i.icon.as_deref(), Some("blender.svg"));
        assert!(i.problem.is_none());
    }

    #[test]
    fn source_code_recipes_and_packages_are_told_apart() {
        let mut src = vec![d("tool-1.0"), e("tool-1.0/configure", true), e("tool-1.0/Makefile.in", false)];
        for n in 0..6 {
            src.push(e(&format!("tool-1.0/src/f{n}.c"), false));
        }
        assert_eq!(classify("tool-1.0.tar.gz", &src).kind, Kind::SourceCode);
        let recipe = vec![d("foo"), e("foo/PKGBUILD", false), e("foo/foo.install", false)];
        assert_eq!(classify("foo.tar.gz", &recipe).kind, Kind::BuildRecipe);
        let pkg = vec![e(".PKGINFO", false), e("usr/bin/x", true)];
        assert_eq!(classify("x-1-1-x86_64.pkg.tar.zst", &pkg).kind, Kind::PacmanPackage);
        let data = vec![d("photos"), e("photos/a.jpg", false), e("photos/b.jpg", false)];
        assert_eq!(classify("photos.tar.gz", &data).kind, Kind::Other);
        // an installer script alone is not an app
        let installer = vec![d("x"), e("x/install.sh", true), e("x/data.bin", false)];
        assert_eq!(classify("x.tar.gz", &installer).kind, Kind::Other);
    }

    #[test]
    fn a_single_loose_program_still_counts() {
        let i = classify("mytool-linux-x64.tar.gz", &[e("mytool", true), e("LICENSE", false)]);
        assert_eq!(i.kind, Kind::PortableApp);
        assert_eq!(i.top_dir, None);
        assert_eq!(i.candidates, ["mytool"]);
    }

    #[test]
    fn desktop_entries_are_quoted() {
        assert_eq!(exec_quote("/a b/c"), "\"/a b/c\"");
        assert_eq!(exec_quote("/a\"b$c%d"), "\"/a\\\"b\\$c%%d\"");
        let t = desktop_text("My App", Path::new("/h/My App/run"), Path::new("/h/My App"), "application-x-executable");
        assert!(t.contains("Exec=\"/h/My App/run\"\n") && t.contains("Path=/h/My App\n") && t.contains("Name=My App\n"));
        assert!(!desktop_text("Bad\nName", Path::new("/x"), Path::new("/"), "i").contains("Bad\nName"));
    }

    // ── a real archive, a real tar ──────────────────────────────────────────────────────────────────────────────

    fn scratch(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("zohara-archive-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn install_run_list_and_remove_a_real_archive() {
        use std::os::unix::fs::PermissionsExt;
        let work = scratch("real");
        let src = work.join("src/Hello-1.2");
        std::fs::create_dir_all(src.join("lib")).unwrap();
        std::fs::write(src.join("hello"), "#!/bin/sh\necho hello-from-archive-app\n").unwrap();
        std::fs::set_permissions(src.join("hello"), std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(src.join("icon.png"), vec![0u8; 20000]).unwrap();
        std::fs::write(src.join("lib/data.txt"), "x").unwrap();
        let archive = work.join("Hello-1.2-linux-x64.tar.gz");
        let st = Command::new("tar").arg("-czf").arg(&archive).arg("-C").arg(work.join("src")).arg("Hello-1.2").status().unwrap();
        assert!(st.success());

        let info = inspect(&archive).unwrap();
        assert_eq!(info.kind, Kind::PortableApp);
        assert_eq!(info.suggested_name, "Hello");
        assert_eq!(info.candidates[0], "hello");
        assert_eq!(info.icon.as_deref(), Some("icon.png"));

        let dirs = Dirs { apps: work.join("apps"), menu: work.join("menu") };
        let done = install(&archive, "Hello", "hello", &dirs).unwrap();
        assert_eq!(done.slug, "hello");
        assert!(done.exec.ends_with("apps/hello/hello"), "the top folder is stripped");
        let out = Command::new(&done.exec).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hello-from-archive-app");
        let text = std::fs::read_to_string(&done.desktop_file).unwrap();
        assert!(text.contains("Name=Hello") && text.contains("icon.png") && text.contains("X-Zohara-ArchiveApp=true"));
        assert_eq!(list_installed(&dirs).len(), 1);

        // installing again replaces; a program that is not in the archive is refused
        assert!(install(&archive, "Hello", "hello", &dirs).is_ok());
        assert!(install(&archive, "Hello", "nothing", &dirs).is_err());

        remove("hello", &dirs).unwrap();
        assert!(!done.dir.exists() && !done.desktop_file.exists());
        assert!(list_installed(&dirs).is_empty());
        assert!(remove("../x", &dirs).is_err());
        assert!(remove("hello", &dirs).is_err(), "nothing left to remove");
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn a_folder_zohara_did_not_make_is_never_replaced_or_removed() {
        let work = scratch("foreign");
        let dirs = Dirs { apps: work.join("apps"), menu: work.join("menu") };
        std::fs::create_dir_all(dirs.apps.join("keep")).unwrap();
        std::fs::write(dirs.apps.join("keep/mine.txt"), "mine").unwrap();
        assert!(remove("keep", &dirs).is_err());
        assert!(dirs.apps.join("keep/mine.txt").exists());
        let _ = std::fs::remove_dir_all(&work);
    }

    #[test]
    fn a_file_that_is_not_an_archive_says_so() {
        let work = scratch("bad");
        let f = work.join("not-an-archive.tar.gz");
        std::fs::write(&f, "this is plain text").unwrap();
        let err = inspect(&f).unwrap_err();
        assert!(err.starts_with("This is not a readable archive"), "{err}");
        let _ = std::fs::remove_dir_all(&work);
    }
}

// ── Right-click > Zohara > Install as an app ────────────────────────────────────────────────────────────────────

fn notify(title: &str, body: &str, icon: &str) {
    let _ = Command::new("notify-send").args(["-a", "Zohara", "-i", icon, title, body]).status();
}

/// `zohara-settings --install-archive FILE`: no window. A ready-to-run app is installed under its suggested name with the best
/// guess for its main program (the Apps page lets the person choose both); anything else gets a plain explanation.
pub fn install_from_file_manager(path: &str) -> i32 {
    let file = Path::new(path);
    let info = match inspect(file) {
        Ok(i) => i,
        Err(e) => {
            notify("Can't read this archive", &e, "dialog-error");
            return 1;
        }
    };
    if let Some(p) = &info.problem {
        notify("This archive was not unpacked", p, "dialog-warning");
        return 1;
    }
    if info.kind != Kind::PortableApp {
        notify("Not an app Zohara can install this way", &explain(&info), "dialog-information");
        return 1;
    }
    let Some(exec) = info.candidates.first() else { return 1 };
    match install(file, &info.suggested_name, exec, &Dirs::real()) {
        Ok(done) => {
            notify(&format!("{} is installed", done.name), "It is in the Start menu. Settings > Apps > Apps from archives removes it again.", "emblem-ok-symbolic");
            0
        }
        Err(e) => {
            notify("Couldn't install the app", &e, "dialog-error");
            1
        }
    }
}

/// "1.4 GB", "230 MB" ...
pub fn readable_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["bytes", "KB", "MB", "GB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{bytes} bytes")
    } else if v >= 100.0 {
        format!("{:.0} {}", v, UNITS[u])
    } else {
        format!("{:.1} {}", v, UNITS[u])
    }
}

#[cfg(test)]
mod size_tests {
    use super::readable_size;

    #[test]
    fn sizes_read_plainly() {
        assert_eq!(readable_size(500), "500 bytes");
        assert_eq!(readable_size(2048), "2.0 KB");
        assert_eq!(readable_size(230 * 1024 * 1024), "230 MB");
        assert_eq!(readable_size(1_500_000_000), "1.4 GB");
    }
}
