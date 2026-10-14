//! The settings window for one Lutris or Heroic game (opened from Gaming > Your games). It changes the launcher's own
//! settings file for the game, so the choices hold however the game is started. The launcher must be closed while saving.

use crate::backend::game_launch::GameSettings;
use crate::backend::launcher_config as lc;
use crate::backend::worker::in_background;
use crate::backend::gpu_pref;
use crate::pages::game_libraries::Cfg;
use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use std::process::Command;

struct Loaded {
    settings: GameSettings,
    /// The file has something we cannot edit safely.
    unreadable: bool,
    gpu_env: Vec<(String, String)>,
}

fn flatpak(cfg: &Cfg) -> bool {
    match cfg {
        Cfg::Lutris { flatpak, .. } | Cfg::Heroic { flatpak, .. } => *flatpak,
    }
}

fn launcher_name(cfg: &Cfg) -> &'static str {
    match cfg {
        Cfg::Lutris { .. } => "Lutris",
        Cfg::Heroic { .. } => "Heroic",
    }
}

fn load(cfg: &Cfg) -> Loaded {
    let gpu_env = gpu_pref::dedicated_env();
    match cfg {
        Cfg::Lutris { file, .. } => {
            let text = std::fs::read_to_string(file).unwrap_or_default();
            match lc::lutris_read(&text, &gpu_env) {
                Some(settings) => Loaded { settings, unreadable: false, gpu_env },
                None => Loaded { settings: GameSettings::default(), unreadable: true, gpu_env },
            }
        }
        Cfg::Heroic { file, app, .. } => {
            let text = std::fs::read_to_string(file).unwrap_or_default();
            Loaded { settings: lc::heroic_read(&text, app, &gpu_env), unreadable: !text.trim().is_empty() && serde_json::from_str::<serde_json::Value>(&text).is_err(), gpu_env }
        }
    }
}

pub fn open(from: &impl IsA<gtk4::Widget>, name: &str, cfg: Cfg) {
    let Some(parent) = from.root().and_downcast::<gtk4::Window>() else { return };
    let win = adw::Window::builder().title(format!("{name}: settings")).transient_for(&parent).modal(true).default_width(560).default_height(600).build();
    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&adw::HeaderBar::new());
    let spin = gtk4::Spinner::new();
    spin.start();
    spin.set_halign(gtk4::Align::Center);
    spin.set_valign(gtk4::Align::Center);
    tv.set_content(Some(&spin));
    win.set_content(Some(&tv));
    win.present();
    let c2 = cfg.clone();
    in_background(move || load(&c2), move |l| fill(&win, &tv, cfg, l));
}

fn fill(win: &adw::Window, tv: &adw::ToolbarView, cfg: Cfg, l: Loaded) {
    let launcher = launcher_name(&cfg);
    let in_flatpak = flatpak(&cfg);
    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::new();
    group.set_title("How this game runs");
    group.set_description(Some(&format!(
        "{launcher}{}. These choices are saved in {launcher}'s own settings for the game, so they hold however you start it. Close {launcher} before you save.",
        if in_flatpak { " (Flatpak)" } else { "" }
    )));
    if l.unreadable {
        let note = adw::ActionRow::new();
        note.set_title("This game's settings file looks unusual");
        note.set_subtitle(&format!("{launcher} wrote it in a way Zohara does not change, so it is left alone. Change this game inside {launcher}."));
        note.set_subtitle_lines(0);
        group.add(&note);
    }
    let cur = l.settings.clone();
    let is_lutris_wine = matches!(cfg, Cfg::Lutris { .. });

    let gpu = adw::ComboRow::new();
    gpu.set_title("Graphics chip");
    gpu.set_subtitle_lines(0);
    gpu.set_model(Some(&gtk4::StringList::new(&["Automatic", "Dedicated"])));
    gpu.set_selected(cur.dedicated_gpu as u32);
    if !gpu_pref::has_two_gpus() || l.gpu_env.is_empty() {
        gpu.set_sensitive(false);
        gpu.set_subtitle("This computer has one graphics chip, so there is nothing to choose.");
    } else {
        gpu.set_subtitle("Automatic lets the system decide. On a laptop with two chips, Dedicated runs the game on the stronger one.");
    }
    let dxvk = adw::SwitchRow::new();
    dxvk.set_title("DirectX through Vulkan (DXVK)");
    dxvk.set_subtitle(if is_lutris_wine { "Windows games only. Faster for most games; turn it off if a game shows glitches." } else { "Windows games only. Faster for most games; turn it off if a game shows glitches." });
    dxvk.set_subtitle_lines(0);
    dxvk.set_active(cur.dxvk);

    let has = |cmd: &str| !in_flatpak && Command::new("sh").args(["-c", &format!("command -v {cmd}")]).output().map(|o| o.status.success()).unwrap_or(false);
    let gm = adw::SwitchRow::new();
    gm.set_title("Game Mode");
    gm.set_subtitle(if has("gamemoderun") { "Give the game priority while it runs" } else { "Not available for this launcher" });
    gm.set_active(cur.gamemode); // shown as it is, even when it cannot be changed here
    gm.set_sensitive(has("gamemoderun"));
    let ov = adw::SwitchRow::new();
    ov.set_title("Performance overlay (MangoHud)");
    ov.set_subtitle(if has("mangohud") { "Shows frame rate and load on screen while playing" } else { "Not available for this launcher" });
    ov.set_active(cur.overlay);
    ov.set_sensitive(has("mangohud"));
    let options = adw::EntryRow::new();
    options.set_title("Game options (for example -windowed)");
    options.set_text(&cur.options);
    let env = adw::EntryRow::new();
    env.set_title("Environment variables (NAME=value NAME2=value)");
    env.set_text(&cur.env);
    for r in [gpu.clone().upcast::<gtk4::Widget>(), dxvk.clone().upcast(), gm.clone().upcast(), ov.clone().upcast(), options.clone().upcast(), env.clone().upcast()] {
        r.set_sensitive(r.is_sensitive() && !l.unreadable);
        group.add(&r);
    }
    page.add(&group);

    let status = gtk4::Label::new(None);
    status.set_wrap(true);
    status.set_halign(gtk4::Align::Start);
    status.set_hexpand(true);
    status.add_css_class("dim-label");
    let running = match cfg {
        Cfg::Lutris { .. } => lc::launcher_running("lutris"),
        Cfg::Heroic { .. } => lc::launcher_running("heroic"),
    };
    if running {
        status.set_text(&format!("{launcher} is open. Close it completely before you press Save."));
    }
    let save = gtk4::Button::with_label("Save");
    save.add_css_class("suggested-action");
    save.set_sensitive(!l.unreadable);
    let cancel = gtk4::Button::with_label("Cancel");
    let w2 = win.clone();
    cancel.connect_clicked(move |_| w2.close());
    {
        let (win, status, cfg, gpu_env) = (win.clone(), status.clone(), cfg.clone(), l.gpu_env.clone());
        let (dxvk, gpu, gm, ov, options, env) = (dxvk.clone(), gpu.clone(), gm.clone(), ov.clone(), options.clone(), env.clone());
        save.connect_clicked(move |b| {
            let s = GameSettings {
                dxvk: dxvk.is_active(),
                gamemode: gm.is_active(),
                overlay: ov.is_active(),
                dedicated_gpu: gpu.selected() == 1,
                options: options.text().to_string(),
                env: env.text().to_string(),
                ..GameSettings::default()
            };
            b.set_sensitive(false);
            status.set_text("Saving…");
            let (win, status, cfg, gpu_env, b) = (win.clone(), status.clone(), cfg.clone(), gpu_env.clone(), b.clone());
            in_background(
                move || match &cfg {
                    Cfg::Lutris { file, .. } => lc::lutris_apply(file, &s, &gpu_env),
                    Cfg::Heroic { file, app, .. } => lc::heroic_apply(file, app, &s, &gpu_env),
                },
                move |res| match res {
                    Ok(()) => win.close(),
                    Err(e) => {
                        status.set_text(&e);
                        b.set_sensitive(true);
                    }
                },
            );
        });
    }
    let bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    bar.set_margin_top(8);
    bar.set_margin_bottom(12);
    bar.set_margin_start(18);
    bar.set_margin_end(18);
    bar.append(&status);
    bar.append(&cancel);
    bar.append(&save);
    tv.set_content(Some(&page));
    tv.add_bottom_bar(&bar);
}
