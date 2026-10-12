//! Which graphics chip an app uses on a laptop with two (an Intel or AMD chip for everyday work and a stronger NVIDIA
//! or AMD one for games and heavy apps).
//!
//! "Always": a copy of the app's launcher in the person's own folder with `PrefersNonDefaultGPU=true`. KDE reads that
//! when it starts the app and runs it on the stronger chip (with `switcheroo-control`'s settings for it). The copy is
//! marked as ours, so turning the choice off removes the copy and the app's own launcher is used again.
//! "Now": the same settings applied to one launch.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub const KEY: &str = "PrefersNonDefaultGPU";
const MARK: &str = "X-Zohara-GpuOverride";

fn is_group_header(line: &str) -> bool {
    line.trim_start().starts_with('[')
}

/// Whether the launcher text asks for the stronger chip.
pub fn prefers(text: &str) -> bool {
    let mut in_entry = false;
    for line in text.lines() {
        let l = line.trim();
        if is_group_header(l) {
            in_entry = l == "[Desktop Entry]";
        } else if in_entry {
            if let Some(v) = l.strip_prefix(KEY).and_then(|r| r.trim_start().strip_prefix('=')) {
                return v.trim() == "true";
            }
        }
    }
    false
}

/// The launcher text with the choice turned on (the key added or set to true) or off (the key removed).
/// Only the `[Desktop Entry]` group is touched.
pub fn with_preference(text: &str, on: bool) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut in_entry = false;
    let mut inserted = false;
    for line in text.lines() {
        let l = line.trim();
        if is_group_header(l) {
            if in_entry && on && !inserted {
                out.push(format!("{KEY}=true"));
                inserted = true;
            }
            in_entry = l == "[Desktop Entry]";
            out.push(line.to_string());
            continue;
        }
        if in_entry && l.strip_prefix(KEY).is_some_and(|r| r.trim_start().starts_with('=')) {
            if on && !inserted {
                out.push(format!("{KEY}=true"));
                inserted = true;
            }
            continue; // the old line goes either way
        }
        out.push(line.to_string());
    }
    if in_entry && on && !inserted {
        out.push(format!("{KEY}=true"));
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

fn has_mark(text: &str) -> bool {
    text.lines().any(|l| l.trim() == format!("{MARK}=true"))
}

pub fn user_applications_dir() -> PathBuf {
    let base = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share"));
    base.join("applications")
}

/// A launcher file name that is safe to write into our folder: no folders, ends in `.desktop`.
fn safe_id(id: &str) -> bool {
    !id.is_empty() && id.len() < 200 && id.ends_with(".desktop") && !id.contains('/') && !id.starts_with('.')
}

/// Whether the app is set to use the stronger chip: the person's own launcher counts first, then the system's.
pub fn is_set(desktop_id: &str, system_path: &Path) -> bool {
    if !safe_id(desktop_id) {
        return false;
    }
    let user = user_applications_dir().join(desktop_id);
    std::fs::read_to_string(&user).or_else(|_| std::fs::read_to_string(system_path)).map(|t| prefers(&t)).unwrap_or(false)
}

/// Turns "always use the stronger chip" on or off for one app.
pub fn set(desktop_id: &str, system_path: &Path, on: bool) -> Result<(), String> {
    if !safe_id(desktop_id) {
        return Err("That app can't be changed.".into());
    }
    let user = user_applications_dir().join(desktop_id);
    let existing = std::fs::read_to_string(&user).ok();
    if on {
        let (base, ours) = match &existing {
            Some(t) => (t.clone(), has_mark(t)),
            None => (std::fs::read_to_string(system_path).map_err(|e| format!("Couldn't read the app's launcher ({e})"))?, true),
        };
        let mut text = with_preference(&base, true);
        if ours && !has_mark(&text) {
            // The marker goes inside [Desktop Entry]: right after its header.
            text = text.replacen("[Desktop Entry]\n", &format!("[Desktop Entry]\n{MARK}=true\n"), 1);
        }
        std::fs::create_dir_all(user_applications_dir()).map_err(|e| e.to_string())?;
        std::fs::write(&user, text).map_err(|e| format!("Couldn't save the choice ({e})"))
    } else {
        match existing {
            None => Ok(()), // only the system launcher: nothing of ours to undo
            Some(t) if has_mark(&t) => std::fs::remove_file(&user).map_err(|e| e.to_string()),
            Some(t) => std::fs::write(&user, with_preference(&t, false)).map_err(|e| e.to_string()),
        }
    }
}

/// `KEY=VALUE KEY=VALUE` of the stronger chip from `switcherooctl list`.
pub fn parse_dedicated_env(list: &str) -> Vec<(String, String)> {
    let mut discrete = false;
    for line in list.lines() {
        let l = line.trim();
        if l.starts_with("Device:") {
            discrete = false;
        } else if let Some(v) = l.strip_prefix("Discrete:") {
            discrete = v.trim() == "yes";
        } else if let Some(v) = l.strip_prefix("Environment:") {
            if discrete {
                return v
                    .split_whitespace()
                    .filter_map(|kv| kv.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())))
                    .filter(|(k, v)| !k.is_empty() && k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') && !v.contains(['\n', '\0']))
                    .collect();
            }
        }
    }
    Vec::new()
}

pub fn dedicated_env() -> Vec<(String, String)> {
    Command::new("switcherooctl")
        .arg("list")
        .stderr(Stdio::null())
        .output()
        .map(|o| parse_dedicated_env(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
}

/// Starts the app once on the stronger chip.
pub fn launch_now(system_path: &Path) -> Result<(), String> {
    let env = dedicated_env();
    if env.is_empty() {
        return Err("No second graphics chip was found. Is switcheroo-control running?".into());
    }
    Command::new("gio")
        .arg("launch")
        .arg(system_path)
        .envs(env)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
        .map_err(|e| format!("Couldn't start the app ({e})"))
}

// ── Flatpak apps ───────────────────────────────────────────────────────────
//
// A Flatpak app runs in a sandbox that does not see the settings the desktop would give it, so the chip's settings go
// in as that app's own environment: `flatpak run --env=...` for one launch, and an entry in the person's overrides file
// for "always" (the same file `flatpak override --user --env=...` writes; `--unset-env` only blanks a value, so
// taking the choice back out is done here).

fn safe_flatpak_id(id: &str) -> bool {
    !id.is_empty() && id.len() < 200 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)) && !id.starts_with('.')
}

pub fn flatpak_override_path(app_id: &str) -> PathBuf {
    let base = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share"));
    base.join("flatpak").join("overrides").join(app_id)
}

/// Whether every one of `env` is set, with that value, in the `[Environment]` group of an overrides file.
pub fn override_has_env(text: &str, env: &[(String, String)]) -> bool {
    if env.is_empty() {
        return false;
    }
    let mut in_env = false;
    let mut found = 0;
    for line in text.lines() {
        let l = line.trim();
        if is_group_header(l) {
            in_env = l == "[Environment]";
        } else if in_env {
            if let Some((k, v)) = l.split_once('=') {
                if env.iter().any(|(ek, ev)| ek == k.trim() && ev == v.trim()) {
                    found += 1;
                }
            }
        }
    }
    found == env.len()
}

/// The overrides file with `env` set in `[Environment]` (on) or taken out of it (off). Nothing else is touched; an
/// `[Environment]` group left empty goes too.
pub fn override_with_env(text: &str, env: &[(String, String)], on: bool) -> String {
    let mut groups: Vec<(String, Vec<String>)> = Vec::new(); // (header line, body lines); the first group may have no header
    let mut current: (String, Vec<String>) = (String::new(), Vec::new());
    for line in text.lines() {
        if is_group_header(line.trim()) {
            groups.push(std::mem::take(&mut current));
            current = (line.trim().to_string(), Vec::new());
        } else {
            current.1.push(line.to_string());
        }
    }
    groups.push(current);
    let ours = |line: &str| line.split_once('=').is_some_and(|(k, _)| env.iter().any(|(ek, _)| ek == k.trim()));
    let mut has_env_group = false;
    for (header, body) in groups.iter_mut() {
        if header == "[Environment]" {
            has_env_group = true;
            body.retain(|l| !ours(l));
            if on {
                body.extend(env.iter().map(|(k, v)| format!("{k}={v}")));
            }
        }
    }
    if on && !has_env_group {
        groups.push(("[Environment]".to_string(), env.iter().map(|(k, v)| format!("{k}={v}")).collect()));
    }
    let mut out = String::new();
    for (header, body) in groups {
        let has_content = body.iter().any(|l| !l.trim().is_empty());
        if header.is_empty() {
            if has_content {
                out += &body.join("\n");
                out.push('\n');
            }
            continue;
        }
        if header == "[Environment]" && !has_content {
            continue; // nothing left in it
        }
        out += &header;
        out.push('\n');
        for l in body.iter().filter(|l| !l.trim().is_empty()) {
            out += l;
            out.push('\n');
        }
    }
    out
}

pub fn flatpak_is_set(app_id: &str) -> bool {
    safe_flatpak_id(app_id)
        && std::fs::read_to_string(flatpak_override_path(app_id)).map(|t| override_has_env(&t, &dedicated_env())).unwrap_or(false)
}

pub fn flatpak_set(app_id: &str, on: bool) -> Result<(), String> {
    if !safe_flatpak_id(app_id) {
        return Err("That app can't be changed.".into());
    }
    let env = dedicated_env();
    if env.is_empty() {
        return Err("No second graphics chip was found. Is switcheroo-control running?".into());
    }
    let path = flatpak_override_path(app_id);
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    let new = override_with_env(&old, &env, on);
    if new.trim().is_empty() {
        return match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        };
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, new).map_err(|e| format!("Couldn't save the choice ({e})"))
}

/// Starts a Flatpak app once on the stronger chip.
pub fn flatpak_launch_now(app_id: &str) -> Result<(), String> {
    if !safe_flatpak_id(app_id) {
        return Err("That app can't be started.".into());
    }
    let env = dedicated_env();
    if env.is_empty() {
        return Err("No second graphics chip was found. Is switcheroo-control running?".into());
    }
    let mut cmd = Command::new("flatpak");
    cmd.arg("run");
    for (k, v) in &env {
        cmd.arg(format!("--env={k}={v}"));
    }
    cmd.arg(app_id).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    cmd.spawn().map(drop).map_err(|e| format!("Couldn't start the app ({e})"))
}

/// Whether Flatpak apps can use the NVIDIA chip: the driver's Flatpak counterpart has to be installed, or the app
/// quietly stays on the other chip. Always true when the stronger chip isn't NVIDIA.
pub fn flatpak_gpu_ready() -> bool {
    let nvidia = dedicated_env().iter().any(|(k, _)| k == "__NV_PRIME_RENDER_OFFLOAD");
    if !nvidia {
        return true;
    }
    let version = std::fs::read_to_string("/sys/module/nvidia/version").unwrap_or_default().trim().replace('.', "-");
    if version.is_empty() {
        return false;
    }
    let list = Command::new("flatpak").args(["list", "--runtime", "--columns=application"]).stderr(Stdio::null()).output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
    list.lines().any(|l| l.trim() == format!("org.freedesktop.Platform.GL.nvidia-{version}"))
}

/// A laptop with two graphics chips (from /sys: instant).
pub fn has_two_gpus() -> bool {
    super::gpu::detect().count() >= 2
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP: &str = "[Desktop Entry]\nType=Application\nName=Game\nExec=game %U\n\n[Desktop Action new]\nName=New\nExec=game --new\n";

    #[test]
    fn the_key_is_added_inside_desktop_entry_only_and_removed_again() {
        assert!(!prefers(APP));
        let on = with_preference(APP, true);
        assert!(prefers(&on));
        let entry_part = on.split("[Desktop Action").next().unwrap();
        assert!(entry_part.contains("PrefersNonDefaultGPU=true"), "{on}");
        assert_eq!(on.matches(KEY).count(), 1);
        let off = with_preference(&on, false);
        assert!(!prefers(&off));
        assert!(!off.contains(KEY));
        assert!(off.contains("[Desktop Action new]\nName=New\nExec=game --new"), "other groups are untouched");
    }

    #[test]
    fn an_existing_value_is_replaced_not_repeated() {
        let t = "[Desktop Entry]\nName=X\nPrefersNonDefaultGPU=false\nExec=x\n";
        let on = with_preference(t, true);
        assert!(prefers(&on));
        assert_eq!(on.matches(KEY).count(), 1);
        assert!(!prefers(t));
    }

    #[test]
    fn a_file_that_ends_inside_desktop_entry_still_gets_the_key() {
        assert!(prefers(&with_preference("[Desktop Entry]\nName=X\nExec=x", true)));
    }

    #[test]
    fn the_stronger_chip_is_the_discrete_one_and_its_settings_are_read() {
        let list = "Device: 0\n  Name:        Intel\n  Default:     yes\n  Discrete:    no\n  Environment: DRI_PRIME=pci-0000_00_02_0 VK_LOADER_DRIVERS_SELECT=*intel*\n\nDevice: 1\n  Name:        NVIDIA\n  Default:     no\n  Discrete:    yes\n  Environment: __GLX_VENDOR_LIBRARY_NAME=nvidia __NV_PRIME_RENDER_OFFLOAD=1 VK_LOADER_DRIVERS_SELECT=*nvidia*\n";
        let env = parse_dedicated_env(list);
        assert!(env.contains(&("__NV_PRIME_RENDER_OFFLOAD".to_string(), "1".to_string())));
        assert!(env.contains(&("__GLX_VENDOR_LIBRARY_NAME".to_string(), "nvidia".to_string())));
        assert!(!env.iter().any(|(k, _)| k == "DRI_PRIME"), "not the integrated chip's");
        assert!(parse_dedicated_env("Device: 0\n  Discrete: no\n  Environment: A=1\n").is_empty());
        assert!(parse_dedicated_env("").is_empty());
    }

    #[test]
    fn launcher_names_are_checked_before_a_file_is_written() {
        assert!(safe_id("firefox.desktop"));
        assert!(!safe_id("../evil.desktop"));
        assert!(!safe_id("a/b.desktop"));
        assert!(!safe_id(".hidden.desktop"));
        assert!(!safe_id("notadesktopfile"));
        assert!(set("../x.desktop", Path::new("/nonexistent"), true).is_err());
    }

    /// Against a throwaway folder: on makes a marked copy that prefers the stronger chip, off removes it again.
    #[test]
    fn turning_the_choice_on_and_off_leaves_no_trace() {
        let dir = std::env::temp_dir().join(format!("zs-gpupref-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let system = dir.join("system-game.desktop");
        std::fs::write(&system, APP).unwrap();
        // The user folder is under XDG_DATA_HOME: point it at the throwaway folder for this test only.
        let user = dir.join("data");
        std::env::set_var("XDG_DATA_HOME", &user);
        assert!(!is_set("game.desktop", &system));
        set("game.desktop", &system, true).unwrap();
        let written = std::fs::read_to_string(user.join("applications/game.desktop")).unwrap();
        assert!(prefers(&written) && written.contains("X-Zohara-GpuOverride=true") && written.contains("Exec=game %U"));
        assert!(is_set("game.desktop", &system));
        set("game.desktop", &system, true).unwrap(); // again: still one key, one marker
        let again = std::fs::read_to_string(user.join("applications/game.desktop")).unwrap();
        assert_eq!(again.matches(KEY).count(), 1);
        assert_eq!(again.matches(MARK).count(), 1);
        set("game.desktop", &system, false).unwrap();
        assert!(!user.join("applications/game.desktop").exists(), "our copy is gone");
        assert!(!is_set("game.desktop", &system));
        // The system launcher itself was never touched.
        assert_eq!(std::fs::read_to_string(&system).unwrap(), APP);
        // A launcher the person made themselves keeps their other changes.
        std::fs::create_dir_all(user.join("applications")).unwrap();
        std::fs::write(user.join("applications/mine.desktop"), "[Desktop Entry]\nName=Mine\nExec=mine\nPrefersNonDefaultGPU=true\n").unwrap();
        set("mine.desktop", &system, false).unwrap();
        let mine = std::fs::read_to_string(user.join("applications/mine.desktop")).unwrap();
        assert!(mine.contains("Name=Mine") && !prefers(&mine));
        std::env::remove_var("XDG_DATA_HOME");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn env_file(path: &str) -> String {
        for _ in 0..30 {
            if let Ok(t) = std::fs::read_to_string(path) {
                if !t.is_empty() {
                    return t;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        String::new()
    }

    /// Needs a Plasma session on a laptop with two graphics chips. `cargo test live_gpu -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_gpu_choice_reaches_the_app_both_when_always_and_when_opened_now() {
        assert!(has_two_gpus(), "needs two graphics chips");
        let dir = std::env::temp_dir().join(format!("zs-gpu-live-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (out_always, out_plain, out_now) = (dir.join("always.txt"), dir.join("plain.txt"), dir.join("now.txt"));
        let make = |out: &Path| format!("[Desktop Entry]\nType=Application\nName=GPU live test\nExec=sh -c \"env > {}\"\nNoDisplay=true\n", out.display());
        let system = dir.join("system.desktop");
        let id = format!("zs-gpu-live-{}.desktop", std::process::id());

        // Always: our copy asks for the stronger chip, KDE starts it with that chip's settings.
        std::fs::write(&system, make(&out_always)).unwrap();
        set(&id, &system, true).unwrap();
        let user_file = user_applications_dir().join(&id);
        Command::new("kioclient").arg("exec").arg(&user_file).status().unwrap();
        let with = env_file(out_always.to_str().unwrap());

        // Off again: the copy is gone, and a launch has the ordinary settings.
        set(&id, &system, false).unwrap();
        assert!(!user_file.exists(), "copy removed");
        std::fs::write(&system, make(&out_plain)).unwrap();
        Command::new("kioclient").arg("exec").arg(&system).status().unwrap();
        let without = env_file(out_plain.to_str().unwrap());

        // Now: one launch on the stronger chip.
        std::fs::write(&system, make(&out_now)).unwrap();
        launch_now(&system).unwrap();
        let now = env_file(out_now.to_str().unwrap());
        let _ = std::fs::remove_dir_all(&dir);

        println!("always: nvidia offload = {} | off: {} | open now: {}", with.contains("__NV_PRIME_RENDER_OFFLOAD=1"), without.contains("__NV_PRIME_RENDER_OFFLOAD=1"), now.contains("__NV_PRIME_RENDER_OFFLOAD=1"));
        assert!(with.contains("__NV_PRIME_RENDER_OFFLOAD=1") && with.contains("__GLX_VENDOR_LIBRARY_NAME=nvidia"), "always did not reach the app:\n{with}");
        assert!(!without.contains("__NV_PRIME_RENDER_OFFLOAD"), "off still used the stronger chip");
        assert!(now.contains("__NV_PRIME_RENDER_OFFLOAD=1"), "open now did not reach the app:\n{now}");
    }

    fn nv() -> Vec<(String, String)> {
        vec![("__NV_PRIME_RENDER_OFFLOAD".into(), "1".into()), ("__GLX_VENDOR_LIBRARY_NAME".into(), "nvidia".into())]
    }

    #[test]
    fn a_flatpak_override_gets_the_settings_added_and_taken_out_again_leaving_everything_else() {
        let none = override_with_env("", &nv(), true);
        assert_eq!(none, "[Environment]\n__NV_PRIME_RENDER_OFFLOAD=1\n__GLX_VENDOR_LIBRARY_NAME=nvidia\n");
        assert!(override_has_env(&none, &nv()));
        assert_eq!(override_with_env(&none, &nv(), false), "", "nothing left: the file can go");

        let theirs = "[Context]\nshared=network;\nsockets=x11;\n\n[Environment]\nFOO=bar\n";
        let on = override_with_env(theirs, &nv(), true);
        assert!(on.contains("[Context]\nshared=network;\nsockets=x11;\n"));
        assert!(on.contains("FOO=bar") && override_has_env(&on, &nv()));
        assert_eq!(on.matches("__NV_PRIME_RENDER_OFFLOAD").count(), 1);
        let off = override_with_env(&on, &nv(), false);
        assert!(off.contains("FOO=bar") && !off.contains("NV_PRIME") && off.contains("[Context]\nshared=network;"));
        assert!(!override_has_env(&off, &nv()));
        // Turning on twice does not repeat the lines.
        assert_eq!(override_with_env(&on, &nv(), true).matches("__GLX_VENDOR_LIBRARY_NAME").count(), 1);
    }

    #[test]
    fn a_value_that_differs_or_is_missing_does_not_count_as_set() {
        assert!(!override_has_env("[Environment]\n__NV_PRIME_RENDER_OFFLOAD=0\n__GLX_VENDOR_LIBRARY_NAME=nvidia\n", &nv()));
        assert!(!override_has_env("[Environment]\n__NV_PRIME_RENDER_OFFLOAD=1\n", &nv()));
        assert!(!override_has_env("[Context]\n__NV_PRIME_RENDER_OFFLOAD=1\n__GLX_VENDOR_LIBRARY_NAME=nvidia\n", &nv()), "wrong group");
        assert!(!override_has_env("", &[]));
    }

    #[test]
    fn flatpak_ids_are_checked() {
        assert!(safe_flatpak_id("com.spotify.Client"));
        assert!(!safe_flatpak_id("../x"));
        assert!(!safe_flatpak_id("a b"));
        assert!(!safe_flatpak_id(""));
        assert!(flatpak_set("../x", true).is_err());
    }

    /// A real Flatpak app, without starting it: the settings must be inside its sandbox when "always" is on, and gone
    /// when off, with the person's own overrides file left as it was. `cargo test live_flatpak_gpu -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_flatpak_gpu_choice_reaches_the_sandbox() {
        assert!(has_two_gpus() && !dedicated_env().is_empty());
        let app = "com.spotify.Client";
        let path = flatpak_override_path(app);
        let before = std::fs::read_to_string(&path).ok();
        let inside = |key: &str| {
            let o = Command::new("flatpak").args(["run", "--command=sh", app, "-c", &format!("echo \"V=${key}\"")]).output().unwrap();
            String::from_utf8_lossy(&o.stdout).lines().find_map(|l| l.strip_prefix("V=")).unwrap_or("").to_string()
        };
        let off_value = inside("__NV_PRIME_RENDER_OFFLOAD");
        flatpak_set(app, true).unwrap();
        let on_value = inside("__NV_PRIME_RENDER_OFFLOAD");
        let on_flag = flatpak_is_set(app);
        flatpak_set(app, false).unwrap();
        let back_value = inside("__NV_PRIME_RENDER_OFFLOAD");
        let after = std::fs::read_to_string(&path).ok();
        println!("before: {off_value:?} | always on: {on_value:?} (is_set {on_flag}) | off again: {back_value:?} | file restored: {}", before == after);
        assert_eq!(off_value, "");
        assert_eq!(on_value, "1");
        assert!(on_flag && !flatpak_is_set(app));
        assert_eq!(back_value, "");
        assert_eq!(before, after, "the overrides file is as it was");
        assert!(flatpak_gpu_ready(), "the NVIDIA part for Flatpak is installed");
    }
}
