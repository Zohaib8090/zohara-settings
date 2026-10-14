//! Gaming: Feral GameMode status and its config, the MangoHud performance
//! overlay, power mode, graphics driver check, Windows game support (32-bit
//! libraries, Steam, Proton-GE, ProtonUp-Qt) and game stores from Flathub.

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
    // `lspci` can take a couple of seconds (it wakes a sleeping NVIDIA chip), so it runs in the background and the
    // page opens straight away.
    let loading = adw::ActionRow::new();
    loading.set_title("Checking your graphics…");
    g.add(&loading);

    let g2 = g.clone();
    in_background(
        || {
            let lspci = output("lspci", &[]).unwrap_or_default();
            let gpus: Vec<String> = lspci
                .lines()
                .filter(|l| l.contains("VGA compatible controller") || l.contains("3D controller") || l.contains("Display controller"))
                .map(|l| l.splitn(2, ": ").nth(1).unwrap_or(l).to_string())
                .collect();
            let icds: Vec<String> = std::fs::read_dir("/usr/share/vulkan/icd.d")
                .map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().to_lowercase()).collect())
                .unwrap_or_default();
            (gpus, icds)
        },
        move |(gpus, icds)| {
            g2.remove(&loading);
            if gpus.is_empty() {
                let r = adw::ActionRow::new();
                r.set_title("No graphics card detected");
                g2.add(&r);
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
                g2.add(&r);
            }
        },
    );
    g
}

// ── Live graphics ──────────────────────────────────────────────────────────

/// Each graphics chip with its load, video memory and temperature, and which apps use which chip. Refreshes every two
/// seconds while the page is on screen. A sleeping chip is not woken (see `backend::gpu_live`).
fn live_group() -> adw::PreferencesGroup {
    use crate::backend::gpu_live::{self, Gpu, GpuApp};
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    let g = adw::PreferencesGroup::new();
    g.set_title("Graphics in use");
    g.set_description(Some("Live numbers for each graphics chip, and which app is using which one. A chip that nothing uses goes to sleep to save battery, and is shown as asleep."));

    let rows: Rc<RefCell<Vec<adw::ActionRow>>> = Rc::new(RefCell::new(Vec::new()));
    let ids: std::sync::Arc<String> = std::sync::Arc::new(std::fs::read_to_string("/usr/share/hwdata/pci.ids").unwrap_or_default());
    let busy = Rc::new(Cell::new(false));

    let show = {
        let (g, rows) = (g.clone(), rows.clone());
        move |gpus: Vec<Gpu>, apps: Vec<GpuApp>| {
            for r in rows.borrow_mut().drain(..) {
                g.remove(&r);
            }
            if gpus.is_empty() {
                let r = adw::ActionRow::new();
                r.set_title("No graphics chip found");
                g.add(&r);
                rows.borrow_mut().push(r);
                return;
            }
            for gpu in &gpus {
                let r = adw::ActionRow::new();
                r.set_title(&glib::markup_escape_text(&gpu.name));
                r.set_subtitle(&glib::markup_escape_text(&gpu_live::describe(gpu)));
                r.set_subtitle_lines(0);
                r.add_prefix(&gtk4::Image::from_icon_name("video-display-symbolic"));
                if gpu.primary && gpus.len() > 1 {
                    let tag = gtk4::Label::new(Some("Drives the screen"));
                    tag.add_css_class("dim-label");
                    r.add_suffix(&tag);
                }
                g.add(&r);
                rows.borrow_mut().push(r);
            }
            let r = adw::ActionRow::new();
            r.set_title("Apps using a graphics chip");
            r.set_subtitle_lines(0);
            r.set_subtitle(&if apps.is_empty() {
                "None right now. Start a game and it shows up here.".to_string()
            } else {
                apps.iter().take(8).map(|a| format!("{}  →  {}", a.name, gpus.get(a.gpu).map(|g| g.name.as_str()).unwrap_or("?"))).collect::<Vec<_>>().join("\n")
            });
            r.add_prefix(&gtk4::Image::from_icon_name("applications-games-symbolic"));
            g.add(&r);
            rows.borrow_mut().push(r);
        }
    };
    let show = Rc::new(show);

    let tick = {
        let (g, ids, busy, show) = (g.clone(), ids.clone(), busy.clone(), show.clone());
        Rc::new(move || {
            if busy.get() || !g.is_mapped() {
                return;
            }
            busy.set(true);
            let (ids, busy, show) = (ids.clone(), busy.clone(), show.clone());
            in_background(
                move || {
                    let gpus = gpu_live::read_gpus(&ids);
                    let apps = gpu_live::apps_on_gpus(&gpus);
                    (gpus, apps)
                },
                move |(gpus, apps)| {
                    busy.set(false);
                    show(gpus, apps);
                },
            );
        })
    };
    // First look as soon as the page is on screen, then every two seconds while it is.
    {
        let tick = tick.clone();
        g.connect_map(move |_| tick());
    }
    let weak = g.downgrade();
    glib::timeout_add_local(std::time::Duration::from_secs(2), move || {
        if weak.upgrade().is_none() {
            return glib::ControlFlow::Break;
        }
        tick();
        glib::ControlFlow::Continue
    });
    g
}

// ── Game stores ────────────────────────────────────────────────────────────

const STORES: [(&str, &str, &str); 4] = [
    ("Steam", "com.valvesoftware.Steam", "The biggest PC game store"),
    ("Heroic Games Launcher", "com.heroicgameslauncher.hgl", "Epic Games, GOG and Amazon games"),
    ("Lutris", "net.lutris.Lutris", "Games from many stores and emulators in one place"),
    ("Bottles", "com.usebottles.bottles", "Run Windows games and apps"),
];

/// Makes sure the Flathub source exists and is on before installing from it. The installer adds it once at the end
/// of setup (and ignores failure), so a machine installed without internet, or one where the source was switched
/// off, would otherwise fail every install with a vague message.
fn ensure_flathub() {
    let _ = Command::new("flatpak")
        .args(["remote-add", "--if-not-exists", "flathub", "https://dl.flathub.org/repo/flathub.flatpakrepo"])
        .output();
    let _ = Command::new("flatpak").args(["remote-modify", "--enable", "flathub"]).output();
}

/// The last line flatpak printed on stderr, trimmed, for the error dialog.
fn last_error_line(stderr: &str) -> String {
    let line = stderr.lines().rev().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    line.chars().take(200).collect()
}

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
                    ensure_flathub();
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
                        let why = last_error_line(&err);
                        let mut body = String::from("Check your internet connection and that Flathub is turned on in Apps > App sources.");
                        if !why.is_empty() {
                            body.push_str(&format!("\n\nWhat flatpak said: {why}"));
                        }
                        message(&page3, &format!("{name} not installed"), &body);
                    }
                },
            );
        });
        r.add_suffix(&btn);
        g.add(&r);
    }
    g
}

/// Windows games: one-click support (32-bit libraries + Steam, Lutris, winetricks), Proton-GE, ProtonUp-Qt.
fn windows_group(page: &gtk4::Box) -> adw::PreferencesGroup {
    use crate::backend::proton;
    use std::cell::RefCell;
    use std::rc::Rc;

    let g = adw::PreferencesGroup::new();
    g.set_title("Windows games");
    g.set_description(Some("Play Windows games through Steam's Proton. Proton-GE adds fixes for more games than Valve's own version."));

    // 1. Support
    let support = adw::ActionRow::new();
    support.set_title("Windows game support");
    support.set_subtitle_lines(0);
    let sbtn = gtk4::Button::new();
    sbtn.set_valign(gtk4::Align::Center);
    support.add_suffix(&sbtn);
    g.add(&support);

    // 2. Proton-GE
    let ge = adw::ActionRow::new();
    ge.set_title("Proton-GE");
    ge.set_subtitle_lines(0);
    let gbtn = gtk4::Button::new();
    gbtn.set_valign(gtk4::Align::Center);
    ge.add_suffix(&gbtn);
    g.add(&ge);

    // 3. ProtonUp-Qt
    let pq = adw::ActionRow::new();
    pq.set_title("ProtonUp-Qt");
    pq.set_subtitle("Install and update other Proton and Wine versions, for Steam, Lutris and Heroic.");
    let qbtn = gtk4::Button::new();
    qbtn.set_valign(gtk4::Align::Center);
    pq.add_suffix(&qbtn);
    g.add(&pq);

    // Installed Proton-GE versions, each with Remove; rebuilt on refresh.
    let shown: Rc<RefCell<Vec<adw::ActionRow>>> = Rc::new(RefCell::new(Vec::new()));
    let latest: Rc<RefCell<Option<proton::Release>>> = Rc::new(RefCell::new(None));
    let busy = Rc::new(std::cell::Cell::new(false));

    let refresh: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::new(RefCell::new(None));
    let refresh_fn: Rc<dyn Fn()> = {
        let (g, support, sbtn, ge, gbtn, pq, qbtn, shown, latest, busy, refresh) =
            (g.clone(), support.clone(), sbtn.clone(), ge.clone(), gbtn.clone(), pq.clone(), qbtn.clone(), shown.clone(), latest.clone(), busy.clone(), refresh.clone());
        Rc::new(move || {
            let ready = proton::support_ready();
            support.set_subtitle(if ready {
                "Ready: 32-bit libraries, Lutris and winetricks are installed."
            } else {
                "Not set up. Turns on 32-bit libraries and installs Lutris and winetricks (and Steam, if you have none). Asks for your password."
            });
            sbtn.set_label(if ready { "Open Steam" } else { "Set up" });
            sbtn.set_visible(!ready || proton::steam_command().is_some());
            if ready { sbtn.remove_css_class("suggested-action") } else { sbtn.add_css_class("suggested-action") }
            sbtn.set_sensitive(!busy.get());

            let have = proton::installed();
            let newest = latest.borrow().as_ref().map(|r| r.tag.clone());
            let up_to_date = newest.as_ref().map(|n| have.contains(n) && proton::fully_installed(n)).unwrap_or(false);
            ge.set_subtitle(&match (have.first(), &newest) {
                (None, Some(n)) => format!("Not installed. Newest is {n}. It goes into Steam's compatibility list."),
                (None, None) => "Not installed. Press Install to download the newest version (about 560 MB).".to_string(),
                (Some(h), Some(n)) if h != n && !up_to_date => format!("{h} installed. {n} is available."),
                (Some(h), _) => format!("{h} installed. Pick it in Steam: a game's Properties > Compatibility."),
            });
            gbtn.set_label(if have.is_empty() { "Install" } else if up_to_date || newest.is_none() { "Check for newer" } else { "Update" });
            if up_to_date { gbtn.remove_css_class("suggested-action") } else { gbtn.add_css_class("suggested-action") }
            gbtn.set_sensitive(!busy.get());

            qbtn.set_label(if proton::protonup_qt_present() { "Open" } else { "Install" });

            for r in shown.borrow_mut().drain(..) {
                g.remove(&r);
            }
            for tag in have {
                let row = adw::ActionRow::new();
                row.set_title(&tag);
                row.set_subtitle("Installed");
                let rm = gtk4::Button::with_label("Remove");
                rm.set_valign(gtk4::Align::Center);
                rm.add_css_class("destructive-action");
                let refresh = refresh.clone();
                let t = tag.clone();
                rm.connect_clicked(move |_| {
                    let _ = proton::remove(&t);
                    if let Some(f) = refresh.borrow().as_ref() { f() }
                });
                row.add_suffix(&rm);
                g.add(&row);
                shown.borrow_mut().push(row);
            }
        })
    };
    *refresh.borrow_mut() = Some(refresh_fn.clone());
    refresh_fn();

    // Support button
    {
        let (page, support, refresh_fn, busy) = (page.clone(), support.clone(), refresh_fn.clone(), busy.clone());
        sbtn.connect_clicked(move |b| {
            if proton::support_ready() {
                if let Some((prog, args)) = proton::steam_command() {
                    let _ = Command::new(prog).args(args).spawn();
                }
                return;
            }
            busy.set(true);
            b.set_sensitive(false);
            support.set_subtitle("Working… this downloads a few hundred MB and can take several minutes.");
            let (page, refresh_fn, busy) = (page.clone(), refresh_fn.clone(), busy.clone());
            in_background(proton::run_support, move |err| {
                busy.set(false);
                refresh_fn();
                if let Some(e) = err {
                    message(&page, "Windows game support not set up", &e);
                }
            });
        });
    }

    // Proton-GE button
    {
        let (page, ge, refresh_fn, latest, busy) = (page.clone(), ge.clone(), refresh_fn.clone(), latest.clone(), busy.clone());
        gbtn.connect_clicked(move |b| {
            busy.set(true);
            b.set_sensitive(false);
            b.set_label("Working…");
            ge.set_subtitle("Looking for the newest version…");
            let (page, ge, refresh_fn, latest, busy) = (page.clone(), ge.clone(), refresh_fn.clone(), latest.clone(), busy.clone());
            in_background(
                || -> Result<proton::Release, String> { proton::latest() },
                move |res| match res {
                    Err(e) => {
                        busy.set(false);
                        refresh_fn();
                        message(&page, "Proton-GE", &e);
                    }
                    Ok(rel) => {
                        *latest.borrow_mut() = Some(rel.clone());
                        if proton::fully_installed(&rel.tag) {
                            busy.set(false);
                            refresh_fn(); // already the newest
                            return;
                        }
                        // Show the download's progress by watching the file grow.
                        let path = proton::download_path(&rel.tag);
                        let (ge2, size) = (ge.clone(), rel.size.max(1));
                        let done = Rc::new(std::cell::Cell::new(false));
                        let done2 = done.clone();
                        let tag = rel.tag.clone();
                        glib::timeout_add_local(std::time::Duration::from_millis(700), move || {
                            if done2.get() {
                                return glib::ControlFlow::Break;
                            }
                            let have = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                            ge2.set_subtitle(&if have >= size {
                                format!("Checking and unpacking {tag}…")
                            } else {
                                format!("Downloading {tag}: {}% ({} of {} MB)", (have * 100 / size).min(99), have / 1_000_000, size / 1_000_000)
                            });
                            glib::ControlFlow::Continue
                        });
                        let rel2 = rel.clone();
                        in_background(move || proton::install(&rel2), move |r| {
                            done.set(true);
                            busy.set(false);
                            refresh_fn();
                            if let Err(e) = r {
                                message(&page, "Proton-GE not installed", &e);
                            }
                        });
                    }
                },
            );
        });
    }

    // ProtonUp-Qt button
    {
        let (page, refresh_fn) = (page.clone(), refresh_fn.clone());
        qbtn.connect_clicked(move |b| {
            if proton::protonup_qt_present() {
                let cmd = proton::protonup_qt_command();
                let _ = Command::new(&cmd[0]).args(&cmd[1..]).spawn();
                return;
            }
            b.set_sensitive(false);
            b.set_label("Installing…");
            let (page, refresh_fn, b2) = (page.clone(), refresh_fn.clone(), b.clone());
            in_background(
                || {
                    let pac = Command::new("pkexec").args(["pacman", "-S", "--needed", "--noconfirm", "protonup-qt"]).status().map(|s| s.success()).unwrap_or(false);
                    pac || {
                        ensure_flathub();
                        Command::new("flatpak").args(["install", "-y", "--noninteractive", "flathub", "net.davidotek.pupgui2"]).status().map(|s| s.success()).unwrap_or(false)
                    }
                },
                move |ok| {
                    b2.set_sensitive(true);
                    refresh_fn();
                    if !ok {
                        message(&page, "ProtonUp-Qt not installed", "Check your internet connection and try again.");
                    }
                },
            );
        });
    }

    g
}

/// Windows programs with a double-click: the switch that makes the file manager open .exe/.bat/.msi files with
/// Proton-GE (or Wine) directly, and the defaults they run with.
fn double_click_group(page: &gtk4::Box) -> adw::PreferencesGroup {
    use crate::backend::game_launch::Runner;
    use crate::backend::{gpu_pref, proton, windows_apps};
    use std::rc::Rc;

    let g = adw::PreferencesGroup::new();
    g.set_title("Windows programs");
    g.set_description(Some("Run any Windows program or game by double-clicking it in the file manager: .exe, .bat, installers and shortcuts. No Steam needed."));

    let current = windows_apps::load();

    let on = adw::SwitchRow::new();
    on.set_title("Open Windows programs with a double-click");
    on.set_subtitle_lines(0);
    on.set_active(windows_apps::enabled());
    let set_on_subtitle = {
        let on = on.clone();
        move || {
            on.set_subtitle(if on.is_active() {
                "On: double-clicking a Windows file runs it with the choices below."
            } else {
                "Off: Windows files open the way they did before (plain Wine)."
            })
        }
    };
    set_on_subtitle();

    let layer = adw::ComboRow::new();
    layer.set_title("Compatibility layer");
    layer.set_subtitle_lines(0);
    layer.set_model(Some(&gtk4::StringList::new(&["Proton-GE (recommended)", "Wine"])));
    layer.set_selected(if current.runner == Runner::Wine { 1 } else { 0 });
    let set_layer_subtitle = {
        let layer = layer.clone();
        move || {
            let has = proton::installed().first().cloned();
            layer.set_subtitle(&match (layer.selected(), has) {
                (0, Some(v)) => format!("Uses {v} with DXVK: the fastest for games. Programs share one Windows C: drive."),
                (0, None) => "Proton-GE isn't installed yet: install it above. Until then plain Wine is used.".to_string(),
                _ => "Plain Wine. Fine for older programs; slower for modern games.".to_string(),
            });
        }
    };
    set_layer_subtitle();

    let dxvk = adw::SwitchRow::new();
    dxvk.set_title("DirectX through Vulkan (DXVK)");
    dxvk.set_subtitle("Proton-GE only. Faster for most games; turn it off if a game shows glitches.");
    dxvk.set_subtitle_lines(0);
    dxvk.set_active(current.dxvk);

    let gpu = adw::SwitchRow::new();
    gpu.set_title("Use the stronger graphics chip");
    gpu.set_subtitle_lines(0);
    if gpu_pref::has_two_gpus() {
        gpu.set_subtitle("On a laptop with two chips, Windows programs run on the dedicated one.");
        gpu.set_active(current.dedicated_gpu);
    } else {
        gpu.set_subtitle("This computer has one graphics chip.");
        gpu.set_sensitive(false);
    }

    let has = |c: &str| Command::new("sh").args(["-c", &format!("command -v {c}")]).output().map(|o| o.status.success()).unwrap_or(false);
    let gm = adw::SwitchRow::new();
    gm.set_title("Game Mode");
    gm.set_subtitle(if has("gamemoderun") { "Give the program priority while it runs" } else { "Game Mode isn't installed" });
    gm.set_sensitive(has("gamemoderun"));
    gm.set_active(current.gamemode && has("gamemoderun"));
    let ov = adw::SwitchRow::new();
    ov.set_title("Performance overlay (MangoHud)");
    ov.set_subtitle(if has("mangohud") { "Frame rate and load on screen" } else { "MangoHud isn't installed" });
    ov.set_sensitive(has("mangohud"));
    ov.set_active(current.overlay && has("mangohud"));

    let safe = adw::SwitchRow::new();
    safe.set_title("Safety mode for unknown programs");
    safe.set_subtitle("Programs that are not one of My games run in a protected space: they see an empty home folder, not your files, passwords or browser. It limits damage; it is not an antivirus, and anti-cheat games will not run in it.");
    safe.set_subtitle_lines(0);
    safe.set_active(windows_apps::sandbox_default());
    safe.set_sensitive(crate::backend::windows_sandbox::available());
    if !safe.is_sensitive() {
        safe.set_subtitle("This computer can't make a protected space (bubblewrap is missing or blocked), so programs run without it.");
    }
    {
        let page = page.clone();
        safe.connect_active_notify(move |r| {
            if let Err(e) = windows_apps::set_sandbox_default(r.is_active()) {
                message(&page, "Couldn't save", &e);
            }
        });
    }

    let cdrive = adw::ActionRow::new();
    cdrive.set_title("Windows C: drive");
    cdrive.set_subtitle("Where installers put programs. Open it to find an installed program and double-click it.");
    cdrive.set_subtitle_lines(0);
    let open_c = gtk4::Button::with_label("Open");
    open_c.set_valign(gtk4::Align::Center);
    cdrive.add_suffix(&open_c);

    // Every change is saved at once.
    let save: Rc<dyn Fn()> = {
        let (layer, dxvk, gpu, gm, ov, page) = (layer.clone(), dxvk.clone(), gpu.clone(), gm.clone(), ov.clone(), page.clone());
        Rc::new(move || {
            let s = crate::backend::game_launch::GameSettings {
                runner: if layer.selected() == 1 { Runner::Wine } else { Runner::Proton },
                dxvk: dxvk.is_active(),
                dedicated_gpu: gpu.is_active() && gpu.is_sensitive(),
                gamemode: gm.is_active() && gm.is_sensitive(),
                overlay: ov.is_active() && ov.is_sensitive(),
                ..windows_apps::defaults()
            };
            if let Err(e) = windows_apps::save(&s) {
                message(&page, "Couldn't save", &e);
            }
        })
    };
    {
        let (save, set_layer_subtitle, dxvk) = (save.clone(), set_layer_subtitle.clone(), dxvk.clone());
        layer.connect_selected_notify(move |l| {
            dxvk.set_sensitive(l.selected() == 0);
            set_layer_subtitle();
            save();
        });
    }
    dxvk.set_sensitive(layer.selected() == 0);
    for sw in [&dxvk, &gpu, &gm, &ov] {
        let save = save.clone();
        sw.connect_active_notify(move |_| save());
    }
    {
        let (page, set_on_subtitle) = (page.clone(), set_on_subtitle.clone());
        on.connect_active_notify(move |r| {
            if let Err(e) = windows_apps::set_enabled(r.is_active()) {
                message(&page, "Couldn't change the file types", &e);
            }
            set_on_subtitle();
        });
    }
    {
        let layer = layer.clone();
        open_c.connect_clicked(move |_| {
            let s = crate::backend::game_launch::GameSettings { runner: if layer.selected() == 1 { Runner::Wine } else { Runner::Proton }, ..windows_apps::defaults() };
            let dir = windows_apps::c_drive(&s);
            if !dir.exists() {
                // Not made yet: open the folder above it, which is made on the first run.
                let _ = std::fs::create_dir_all(dir.parent().unwrap_or(&dir));
            }
            let target = if dir.exists() { dir } else { dir.parent().map(|p| p.to_path_buf()).unwrap_or(dir) };
            let _ = Command::new("xdg-open").arg(target).spawn();
        });
    }

    for w in [on.upcast_ref::<gtk4::Widget>(), layer.upcast_ref(), dxvk.upcast_ref(), gpu.upcast_ref(), gm.upcast_ref(), ov.upcast_ref(), safe.upcast_ref(), cdrive.upcast_ref()] {
        g.add(w);
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

    root.append(&super::game_libraries::build());
    root.append(&super::my_games::build());
    root.append(&gamemode_group(&root));
    root.append(&mangohud_group(&root));
    if let Some(p) = power_group() {
        root.append(&p);
    }
    root.append(&live_group());
    root.append(&graphics_group());
    root.append(&windows_group(&root));
    root.append(&double_click_group(&root));
    root.append(&java_group(&root));
    root.append(&stores_group(&root));

    scroll.set_child(Some(&root));
    scroll.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_line_is_the_last_non_empty_one() {
        assert_eq!(last_error_line("a\nerror: no remote\n\n"), "error: no remote");
        assert_eq!(last_error_line(""), "");
        assert_eq!(last_error_line(&"x".repeat(500)).len(), 200);
    }
}

// ── Java for games ─────────────────────────────────────────────────────────

/// Gaming > Java: which Java is installed, which one Minecraft needs, install and switch the system Java.
fn java_group(root: &gtk4::Box) -> gtk4::Box {
    let holder = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    java_fill(&holder, root);
    holder
}

fn java_fill(holder: &gtk4::Box, root: &gtk4::Box) {
    use crate::backend::java_games as jg;
    while let Some(c) = holder.first_child() {
        holder.remove(&c);
    }
    let g = adw::PreferencesGroup::new();
    g.set_title("Java for Minecraft");
    g.set_description(Some("Minecraft needs a certain Java for each version: 1.20.5 and newer need Java 21, 1.18 to 1.20.4 need Java 17, older ones Java 8, and the newest year-numbered versions (26.x) a newer Java still. Launchers installed as Flatpaks (Modrinth, Prism) bring their own and cannot use these."));
    let loading = adw::ActionRow::new();
    loading.set_title("Looking for Java…");
    g.add(&loading);
    holder.append(&g);
    let (holder, root) = (holder.clone(), root.clone());
    in_background(
        move || (jg::system_javas(), jg::launcher_javas()),
        move |(system, launcher)| {
            while let Some(c) = holder.first_child() {
                holder.remove(&c);
            }
            let g = adw::PreferencesGroup::new();
            g.set_title("Java for Minecraft");
            g.set_description(Some("Minecraft needs a certain Java for each version: 1.20.5 and newer need Java 21, 1.18 to 1.20.4 need Java 17, older ones Java 8, and the newest year-numbered versions (26.x) a newer Java still. Launchers installed as Flatpaks (Modrinth, Prism) bring their own and cannot use these."));
            if system.is_empty() {
                let r = adw::ActionRow::new();
                r.set_title("No system Java installed");
                r.set_subtitle("Programs that are not Flatpaks (a Minecraft launcher from the AUR, a .jar file) need one. Install the version your game needs below.");
                r.set_subtitle_lines(0);
                g.add(&r);
            }
            for j in &system {
                let r = adw::ActionRow::new();
                r.set_title(&format!("Java {}", j.major));
                r.set_subtitle(if j.is_default { "The system's default Java" } else { "Installed" });
                if !j.is_default {
                    let b = gtk4::Button::with_label("Make default");
                    b.set_valign(gtk4::Align::Center);
                    let (name, all, holder, root) = (j.name.clone(), system.clone(), holder.clone(), root.clone());
                    b.connect_clicked(move |b| {
                        let script = match jg::default_script(&name, &all) {
                            Ok(s) => s,
                            Err(e) => return message(b, "Couldn't change it", &e),
                        };
                        b.set_sensitive(false);
                        let (holder, root, b) = (holder.clone(), root.clone(), b.clone());
                        in_background(move || jg::run_admin(&script), move |res| {
                            if let Err(e) = res {
                                message(&b, "Couldn't change it", &e);
                            }
                            java_fill(&holder, &root);
                        });
                    });
                    r.add_suffix(&b);
                }
                g.add(&r);
            }
            for (major, pkg) in jg::PACKAGES {
                if system.iter().any(|j| j.major == major) {
                    continue;
                }
                let r = adw::ActionRow::new();
                r.set_title(&format!("Install Java {major}"));
                r.set_subtitle(&format!("The system package {pkg}. Asks for your password."));
                let b = gtk4::Button::with_label("Install");
                b.set_valign(gtk4::Align::Center);
                let (holder, root) = (holder.clone(), root.clone());
                b.connect_clicked(move |b| {
                    let script = match jg::install_script(major) {
                        Ok(s) => s,
                        Err(e) => return message(b, "Couldn't install", &e),
                    };
                    b.set_sensitive(false);
                    b.set_label("Installing…");
                    let (holder, root, b) = (holder.clone(), root.clone(), b.clone());
                    in_background(move || jg::run_admin(&script), move |res| {
                        if let Err(e) = res {
                            message(&b, "Couldn't install", &e);
                        }
                        java_fill(&holder, &root);
                    });
                });
                r.add_suffix(&b);
                g.add(&r);
            }
            holder.append(&g);
            if !launcher.is_empty() {
                let l = adw::PreferencesGroup::new();
                l.set_title("Java that launchers brought");
                l.set_description(Some("Each launcher picks the right one for a game by itself. They are listed so you can see what is on this computer."));
                for j in &launcher {
                    let r = adw::ActionRow::new();
                    r.set_title(&format!("Java {}: {}", j.major, j.launcher));
                    r.set_subtitle(&glib::markup_escape_text(&j.path.to_string_lossy()));
                    r.set_subtitle_lines(1);
                    l.add(&r);
                }
                holder.append(&l);
            }
        },
    );
}
