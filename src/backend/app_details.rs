//! What the app details screen shows and changes: sizes, saved data, Flatpak permissions, notifications.
//!
//! Anything that deletes is deliberately narrow: only inside `~/.var/app/<id>` (Flatpak) or a few named folders in
//! the person's cache, never a path that came from outside this file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

// ── Sizes ──────────────────────────────────────────────────────────────────

/// 1234567 -> "1.2 MB" (powers of 1000, like the sizes Flatpak prints).
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1000.0 && i < UNITS.len() - 1 {
        v /= 1000.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

/// Size of everything under `path` (links are counted as themselves, never followed).
pub fn dir_size(path: &Path) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(path) else { return 0 };
    if !meta.is_dir() {
        return meta.len();
    }
    let Ok(rd) = std::fs::read_dir(path) else { return 0 };
    rd.flatten().map(|e| dir_size(&e.path())).sum()
}

/// Removes everything inside `dir` but not `dir` itself. Links are removed as links, not followed. Returns the bytes freed.
pub fn clear_contents(dir: &Path) -> Result<u64, String> {
    let before = dir_size(dir);
    let rd = std::fs::read_dir(dir).map_err(|e| e.to_string())?;
    for entry in rd.flatten() {
        let p = entry.path();
        let meta = std::fs::symlink_metadata(&p).map_err(|e| e.to_string())?;
        let r = if meta.is_dir() { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
        r.map_err(|e| format!("Couldn't remove {} ({e})", p.display()))?;
    }
    Ok(before.saturating_sub(dir_size(dir)))
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
}

fn xdg(var: &str, fallback: &str) -> PathBuf {
    std::env::var(var).map(PathBuf::from).unwrap_or_else(|_| home().join(fallback))
}

pub fn safe_flatpak_id(id: &str) -> bool {
    !id.is_empty() && id.len() < 200 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)) && !id.starts_with('.')
}

/// `~/.var/app/<id>`: where a Flatpak app keeps its data.
pub fn flatpak_data_dir(id: &str) -> Option<PathBuf> {
    safe_flatpak_id(id).then(|| home().join(".var/app").join(id))
}

/// Folders an ordinary app may keep things in, found by the names it is known by.
/// (config and data, then cache). Only folders that exist are returned.
pub fn native_dirs(names: &[String]) -> (Vec<PathBuf>, Vec<PathBuf>) {
    native_dirs_in(
        [xdg("XDG_CONFIG_HOME", ".config"), xdg("XDG_DATA_HOME", ".local/share"), xdg("XDG_STATE_HOME", ".local/state"), xdg("XDG_CACHE_HOME", ".cache")],
        names,
    )
}

/// `native_dirs` with the four base folders given: config, data, state, cache.
pub fn native_dirs_in([config, share, state, cache]: [PathBuf; 4], names: &[String]) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let ok = |n: &String| !n.is_empty() && n.len() < 100 && !n.contains(['/', '\0']) && !n.starts_with('.');
    let mut data = Vec::new();
    let mut caches = Vec::new();
    for n in names.iter().filter(|n| ok(n)) {
        for base in [&config, &share, &state] {
            let p = base.join(n);
            if p.is_dir() && !data.contains(&p) {
                data.push(p);
            }
        }
        let c = cache.join(n);
        if c.is_dir() && !caches.contains(&c) {
            caches.push(c);
        }
    }
    (data, caches)
}

/// Total size of a list of folders.
pub fn total_size(dirs: &[PathBuf]) -> u64 {
    dirs.iter().map(|d| dir_size(d)).sum()
}

// ── Flatpak: what an app may do ────────────────────────────────────────────

/// The `[Context]` group of an app's permissions (or of the person's changes to them): each key with its list.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Context {
    pub lists: BTreeMap<String, Vec<String>>,
}

pub fn parse_context(text: &str) -> Context {
    let mut ctx = Context::default();
    let mut in_ctx = false;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_ctx = l == "[Context]";
        } else if in_ctx {
            if let Some((k, v)) = l.split_once('=') {
                ctx.lists.insert(k.trim().to_string(), v.split(';').map(str::trim).filter(|t| !t.is_empty()).map(String::from).collect());
            }
        }
    }
    ctx
}

impl Context {
    /// What is allowed once the person's changes (`over`) are applied on top of the app's own list (`self`):
    /// a `!token` removes it, any other token adds it.
    pub fn with_overrides(&self, over: &Context) -> Context {
        let mut out = self.clone();
        for (key, tokens) in &over.lists {
            let list = out.lists.entry(key.clone()).or_default();
            for t in tokens {
                if let Some(removed) = t.strip_prefix('!') {
                    list.retain(|x| strip_mode(x) != strip_mode(removed));
                } else if !list.iter().any(|x| strip_mode(x) == strip_mode(t)) {
                    list.push(t.clone());
                } else {
                    // Same thing with another access mode (home:ro -> home): the later one wins.
                    list.retain(|x| strip_mode(x) != strip_mode(t));
                    list.push(t.clone());
                }
            }
        }
        out
    }

    pub fn has(&self, key: &str, token: &str) -> bool {
        self.lists.get(key).is_some_and(|l| l.iter().any(|t| strip_mode(t) == token))
    }
}

/// `home:ro` -> `home`.
fn strip_mode(t: &str) -> &str {
    t.split(':').next().unwrap_or(t)
}

/// How much of the person's files an app can reach.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Files {
    /// Only what the person picks for it.
    Chosen,
    Home,
    Everything,
}

pub fn files_level(ctx: &Context) -> Files {
    if ctx.has("filesystems", "host") {
        Files::Everything
    } else if ctx.has("filesystems", "home") {
        Files::Home
    } else {
        Files::Chosen
    }
}

/// The things the screen lets a person switch, each as (what it is, `flatpak override` words to allow it, to deny it).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Switch {
    Network,
    Sound,
    Graphics,
    AllDevices,
    Bluetooth,
}

impl Switch {
    pub fn allowed_in(self, ctx: &Context) -> bool {
        match self {
            Switch::Network => ctx.has("shared", "network"),
            Switch::Sound => ctx.has("sockets", "pulseaudio"),
            Switch::Graphics => ctx.has("devices", "dri") || ctx.has("devices", "all"),
            Switch::AllDevices => ctx.has("devices", "all"),
            Switch::Bluetooth => ctx.has("features", "bluetooth"),
        }
    }

    pub fn override_args(self, on: bool) -> Vec<String> {
        let (yes, no) = match self {
            Switch::Network => ("--share=network", "--unshare=network"),
            Switch::Sound => ("--socket=pulseaudio", "--nosocket=pulseaudio"),
            Switch::Graphics => ("--device=dri", "--nodevice=dri"),
            Switch::AllDevices => ("--device=all", "--nodevice=all"),
            Switch::Bluetooth => ("--allow=bluetooth", "--disallow=bluetooth"),
        };
        vec![(if on { yes } else { no }).to_string()]
    }
}

pub fn files_override_args(level: Files) -> Vec<String> {
    let mut a = vec!["--nofilesystem=host".to_string(), "--nofilesystem=home".to_string()];
    match level {
        Files::Chosen => {}
        Files::Home => a.push("--filesystem=home".into()),
        Files::Everything => a.push("--filesystem=host".into()),
    }
    a
}

fn run(cmd: &str, args: &[String]) -> Result<String, String> {
    let o = Command::new(cmd).args(args).stdin(Stdio::null()).output().map_err(|e| format!("{cmd}: {e}"))?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).to_string())
    } else {
        let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
        Err(if err.is_empty() { format!("{cmd} failed") } else { err })
    }
}

fn flatpak_override(app: &str, mut args: Vec<String>) -> Result<(), String> {
    if !safe_flatpak_id(app) {
        return Err("That app can't be changed.".into());
    }
    let mut full = vec!["override".to_string(), "--user".to_string()];
    full.append(&mut args);
    full.push(app.to_string());
    run("flatpak", &full).map(drop)
}

pub fn set_switch(app: &str, s: Switch, on: bool) -> Result<(), String> {
    flatpak_override(app, s.override_args(on))
}

pub fn set_files(app: &str, level: Files) -> Result<(), String> {
    flatpak_override(app, files_override_args(level))
}

/// Gives back the app's own permissions (removes everything the person changed, including the graphics choice).
pub fn reset_permissions(app: &str) -> Result<(), String> {
    flatpak_override(app, vec!["--reset".to_string()])
}

/// What the app may do now: its own list with the person's changes on top.
pub fn effective_context(app: &str) -> Option<Context> {
    if !safe_flatpak_id(app) {
        return None;
    }
    let meta = run("flatpak", &["info".into(), "--show-permissions".into(), app.to_string()]).ok()?;
    let over = run("flatpak", &["override".into(), "--user".into(), "--show".into(), app.to_string()]).unwrap_or_default();
    Some(parse_context(&meta).with_overrides(&parse_context(&over)))
}

// ── Flatpak: the choices made through the desktop (camera, microphone, notifications) ──

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Portal {
    Camera,
    Microphone,
    Notifications,
}

impl Portal {
    fn table_and_id(self) -> (&'static str, &'static str) {
        match self {
            Portal::Camera => ("devices", "camera"),
            Portal::Microphone => ("devices", "microphone"),
            Portal::Notifications => ("notifications", "notification"),
        }
    }
}

/// From `flatpak permissions` output (tab-separated: table, id, app, permission, data): what is stored for `app`.
pub fn parse_portal(text: &str, app: &str, which: Portal) -> Option<bool> {
    let (table, id) = which.table_and_id();
    text.lines().find_map(|l| {
        let f: Vec<&str> = l.split('\t').map(str::trim).collect();
        (f.len() >= 4 && f[0] == table && f[1] == id && f[2] == app).then(|| f[3] == "yes")
    })
}

/// Whether the desktop lets this app use it. None: never decided (it will ask).
pub fn portal_state(app: &str, which: Portal) -> Option<bool> {
    if !safe_flatpak_id(app) {
        return None;
    }
    run("flatpak", &["permissions".into()]).ok().and_then(|t| parse_portal(&t, app, which))
}

pub fn set_portal(app: &str, which: Portal, allow: bool) -> Result<(), String> {
    if !safe_flatpak_id(app) {
        return Err("That app can't be changed.".into());
    }
    let (table, id) = which.table_and_id();
    run("flatpak", &["permission-set".into(), table.into(), id.into(), app.to_string(), (if allow { "yes" } else { "no" }).to_string()]).map(drop)
}

// ── Sizes of installed packages ────────────────────────────────────────────

/// "Installed Size  : 14.20 MiB" from `pacman -Qi`.
pub fn parse_pacman_size(text: &str) -> Option<String> {
    text.lines().find_map(|l| l.strip_prefix("Installed Size").and_then(|r| r.split_once(':')).map(|(_, v)| v.trim().to_string())).filter(|s| !s.is_empty())
}

pub fn pacman_app_size(pkg: &str) -> Option<String> {
    let ok = !pkg.is_empty() && pkg.bytes().all(|b| b.is_ascii_alphanumeric() || b"@._+-".contains(&b));
    if !ok {
        return None;
    }
    run("pacman", &["-Qi".into(), pkg.to_string()]).ok().and_then(|t| parse_pacman_size(&t))
}

pub fn flatpak_app_size(app: &str) -> Option<String> {
    let out = run("flatpak", &["list".into(), "--app".into(), "--columns=application,size".into()]).ok()?;
    out.lines().find_map(|l| {
        let mut f = l.split('\t');
        (f.next()?.trim() == app).then(|| f.next().unwrap_or("").replace('\u{a0}', " ").trim().to_string())
    })
}

/// Whether a Flatpak app is running now (its data must not be cleared then).
pub fn flatpak_running(app: &str) -> bool {
    run("flatpak", &["ps".into(), "--columns=application".into()]).map(|t| t.lines().any(|l| l.trim() == app)).unwrap_or(false)
}

// ── Notifications ──────────────────────────────────────────────────────────

/// The id Plasma knows an app's notifications by: its launcher's name without `.desktop`.
pub fn notification_id(desktop_id: &str) -> String {
    desktop_id.trim_end_matches(".desktop").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPOTIFY_OWN: &str = "[Context]\nshared=ipc;network;\nsockets=pulseaudio;wayland;\ndevices=dri;\nfilesystems=xdg-run/pipewire-0:ro;xdg-pictures:ro;xdg-music:ro;\n\n[Session Bus Policy]\norg.kde.StatusNotifierWatcher=talk\n\n[Environment]\nTMPDIR=/tmp\n";

    #[test]
    fn sizes_read_like_sizes() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(999), "999 B");
        assert_eq!(human_size(1_500), "1.5 kB");
        assert_eq!(human_size(14_100_000), "14.1 MB");
        assert_eq!(human_size(2_300_000_000), "2.3 GB");
    }

    #[test]
    fn a_folder_is_measured_and_emptied_without_following_links_or_touching_what_is_outside() {
        let root = std::env::temp_dir().join(format!("zs-appdetails-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (inside, outside) = (root.join("inside"), root.join("outside"));
        std::fs::create_dir_all(inside.join("sub")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(inside.join("a.bin"), vec![0u8; 1000]).unwrap();
        std::fs::write(inside.join("sub/b.bin"), vec![0u8; 500]).unwrap();
        std::fs::write(outside.join("precious.txt"), "keep me").unwrap();
        std::os::unix::fs::symlink(&outside, inside.join("link-to-outside")).unwrap();
        assert!(dir_size(&inside) >= 1500);
        let freed = clear_contents(&inside).unwrap();
        assert!(freed >= 1500);
        assert!(inside.is_dir(), "the folder itself stays");
        assert_eq!(std::fs::read_dir(&inside).unwrap().count(), 0);
        assert_eq!(std::fs::read_to_string(outside.join("precious.txt")).unwrap(), "keep me", "a link inside was removed, not followed");
        assert!(clear_contents(&root.join("missing")).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn flatpak_data_lives_only_in_the_apps_own_folder_and_odd_ids_are_refused() {
        assert!(flatpak_data_dir("com.spotify.Client").unwrap().ends_with(".var/app/com.spotify.Client"));
        assert!(flatpak_data_dir("../../etc").is_none());
        assert!(flatpak_data_dir("a/b").is_none());
        assert!(flatpak_data_dir("").is_none());
        assert!(flatpak_data_dir(".hidden").is_none());
    }

    #[test]
    fn what_an_app_may_do_is_its_own_list_with_the_persons_changes_on_top() {
        let own = parse_context(SPOTIFY_OWN);
        assert!(Switch::Network.allowed_in(&own) && Switch::Sound.allowed_in(&own) && Switch::Graphics.allowed_in(&own));
        assert!(!Switch::Bluetooth.allowed_in(&own) && !Switch::AllDevices.allowed_in(&own));
        assert_eq!(files_level(&own), Files::Chosen, "only some folders read-only: not home");

        let over = parse_context("[Context]\nshared=!network;\nsockets=!fallback-x11;\nfeatures=bluetooth;\nfilesystems=home;\n");
        let now = own.with_overrides(&over);
        assert!(!Switch::Network.allowed_in(&now), "network switched off");
        assert!(Switch::Sound.allowed_in(&now), "untouched stays");
        assert!(Switch::Bluetooth.allowed_in(&now));
        assert_eq!(files_level(&now), Files::Home);
        assert!(now.has("shared", "ipc"), "other entries survive");
    }

    #[test]
    fn a_folder_with_an_access_mode_still_counts_and_everything_beats_home() {
        let ctx = parse_context("[Context]\nfilesystems=home:ro;xdg-music;\n");
        assert_eq!(files_level(&ctx), Files::Home);
        let all = parse_context("[Context]\nfilesystems=host;home;\n");
        assert_eq!(files_level(&all), Files::Everything);
        let denied = parse_context("[Context]\nfilesystems=home;\n").with_overrides(&parse_context("[Context]\nfilesystems=!home;\n"));
        assert_eq!(files_level(&denied), Files::Chosen);
    }

    #[test]
    fn the_override_words_are_flatpaks_own() {
        assert_eq!(Switch::Network.override_args(true), vec!["--share=network"]);
        assert_eq!(Switch::Network.override_args(false), vec!["--unshare=network"]);
        assert_eq!(Switch::Sound.override_args(false), vec!["--nosocket=pulseaudio"]);
        assert_eq!(Switch::Bluetooth.override_args(true), vec!["--allow=bluetooth"]);
        assert_eq!(Switch::AllDevices.override_args(false), vec!["--nodevice=all"]);
        assert_eq!(files_override_args(Files::Chosen), vec!["--nofilesystem=host", "--nofilesystem=home"]);
        assert_eq!(files_override_args(Files::Home), vec!["--nofilesystem=host", "--nofilesystem=home", "--filesystem=home"]);
        assert_eq!(files_override_args(Files::Everything).last().map(String::as_str), Some("--filesystem=host"));
        assert!(set_switch("../x", Switch::Network, true).is_err());
        assert!(set_files("a b", Files::Home).is_err());
    }

    #[test]
    fn choices_made_through_the_desktop_are_read_from_the_permission_store() {
        let text = "notifications\tnotification\tcom.spotify.Client\tyes\t0x00\ndevices\tcamera\tcom.spotify.Client\tno\t0x00\ndevices\tmicrophone\torg.other.App\tyes\t0x00\n";
        assert_eq!(parse_portal(text, "com.spotify.Client", Portal::Notifications), Some(true));
        assert_eq!(parse_portal(text, "com.spotify.Client", Portal::Camera), Some(false));
        assert_eq!(parse_portal(text, "com.spotify.Client", Portal::Microphone), None, "never decided");
        assert_eq!(parse_portal(text, "org.other.App", Portal::Microphone), Some(true));
    }

    #[test]
    fn package_sizes_and_notification_ids() {
        assert_eq!(parse_pacman_size("Name            : ark\nInstalled Size  : 14.20 MiB\nPackager : x\n").as_deref(), Some("14.20 MiB"));
        assert_eq!(parse_pacman_size("nothing"), None);
        assert_eq!(notification_id("org.kde.dolphin.desktop"), "org.kde.dolphin");
        assert_eq!(notification_id("firefox"), "firefox");
        assert!(pacman_app_size("bad name; rm").is_none());
    }

    #[test]
    fn folders_for_ordinary_apps_are_found_by_name_and_odd_names_are_ignored() {
        let root = std::env::temp_dir().join(format!("zs-native-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for d in ["config/vlc", "share/vlc", "cache/vlc", "config/other"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        let bases = [root.join("config"), root.join("share"), root.join("state"), root.join("cache")];
        let (data, cache) = native_dirs_in(bases, &["vlc".to_string(), "../other".to_string(), "".to_string(), "missing".to_string(), "vlc".to_string()]);
        assert_eq!(data.len(), 2, "config and share, listed once");
        assert_eq!(cache.len(), 1);
        assert!(data.iter().all(|p| p.ends_with("vlc")));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A real Flatpak app, never started. Spotify's own overrides file is put back at the end.
    /// `cargo test live_app_details -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_app_details_flatpak_permissions_take_effect_inside_the_sandbox() {
        let app = "com.spotify.Client";
        let path = crate::backend::gpu_pref::flatpak_override_path(app);
        let original = std::fs::read_to_string(&path).ok();
        let inside = |cmd: &str| -> String {
            let o = Command::new("flatpak").args(["run", "--command=sh", app, "-c", cmd]).output().unwrap();
            String::from_utf8_lossy(&o.stdout).trim().to_string()
        };
        // Per-sandbox view of the network interfaces (/sys shows the host's even when the network is cut off).
        let net = || inside("tail -n +3 /proc/net/dev | cut -d: -f1 | tr -d ' ' | tr '\\n' ' '");

        let ctx0 = effective_context(app).expect("permissions");
        let net_before = Switch::Network.allowed_in(&ctx0);
        let ifaces_before = net();

        set_switch(app, Switch::Network, false).unwrap();
        let net_off = Switch::Network.allowed_in(&effective_context(app).unwrap());
        let ifaces_off = net();
        set_switch(app, Switch::Network, true).unwrap();
        let net_on = Switch::Network.allowed_in(&effective_context(app).unwrap());
        let ifaces_on = net();

        set_files(app, Files::Home).unwrap();
        let home_level = files_level(&effective_context(app).unwrap());
        let home_visible = inside("ls -d $HOME/Documents >/dev/null 2>&1 && echo yes || echo no");
        set_files(app, Files::Chosen).unwrap();
        let chosen_level = files_level(&effective_context(app).unwrap());
        let home_hidden = inside("ls -d $HOME/Documents >/dev/null 2>&1 && echo yes || echo no");

        let notif = portal_state(app, Portal::Notifications);
        set_portal(app, Portal::Notifications, false).unwrap();
        let notif_off = portal_state(app, Portal::Notifications);
        set_portal(app, Portal::Notifications, true).unwrap();
        let notif_on = portal_state(app, Portal::Notifications);

        let sizes = (flatpak_app_size(app), dir_size(&flatpak_data_dir(app).unwrap()));
        let running = flatpak_running(app);

        // Put Spotify's own file back exactly.
        match &original {
            Some(t) => std::fs::write(&path, t).unwrap(),
            None => {
                let _ = std::fs::remove_file(&path);
            }
        }
        println!("network: before {net_before} {ifaces_before:?} | off {net_off} {ifaces_off:?} | on {net_on} {ifaces_on:?}");
        println!("files: home -> {home_level:?} sees Documents: {home_visible} | chosen -> {chosen_level:?} sees Documents: {home_hidden}");
        println!("notifications: {notif:?} -> off {notif_off:?} -> on {notif_on:?} | size {:?}, data {} | running: {running}", sizes.0, human_size(sizes.1));
        assert!(net_before && net_on && !net_off);
        assert!(ifaces_off.trim() == "lo", "network off should leave only the loopback: {ifaces_off:?}");
        assert!(ifaces_on.split_whitespace().count() > 1, "network back: {ifaces_on:?}");
        assert_eq!((home_level, home_visible.as_str()), (Files::Home, "yes"));
        assert_eq!((chosen_level, home_hidden.as_str()), (Files::Chosen, "no"));
        assert_eq!((notif, notif_off, notif_on), (Some(true), Some(false), Some(true)));
        assert!(sizes.0.is_some() && sizes.1 > 0);
        let _ = running; // Spotify may well be playing right now: that is a fact to show, not to assert
    }
}
