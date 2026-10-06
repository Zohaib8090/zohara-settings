use gtk4::prelude::*;
use libadwaita as adw;
use adw::prelude::*;

use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::rc::Rc;


/// "Pause updates" state. There's no background auto-update daemon on
/// Zohara OS yet -- this page's "Check for updates" is manual-only -- so
/// today this only gates the hero banner (honest: it says you're paused,
/// it doesn't yet stop anything from happening automatically, because
/// nothing does that automatically). If a background checker is ever
/// added, it needs to read this same file before firing.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PauseState {
    /// Unix timestamp (seconds) the pause ends, if any.
    paused_until: Option<i64>,
}

fn pause_state_path() -> std::path::PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
            std::path::PathBuf::from(home).join(".config")
        });
    base.join("zohara").join("update-pause.json")
}

fn load_pause_state() -> PauseState {
    std::fs::read_to_string(pause_state_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_pause_state(state: &PauseState) {
    let path = pause_state_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(s) = serde_json::to_string_pretty(state) {
        let _ = std::fs::write(path, s);
    }
}

/// Seconds since the Unix epoch, without pulling in a time crate.
fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn active_pause_message() -> Option<String> {
    let state = load_pause_state();
    let until = state.paused_until?;
    let remaining = until - unix_now();
    if remaining <= 0 {
        return None;
    }
    let days = (remaining as f64 / 86400.0).ceil() as i64;
    Some(format!(
        "Updates paused — {days} day{} remaining",
        if days == 1 { "" } else { "s" }
    ))
}

pub fn build() -> gtk4::Widget {
    let scroll = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .build();

    let root_box = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
    root_box.set_margin_start(28);
    root_box.set_margin_end(28);
    root_box.set_margin_top(20);
    root_box.set_margin_bottom(32);

    // ── Page Title ──────────────────────────────────────────────────────────
    let title_lbl = gtk4::Label::builder()
        .label("Zohara Update")
        .halign(gtk4::Align::Start)
        .css_classes(vec!["win11-page-title".to_string()])
        .build();
    root_box.append(&title_lbl);

    // The operating system's updates, right here: check, update, undo, restore points and the update channel.
    // (Apps from Flathub are updated in Zohara Store, a separate program.)
    let to_end = {
        let scroll = scroll.clone();
        move || {
            let adj = scroll.vadjustment();
            adj.set_value(adj.upper());
        }
    };
    if let Some(msg) = active_pause_message() {
        let pause_banner = adw::Banner::new(&msg);
        pause_banner.set_revealed(true);
        root_box.append(&pause_banner);
    }
    root_box.append(&crate::sysupdate::ui::build_content(to_end));

    // ── Section: More options ────────────────────────────────────────────────
    let more_lbl = gtk4::Label::builder()
        .label("More options")
        .halign(gtk4::Align::Start)
        .css_classes(vec!["win11-section-header".to_string()])
        .build();
    root_box.append(&more_lbl);

    let more_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    more_box.set_css_classes(&["win11-card-group"]);

    // Pause updates row — persists a real "paused until" timestamp
    // (~/.config/zohara/update-pause.json). See PauseState's doc comment
    // for what this does and doesn't gate today.
    let pause_row = adw::ActionRow::new();
    pause_row.set_title("Pause updates");
    pause_row.set_subtitle("Select the duration to pause automatic updates");
    pause_row.add_prefix(&gtk4::Image::from_icon_name("media-playback-pause-symbolic"));
    let pause_combo = gtk4::DropDown::from_strings(&[
        "Don't pause",
        "Pause for 1 week",
        "Pause for 2 weeks",
        "Pause for 3 weeks",
        "Pause for 4 weeks",
    ]);
    pause_combo.set_valign(gtk4::Align::Center);
    if let Some(msg) = active_pause_message() {
        pause_row.set_subtitle(&msg);
    }
    {
        let pause_row_clone = pause_row.clone();
        pause_combo.connect_selected_notify(move |dd| {
            let weeks: i64 = match dd.selected() {
                1 => 1,
                2 => 2,
                3 => 3,
                4 => 4,
                _ => 0,
            };
            let state = if weeks == 0 {
                PauseState { paused_until: None }
            } else {
                PauseState { paused_until: Some(unix_now() + weeks * 7 * 86400) }
            };
            save_pause_state(&state);
            pause_row_clone.set_subtitle(
                &active_pause_message().unwrap_or_else(|| {
                    "Select the duration to pause automatic updates".to_string()
                }),
            );
        });
    }
    pause_row.add_suffix(&pause_combo);
    pause_row.set_css_classes(&["win11-expander-row"]);
    more_box.append(&pause_row);

    // Update History — real: tails /var/log/pacman.log for upgrade/install
    // lines instead of being a dead chevron.
    let hist_row = build_action_row(
        "Update history",
        "View installed packages and system upgrade logs",
        "document-open-recent-symbolic",
    );
    hist_row.connect_activated(open_update_history_window);
    more_box.append(&super::in_list(&hist_row));

    // Advanced Options — real: shows the current channel and package
    // cache size, with a safe cache cleanup action.
    let adv_row = build_action_row(
        "Advanced options",
        "Update channel, package cache size and cleanup",
        "preferences-system-symbolic",
    );
    adv_row.connect_activated(open_advanced_update_window);
    more_box.append(&super::in_list(&adv_row));

    // Zohara Insider Program — REMOVED. There is no insider program; the
    // previous UI implied one existed and clicking the row did nothing.
    root_box.append(&more_box);

    // ── Section: Related support ─────────────────────────────────────────────
    let support_lbl = gtk4::Label::builder()
        .label("Related support")
        .halign(gtk4::Align::Start)
        .css_classes(vec!["win11-section-header".to_string()])
        .build();
    root_box.append(&support_lbl);

    let support_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    support_box.set_css_classes(&["win11-card-group"]);

    let help_exp = adw::ExpanderRow::new();
    help_exp.set_title("Help with Zohara Update");
    help_exp.add_prefix(&gtk4::Image::from_icon_name("help-browser-symbolic"));
    help_exp.set_css_classes(&["win11-expander-row"]);

    let help_topics: &[(&str, fn(&adw::ActionRow))] = &[
        ("Troubleshooting package download errors", open_help_download_errors),
        ("Rolling back or downgrading a package", open_help_rollback),
        ("Configuring custom Arch mirrors in pacman.conf", open_help_mirrors),
        ("Cleaning up package cache safely", open_help_cache_cleanup),
    ];
    for (title, handler) in help_topics {
        let r = adw::ActionRow::new();
        r.set_title(title);
        r.add_suffix(&gtk4::Image::from_icon_name("go-next-symbolic"));
        r.set_activatable(true);
        r.connect_activated(*handler);
        help_exp.add_row(&r);
    }
    support_box.append(&help_exp);
    root_box.append(&support_box);

    scroll.set_child(Some(&root_box));
    scroll.upcast()
}


fn build_action_row(title: &str, subtitle: &str, icon_name: &str) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(title);
    row.set_subtitle(subtitle);
    row.add_prefix(&gtk4::Image::from_icon_name(icon_name));
    row.add_suffix(&gtk4::Image::from_icon_name("go-next-symbolic"));
    row.set_css_classes(&["win11-expander-row"]);
    row.set_activatable(true);
    row
}

/// Opens `content` as a small modal window over whatever window `row`
/// lives in. Same pattern as personalization.rs's open_settings_window.
fn open_settings_window(row: &adw::ActionRow, title: &str, content: &gtk4::Box) {
    let Some(parent) = row.root().and_downcast::<gtk4::Window>() else {
        return;
    };
    let win = gtk4::Window::builder()
        .title(title)
        .transient_for(&parent)
        .modal(true)
        .default_width(460)
        .default_height(360)
        .build();
    let scroll = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .build();
    scroll.set_child(Some(content));
    win.set_child(Some(&scroll));
    win.present();
}

fn dialog_content_box() -> gtk4::Box {
    let b = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    b.set_margin_top(16);
    b.set_margin_bottom(16);
    b.set_margin_start(16);
    b.set_margin_end(16);
    b
}

fn info_label(text: &str) -> gtk4::Label {
    let l = gtk4::Label::new(Some(text));
    l.set_wrap(true);
    l.set_halign(gtk4::Align::Start);
    l
}

/// Tails /var/log/pacman.log for upgrade/install lines -- real system
/// history, not a placeholder. pacman's log format
/// ("[2024-01-01T12:00:00+0000] [ALPM] upgraded pkg (old -> new)") is
/// stable and well documented.
fn open_update_history_window(row: &adw::ActionRow) {
    let content = dialog_content_box();
    let entries: Vec<String> = std::fs::read_to_string("/var/log/pacman.log")
        .map(|log| {
            log.lines()
                .filter(|l| l.contains("[ALPM] upgraded") || l.contains("[ALPM] installed"))
                .rev()
                .take(30)
                .map(|l| {
                    // "[2024-01-01T12:00:00+0000] [ALPM] upgraded firefox (120.0-1 -> 121.0-1)"
                    let after_bracket = l.splitn(3, "] ").nth(2).unwrap_or(l);
                    let date = l.split(']').next().unwrap_or("").trim_start_matches('[');
                    format!("{date}  —  {after_bracket}")
                })
                .collect()
        })
        .unwrap_or_default();

    if entries.is_empty() {
        content.append(&info_label(
            "No upgrade history found in /var/log/pacman.log yet.",
        ));
    } else {
        for entry in entries {
            content.append(&info_label(&entry));
        }
    }
    open_settings_window(row, "Update History", &content);
}

/// Shows the real current channel and pacman package cache size, with a
/// safe cleanup action. Deliberately does NOT offer `pacman -Sc`/`-Scc`:
/// those remove the very cached package versions zohara-store's "restore
/// previous version" feature relies on. `paccache -rk1` keeps one old
/// version per package -- safe for rollback, still reclaims space.
fn open_advanced_update_window(row: &adw::ActionRow) {
    let content = dialog_content_box();

    let channel_row = adw::ActionRow::new();
    channel_row.set_title("Update channel");
    channel_row.set_subtitle("Checking…");
    content.append(&channel_row);
    {
        let channel_row_c = channel_row.clone();
        glib::spawn_future_local(async move {
            let out = tokio::process::Command::new("zohara-channel")
                .arg("current")
                .output()
                .await
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .unwrap_or_else(|| "unknown".to_string());
            channel_row_c.set_subtitle(out.trim());
        });
    }

    let cache_row = adw::ActionRow::new();
    cache_row.set_title("Package cache size");
    cache_row.set_subtitle("Checking…");
    content.append(&cache_row);
    {
        let cache_row_c = cache_row.clone();
        glib::spawn_future_local(async move {
            let out = tokio::process::Command::new("du")
                .args(["-sh", "/var/cache/pacman/pkg"])
                .output()
                .await
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .and_then(|s| s.split_whitespace().next().map(str::to_string))
                .unwrap_or_else(|| "unknown".to_string());
            cache_row_c.set_subtitle(&out);
        });
    }

    let cleanup_btn = gtk4::Button::builder()
        .label("Clean up now (keeps 1 old version per package)")
        .css_classes(vec!["win11-secondary-btn".to_string()])
        .build();
    let cache_row_for_cleanup = cache_row.clone();
    cleanup_btn.connect_clicked(move |btn| {
        btn.set_sensitive(false);
        let btn_c = btn.clone();
        let cache_row_c = cache_row_for_cleanup.clone();
        glib::spawn_future_local(async move {
            let has_paccache = tokio::process::Command::new("sh")
                .args(["-c", "command -v paccache"])
                .status()
                .await
                .map(|s| s.success())
                .unwrap_or(false);
            if !has_paccache {
                btn_c.set_label("Install pacman-contrib for cleanup");
                return;
            }
            let ok = tokio::process::Command::new("pkexec")
                .args(["paccache", "-rk1"])
                .status()
                .await
                .map(|s| s.success())
                .unwrap_or(false);
            btn_c.set_label(if ok { "Cleaned up" } else { "Cleanup failed" });
            if ok {
                if let Ok(out) = tokio::process::Command::new("du")
                    .args(["-sh", "/var/cache/pacman/pkg"])
                    .output()
                    .await
                {
                    if let Ok(s) = String::from_utf8(out.stdout) {
                        if let Some(size) = s.split_whitespace().next() {
                            cache_row_c.set_subtitle(size);
                        }
                    }
                }
            }
        });
    });
    content.append(&cleanup_btn);

    open_settings_window(row, "Advanced Options", &content);
}

fn open_help_download_errors(row: &adw::ActionRow) {
    let content = dialog_content_box();
    content.append(&info_label(
        "If package downloads keep failing, it's almost always a mirror \
         problem: run `sudo pacman -Syyu` to force-refresh the databases, \
         or switch mirrors under Advanced Options \u{2192} update channel. \
         Check `journalctl -xe` for the specific pacman error if it keeps happening.",
    ));
    open_settings_window(row, "Troubleshooting Downloads", &content);
}

fn open_help_rollback(row: &adw::ActionRow) {
    let content = dialog_content_box();
    content.append(&info_label(
        "Every package version pacman installs stays cached in \
         /var/cache/pacman/pkg/ until it's cleaned up. To roll back a \
         package: sudo pacman -U /var/cache/pacman/pkg/<pkg>-<old-version>.pkg.tar.zst\n\n\
         For Zohara's own apps (Settings, Store, Link), the Zohara Store's \
         Updates tab has a one-click \u{201c}Restore previous\u{201d} button \
         that does exactly this, plus re-downloads the exact old version \
         from GitHub if it's no longer in the local cache.",
    ));
    open_settings_window(row, "Rolling Back a Package", &content);
}

fn open_help_mirrors(row: &adw::ActionRow) {
    let content = dialog_content_box();
    content.append(&info_label(
        "Custom mirrors are configured in /etc/pacman.d/mirrorlist, which \
         needs root to edit. Open a terminal and run:\n\n\
         sudo nano /etc/pacman.d/mirrorlist\n\n\
         Move faster/closer mirrors near the top of the file — pacman \
         tries them in order.",
    ));
    open_settings_window(row, "Configuring Mirrors", &content);
}

fn open_help_cache_cleanup(row: &adw::ActionRow) {
    let content = dialog_content_box();
    content.append(&info_label(
        "The safe way to clean the package cache is `paccache -rk1` (keeps \
         one old version of every package, so you can still roll back one \
         step). Avoid `pacman -Sc`/`-Scc` — those remove the cached \
         versions rollback depends on entirely.\n\n\
         Advanced Options on this page has a \u{201c}Clean up now\u{201d} \
         button that runs the safe command for you.",
    ));
    open_settings_window(row, "Cleaning Up the Package Cache", &content);
}
