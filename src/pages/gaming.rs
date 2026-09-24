//! Gaming: Feral GameMode status and its config, the MangoHud performance
//! overlay, power mode, graphics driver check, and game stores (installed
//! from Flathub, since the ISO has no 32-bit multilib repo for native Steam).

use crate::backend::worker::in_background;
use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use std::path::{Path, PathBuf};
use std::process::Command;

fn config_home() -> PathBuf {
    std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config"))
}

fn output(cmd: &str, args: &[&str]) -> Option<String> {
    let o = Command::new(cmd).args(args).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

fn message(w: &impl IsA<gtk4::Widget>, title: &str, body: &str) {
    let d = adw::AlertDialog::new(Some(title), Some(body));
    d.add_response("ok", "OK");
    d.present(Some(w));
}

/// Minimal INI edit: set `key=value` in `[section]`, creating either as needed.
fn ini_set(path: &Path, section: &str, key: &str, value: &str) -> std::io::Result<()> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let header = format!("[{section}]");
    let start = match lines.iter().position(|l| l.trim() == header) {
        Some(i) => i,
        None => {
            if !lines.is_empty() {
                lines.push(String::new());
            }
            lines.push(header);
            lines.len() - 1
        }
    };
    let end = lines[start + 1..].iter().position(|l| l.trim_start().starts_with('[')).map(|i| i + start + 1).unwrap_or(lines.len());
    let existing = lines[start + 1..end].iter().position(|l| l.split('=').next().map(str::trim) == Some(key));
    let line = format!("{key}={value}");
    match existing {
        Some(i) => lines[start + 1 + i] = line,
        None => lines.insert(end, line),
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, lines.join("\n") + "\n")
}

fn ini_get(path: &Path, section: &str, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut in_section = false;
    for l in text.lines() {
        let t = l.trim();
        if t.starts_with('[') {
            in_section = t == format!("[{section}]");
        } else if in_section {
            if let Some((k, v)) = t.split_once('=') {
                if k.trim() == key {
                    return Some(v.trim().to_string());
                }
            }
        }
    }
    None
}

// ── Game Mode ──────────────────────────────────────────────────────────────

fn gamemode_ini() -> PathBuf {
    config_home().join("gamemode.ini")
}

fn gamemode_group(page: &gtk4::Box) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("Game Mode");
    if output("sh", &["-c", "command -v gamemoded"]).is_none() {
        let r = adw::ActionRow::new();
        r.set_title("Game Mode isn't installed");
        g.add(&r);
        return g;
    }
    g.set_description(Some("Game Mode gives a running game priority over everything else, then puts things back when you quit."));

    let status = adw::ActionRow::new();
    status.set_title("Status");
    status.add_prefix(&gtk4::Image::from_icon_name("applications-games-symbolic"));
    let active = output("gamemoded", &["-s"]).unwrap_or_default();
    status.set_subtitle(if active.contains("is active") { "Active: a game is using Game Mode now" } else { "Waiting: turns on automatically when a game asks for it" });
    g.add(&status);

    let ini = gamemode_ini();
    let prio = adw::SwitchRow::new();
    prio.set_title("Give games higher priority");
    prio.set_subtitle("Games get CPU time before background apps");
    prio.set_active(ini_get(&ini, "general", "renice").map(|v| v != "0").unwrap_or(false));
    {
        let (ini, page) = (ini.clone(), page.clone());
        prio.connect_active_notify(move |r| {
            if let Err(e) = ini_set(&ini, "general", "renice", if r.is_active() { "10" } else { "0" }) {
                message(&page, "Setting not saved", &e.to_string());
            }
        });
    }
    g.add(&prio);

    let saver = adw::SwitchRow::new();
    saver.set_title("Keep the screen on while playing");
    saver.set_subtitle("Don't dim, lock or turn off the screen during a game");
    saver.set_active(ini_get(&ini, "general", "inhibit_screensaver").map(|v| v != "0").unwrap_or(true));
    {
        let (ini, page) = (ini.clone(), page.clone());
        saver.connect_active_notify(move |r| {
            if let Err(e) = ini_set(&ini, "general", "inhibit_screensaver", if r.is_active() { "1" } else { "0" }) {
                message(&page, "Setting not saved", &e.to_string());
            }
        });
    }
    g.add(&saver);

    let how = adw::ActionRow::new();
    how.set_title("Use Game Mode in Steam");
    how.set_subtitle("In a game's Properties, set Launch Options to: gamemoderun %command%");
    let copy = gtk4::Button::from_icon_name("edit-copy-symbolic");
    copy.add_css_class("flat");
    copy.set_valign(gtk4::Align::Center);
    copy.set_tooltip_text(Some("Copy launch option"));
    copy.connect_clicked(|b| b.clipboard().set_text("gamemoderun %command%"));
    how.add_suffix(&copy);
    g.add(&how);
    g
}

// ── MangoHud ───────────────────────────────────────────────────────────────

fn mangohud_env() -> PathBuf {
    config_home().join("environment.d").join("90-zohara-mangohud.conf")
}

fn mangohud_conf() -> PathBuf {
    config_home().join("MangoHud").join("MangoHud.conf")
}

/// MangoHud.conf is `key=value` lines without sections.
fn conf_set(key: &str, value: Option<&str>) -> std::io::Result<()> {
    let path = mangohud_conf();
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = text
        .lines()
        .filter(|l| l.split('=').next().map(str::trim) != Some(key) && l.trim() != key)
        .map(str::to_string)
        .collect();
    if let Some(v) = value {
        lines.push(if v.is_empty() { key.to_string() } else { format!("{key}={v}") });
    }
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(&path, lines.join("\n") + "\n")
}

fn conf_get(key: &str) -> Option<String> {
    std::fs::read_to_string(mangohud_conf())
        .ok()?
        .lines()
        .find_map(|l| l.split_once('=').filter(|(k, _)| k.trim() == key).map(|(_, v)| v.trim().to_string()))
}

fn mangohud_group(page: &gtk4::Box) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("Performance overlay");
    if output("sh", &["-c", "command -v mangohud"]).is_none() {
        let r = adw::ActionRow::new();
        r.set_title("MangoHud isn't installed");
        g.add(&r);
        return g;
    }
    g.set_description(Some("Shows frame rate, CPU and GPU load in games. Press Right Shift + F12 in a game to hide or show it."));

    let on = adw::SwitchRow::new();
    on.set_title("Show in all games");
    on.set_subtitle("Takes effect after you sign out and back in");
    on.add_prefix(&gtk4::Image::from_icon_name("utilities-system-monitor-symbolic"));
    on.set_active(mangohud_env().exists());
    {
        let page = page.clone();
        on.connect_active_notify(move |r| {
            let res = if r.is_active() {
                std::fs::create_dir_all(mangohud_env().parent().unwrap())
                    // Vulkan games load the layer from this variable; OpenGL games still need `mangohud %command%`.
                    .and_then(|_| std::fs::write(mangohud_env(), "# Managed by Zohara Settings (Gaming)\nMANGOHUD=1\n"))
            } else {
                std::fs::remove_file(mangohud_env()).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) })
            };
            if let Err(e) = res {
                message(&page, "Setting not saved", &e.to_string());
            }
        });
    }
    g.add(&on);

    let style = adw::ComboRow::new();
    style.set_title("Detail");
    const PRESETS: [(&str, &str); 3] = [("Frame rate only", "1"), ("Standard", "3"), ("Detailed", "4")];
    style.set_model(Some(&gtk4::StringList::new(&PRESETS.map(|p| p.0))));
    style.set_selected(conf_get("preset").and_then(|v| PRESETS.iter().position(|p| p.1 == v)).unwrap_or(1) as u32);
    style.connect_selected_notify(|r| {
        let _ = conf_set("preset", Some(PRESETS[r.selected() as usize].1));
    });
    g.add(&style);

    let pos = adw::ComboRow::new();
    pos.set_title("Position");
    const POS: [(&str, &str); 4] = [("Top left", "top-left"), ("Top right", "top-right"), ("Bottom left", "bottom-left"), ("Bottom right", "bottom-right")];
    pos.set_model(Some(&gtk4::StringList::new(&POS.map(|p| p.0))));
    pos.set_selected(conf_get("position").and_then(|v| POS.iter().position(|p| p.1 == v)).unwrap_or(0) as u32);
    pos.connect_selected_notify(|r| {
        let _ = conf_set("position", Some(POS[r.selected() as usize].1));
    });
    g.add(&pos);

    let fps = adw::ComboRow::new();
    fps.set_title("Frame rate limit");
    fps.set_subtitle("A cap saves battery and keeps fans quieter");
    const FPS: [(&str, &str); 6] = [("No limit", ""), ("30 FPS", "30"), ("60 FPS", "60"), ("90 FPS", "90"), ("120 FPS", "120"), ("144 FPS", "144")];
    fps.set_model(Some(&gtk4::StringList::new(&FPS.map(|p| p.0))));
    fps.set_selected(conf_get("fps_limit").and_then(|v| FPS.iter().position(|p| p.1 == v)).unwrap_or(0) as u32);
    fps.connect_selected_notify(|r| {
        let v = FPS[r.selected() as usize].1;
        let _ = conf_set("fps_limit", if v.is_empty() { None } else { Some(v) });
    });
    g.add(&fps);
    g
}

// ── Power mode ─────────────────────────────────────────────────────────────

fn power_group() -> Option<adw::PreferencesGroup> {
    let current = output("powerprofilesctl", &["get"])?;
    let g = adw::PreferencesGroup::new();
    let row = adw::ComboRow::new();
    row.set_title("Power mode");
    row.set_subtitle("Performance gives games the most speed; switch back when you're done to save battery");
    row.add_prefix(&gtk4::Image::from_icon_name("power-profile-performance-symbolic"));
    const MODES: [(&str, &str); 3] = [("power-saver", "Power saver"), ("balanced", "Balanced"), ("performance", "Performance")];
    let available = output("powerprofilesctl", &["list"]).unwrap_or_default();
    let modes: Vec<(&str, &str)> = MODES.iter().copied().filter(|(id, _)| available.contains(&format!("{id}:"))).collect();
    let labels: Vec<&str> = modes.iter().map(|m| m.1).collect();
    row.set_model(Some(&gtk4::StringList::new(&labels)));
    if let Some(i) = modes.iter().position(|m| m.0 == current) {
        row.set_selected(i as u32);
    }
    row.connect_selected_notify(move |r| {
        if let Some((id, _)) = modes.get(r.selected() as usize) {
            let id = id.to_string();
            std::thread::spawn(move || {
                let _ = Command::new("powerprofilesctl").args(["set", &id]).status();
            });
        }
    });
    g.add(&row);
    Some(g)
}

// ── Graphics ───────────────────────────────────────────────────────────────

fn graphics_group() -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("Graphics");
    let lspci = output("lspci", &[]).unwrap_or_default();
    let gpus: Vec<String> = lspci
        .lines()
        .filter(|l| l.contains("VGA compatible controller") || l.contains("3D controller") || l.contains("Display controller"))
        .map(|l| l.splitn(2, ": ").nth(1).unwrap_or(l).to_string())
        .collect();
    let icds: Vec<String> = std::fs::read_dir("/usr/share/vulkan/icd.d")
        .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().to_lowercase()).collect())
        .unwrap_or_default();
    if gpus.is_empty() {
        let r = adw::ActionRow::new();
        r.set_title("No graphics card detected");
        g.add(&r);
    }
    for gpu in gpus {
        let lower = gpu.to_lowercase();
        let driver = if lower.contains("nvidia") {
            icds.iter().any(|i| i.contains("nvidia")).then_some("NVIDIA")
        } else if lower.contains("amd") || lower.contains("ati ") || lower.contains("radeon") {
            icds.iter().any(|i| i.contains("radeon")).then_some("RADV (Mesa)")
        } else if lower.contains("intel") {
            icds.iter().any(|i| i.contains("intel")).then_some("ANV (Mesa)")
        } else {
            None
        };
        let r = adw::ActionRow::new();
        r.set_title(&glib::markup_escape_text(&gpu));
        r.add_prefix(&gtk4::Image::from_icon_name("video-display-symbolic"));
        r.set_subtitle(&match driver {
            Some(d) => format!("Vulkan ready · {d} driver"),
            None => "No Vulkan driver found: most modern games won't start".into(),
        });
        g.add(&r);
    }
    g
}

// ── Game stores ────────────────────────────────────────────────────────────

const STORES: [(&str, &str, &str); 4] = [
    ("Steam", "com.valvesoftware.Steam", "The biggest PC game store"),
    ("Heroic Games Launcher", "com.heroicgameslauncher.hgl", "Epic Games, GOG and Amazon games"),
    ("Lutris", "net.lutris.Lutris", "Games from many stores and emulators in one place"),
    ("Bottles", "com.usebottles.bottles", "Run Windows games and apps"),
];

fn flatpak_installed(id: &str) -> bool {
    Command::new("flatpak").args(["info", id]).output().map(|o| o.status.success()).unwrap_or(false)
}

fn stores_group(page: &gtk4::Box) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("Game stores");
    g.set_description(Some("Installed from Flathub. They include Game Mode support and their own overlay option."));
    for (name, id, desc) in STORES {
        let r = adw::ActionRow::new();
        r.set_title(name);
        r.set_subtitle(desc);
        r.add_prefix(&gtk4::Image::from_icon_name("applications-games-symbolic"));
        let btn = gtk4::Button::new();
        btn.set_valign(gtk4::Align::Center);
        let set_state = {
            let btn = btn.clone();
            move |installed: bool| {
                btn.set_label(if installed { "Open" } else { "Install" });
                if installed {
                    btn.remove_css_class("suggested-action");
                } else {
                    btn.add_css_class("suggested-action");
                }
            }
        };
        set_state(flatpak_installed(id));
        let page2 = page.clone();
        btn.connect_clicked(move |b| {
            if flatpak_installed(id) {
                let _ = Command::new("flatpak").args(["run", id]).spawn();
                return;
            }
            b.set_sensitive(false);
            b.set_label("Installing…");
            let (b2, page3, set_state) = (b.clone(), page2.clone(), set_state.clone());
            in_background(
                move || {
                    Command::new("flatpak")
                        .args(["install", "-y", "--noninteractive", "flathub", id])
                        .output()
                        .map(|o| (o.status.success(), String::from_utf8_lossy(&o.stderr).trim().to_string()))
                        .unwrap_or((false, String::new()))
                },
                move |(ok, err)| {
                    b2.set_sensitive(true);
                    set_state(ok);
                    if !ok {
                        log::warn!("installing {id} failed: {err}");
                        message(&page3, &format!("{name} not installed"), "Check your internet connection and that Flathub is turned on in Apps > App sources.");
                    }
                },
            );
        });
        r.add_suffix(&btn);
        g.add(&r);
    }
    g
}

pub fn build() -> gtk4::Widget {
    let scroll = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .build();

    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 24);
    root.set_margin_start(28);
    root.set_margin_end(28);
    root.set_margin_top(20);
    root.set_margin_bottom(32);
    root.append(
        &gtk4::Label::builder()
            .label("Gaming")
            .halign(gtk4::Align::Start)
            .css_classes(vec!["win11-page-title".to_string()])
            .build(),
    );

    root.append(&gamemode_group(&root));
    root.append(&mangohud_group(&root));
    if let Some(p) = power_group() {
        root.append(&p);
    }
    root.append(&graphics_group());
    root.append(&stores_group(&root));

    scroll.set_child(Some(&root));
    scroll.upcast()
}
