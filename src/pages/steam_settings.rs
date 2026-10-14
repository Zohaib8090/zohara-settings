//! The settings window for one Steam game (opened from Gaming > Your games). It changes the game's launch options and
//! Proton tool in Steam's own files, so the choices hold whether the game is started from Steam, from here, or from
//! the Start menu. Steam must be closed while saving (Steam rewrites those files when it quits).

use crate::backend::game_launch::{GameSettings, Runner};
use crate::backend::steam_games::{self, SteamGame};
use crate::backend::worker::in_background;
use crate::backend::{gpu_pref, proton};
use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use std::process::Command;

/// What was in the game's settings when the window opened (read once, off the window thread).
struct Loaded {
    launch: String,
    tool: String,
    gpu_env: Vec<(String, String)>,
}

pub fn open(from: &impl IsA<gtk4::Widget>, game: SteamGame) {
    let Some(parent) = from.root().and_downcast::<gtk4::Window>() else { return };
    let win = adw::Window::builder().title(format!("{}: settings", game.name)).transient_for(&parent).modal(true).default_width(560).default_height(640).build();
    let tv = adw::ToolbarView::new();
    tv.add_top_bar(&adw::HeaderBar::new());
    let spin = gtk4::Spinner::new();
    spin.start();
    spin.set_halign(gtk4::Align::Center);
    spin.set_valign(gtk4::Align::Center);
    tv.set_content(Some(&spin));
    win.set_content(Some(&tv));
    win.present();
    let g2 = game.clone();
    in_background(
        move || Loaded { launch: steam_games::launch_options(&g2), tool: steam_games::compat_tool(&g2), gpu_env: gpu_pref::dedicated_env() },
        move |l| fill(&win, &tv, game, l),
    );
}

fn fill(win: &adw::Window, tv: &adw::ToolbarView, game: SteamGame, l: Loaded) {
    let parsed = steam_games::parse_launch(&l.launch, &l.gpu_env);
    let mut current = parsed.clone().unwrap_or_default();
    let ge_now = l.tool.starts_with("GE-Proton");
    // a Proton the person picked in Steam (not Proton-GE) is theirs: shown, not changed
    let foreign_tool = !l.tool.is_empty() && !ge_now;
    if ge_now {
        current.runner = Runner::Proton;
    }

    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::new();
    group.set_title("How this game runs");
    group.set_description(Some(if game.root.flatpak {
        "Steam (Flatpak). Game Mode and the overlay are not inside Flatpak's sandbox yet, so only the other choices apply."
    } else {
        "Steam. These choices are saved in Steam's launch options, so they also apply when you start the game from Steam."
    }));

    if parsed.is_none() {
        let note = adw::ActionRow::new();
        note.set_title("This game has launch options of your own");
        note.set_subtitle(&glib::markup_escape_text(&format!("\"{}\" Saving here replaces them.", l.launch)));
        note.set_subtitle_lines(0);
        group.add(&note);
    }

    let installed = proton::installed();
    let run_row = adw::ComboRow::new();
    run_row.set_title("Run with");
    run_row.set_subtitle_lines(0);
    let ge = installed.first().cloned();
    let labels: Vec<String> = vec!["Steam's default".into(), match &ge {
        Some(v) => format!("Proton-GE ({v})"),
        None => "Proton-GE (not installed)".into(),
    }];
    run_row.set_model(Some(&gtk4::StringList::new(&labels.iter().map(|s| s.as_str()).collect::<Vec<_>>())));
    run_row.set_selected((current.runner == Runner::Proton) as u32);
    if foreign_tool {
        run_row.set_sensitive(false);
        run_row.set_subtitle(&glib::markup_escape_text(&format!("You picked {} in Steam. Change it there.", l.tool)));
    } else if ge.is_none() {
        run_row.set_subtitle("Install Proton-GE in Gaming > Windows games first.");
    } else {
        run_row.set_subtitle("Proton-GE fixes more Windows games than Steam's own. Native Linux games ignore this.");
    }

    let dxvk = adw::SwitchRow::new();
    dxvk.set_title("DirectX through Vulkan (DXVK)");
    dxvk.set_subtitle("Proton only. Faster for most games; turn it off if a game shows glitches.");
    dxvk.set_subtitle_lines(0);
    dxvk.set_active(current.dxvk);
    let sync = {
        let (dxvk, run_row) = (dxvk.clone(), run_row.clone());
        move || dxvk.set_sensitive(run_row.is_sensitive() && run_row.selected() == 1)
    };
    sync();
    run_row.connect_selected_notify(move |_| sync());

    let gpu = adw::ComboRow::new();
    gpu.set_title("Graphics chip");
    gpu.set_subtitle_lines(0);
    gpu.set_model(Some(&gtk4::StringList::new(&["Automatic", "Dedicated"])));
    gpu.set_selected(current.dedicated_gpu as u32);
    if !gpu_pref::has_two_gpus() || l.gpu_env.is_empty() {
        gpu.set_sensitive(false);
        gpu.set_subtitle("This computer has one graphics chip, so there is nothing to choose.");
    } else {
        gpu.set_subtitle("Automatic lets the system decide. On a laptop with two chips, Dedicated runs the game on the stronger one.");
    }

    let has = |cmd: &str| !game.root.flatpak && Command::new("sh").args(["-c", &format!("command -v {cmd}")]).output().map(|o| o.status.success()).unwrap_or(false);
    let gm = adw::SwitchRow::new();
    gm.set_title("Game Mode");
    gm.set_subtitle(if has("gamemoderun") { "Give the game priority while it runs" } else { "Not available for this Steam" });
    gm.set_active(current.gamemode && has("gamemoderun"));
    gm.set_sensitive(has("gamemoderun"));
    let ov = adw::SwitchRow::new();
    ov.set_title("Performance overlay (MangoHud)");
    ov.set_subtitle(if has("mangohud") { "Shows frame rate and load on screen while playing" } else { "Not available for this Steam" });
    ov.set_active(current.overlay && has("mangohud"));
    ov.set_sensitive(has("mangohud"));

    let options = adw::EntryRow::new();
    options.set_title("Game options (for example -windowed)");
    options.set_text(&current.options);
    let env = adw::EntryRow::new();
    env.set_title("Environment variables (NAME=value NAME2=value)");
    env.set_text(&current.env);
    for r in [run_row.clone().upcast::<gtk4::Widget>(), dxvk.clone().upcast(), gpu.clone().upcast(), gm.clone().upcast(), ov.clone().upcast(), options.clone().upcast(), env.clone().upcast()] {
        group.add(&r);
    }
    page.add(&group);

    let status = gtk4::Label::new(None);
    status.set_wrap(true);
    status.set_halign(gtk4::Align::Start);
    status.set_hexpand(true);
    status.add_css_class("dim-label");
    if steam_games::steam_running() {
        status.set_text("Steam is running. Close it (Steam > Exit) before you press Save.");
    }
    let save = gtk4::Button::with_label("Save");
    save.add_css_class("suggested-action");
    let cancel = gtk4::Button::with_label("Cancel");
    let w2 = win.clone();
    cancel.connect_clicked(move |_| w2.close());
    {
        let (win, status, game, gpu_env, tool_now) = (win.clone(), status.clone(), game.clone(), l.gpu_env.clone(), l.tool.clone());
        let (run_row, dxvk, gpu, gm, ov, options, env) = (run_row.clone(), dxvk.clone(), gpu.clone(), gm.clone(), ov.clone(), options.clone(), env.clone());
        save.connect_clicked(move |b| {
            let proton_chosen = run_row.is_sensitive() && run_row.selected() == 1;
            let tool = if foreign_tool {
                tool_now.clone()
            } else if proton_chosen {
                match proton::installed().first() {
                    Some(t) => t.clone(),
                    None => {
                        status.set_text("Proton-GE is not installed. Install it in Gaming > Windows games first.");
                        return;
                    }
                }
            } else {
                String::new()
            };
            let settings = GameSettings {
                runner: if proton_chosen { Runner::Proton } else { Runner::Auto },
                dxvk: dxvk.is_active(),
                gamemode: gm.is_active(),
                overlay: ov.is_active(),
                dedicated_gpu: gpu.selected() == 1,
                options: options.text().to_string(),
                env: env.text().to_string(),
            };
            let launch = match steam_games::launch_text(&settings, &gpu_env) {
                Ok(t) => t,
                Err(e) => {
                    status.set_text(&e);
                    return;
                }
            };
            b.set_sensitive(false);
            status.set_text("Saving…");
            let (win, status, game, b) = (win.clone(), status.clone(), game.clone(), b.clone());
            in_background(move || steam_games::apply(&game, &launch, &tool), move |res| match res {
                Ok(()) => win.close(),
                Err(e) => {
                    status.set_text(&e);
                    b.set_sensitive(true);
                }
            });
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
