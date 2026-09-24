//! Storage: real drives and partitions (lsblk) with mount / eject for
//! removable media (udisks2), what's using space in the home folder, and
//! cleanup of trash, app caches and the pacman package cache. Storage Sense
//! uses Plasma's own trash time limit plus a systemd user tmpfiles rule for
//! ~/.cache, so the cleanup keeps running without Settings being open.

use crate::backend::kconfig;
use crate::backend::worker::in_background;
use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
}

fn trash_dir() -> PathBuf {
    let data = std::env::var("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|_| home().join(".local/share"));
    data.join("Trash")
}

fn cache_dir() -> PathBuf {
    std::env::var("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|_| home().join(".cache"))
}

fn tmpfiles_rule() -> PathBuf {
    let cfg = std::env::var("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|_| home().join(".config"));
    cfg.join("user-tmpfiles.d").join("zohara-storage-sense.conf")
}

fn human(bytes: u64) -> String {
    let b = bytes as f64;
    match bytes {
        0..=1023 => format!("{bytes} B"),
        1024..=1_048_575 => format!("{:.0} KB", b / 1024.0),
        1_048_576..=1_073_741_823 => format!("{:.1} MB", b / 1_048_576.0),
        _ => format!("{:.1} GB", b / 1_073_741_824.0),
    }
}

fn du(path: &Path) -> u64 {
    if !path.exists() {
        return 0;
    }
    Command::new("du")
        .args(["-sb", "--"])
        .arg(path)
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).split_whitespace().next()?.parse().ok())
        .unwrap_or(0)
}

fn run_ok(cmd: &str, args: &[&str]) -> bool {
    Command::new(cmd).args(args).status().map(|s| s.success()).unwrap_or(false)
}

fn error_dialog(w: &impl IsA<gtk4::Widget>, title: &str, body: &str) {
    let d = adw::AlertDialog::new(Some(title), Some(body));
    d.add_response("ok", "OK");
    d.present(Some(w));
}

/// Ask for confirmation, then run `action` on a worker thread.
fn confirm(
    w: &impl IsA<gtk4::Widget>,
    title: &str,
    body: &str,
    verb: &str,
    action: impl Fn() + Send + Clone + 'static,
    after: impl Fn() + Clone + 'static,
) {
    let d = adw::AlertDialog::new(Some(title), Some(body));
    d.add_responses(&[("cancel", "Cancel"), ("go", verb)]);
    d.set_response_appearance("go", adw::ResponseAppearance::Destructive);
    d.set_default_response(Some("cancel"));
    d.connect_response(None, move |_, r| {
        if r == "go" {
            let (action, after) = (action.clone(), after.clone());
            in_background(action, move |_| after());
        }
    });
    d.present(Some(w));
}

// ── Drives ─────────────────────────────────────────────────────────────────

fn lsblk() -> Vec<Value> {
    Command::new("lsblk")
        .args(["-J", "-b", "-o", "NAME,PATH,TYPE,SIZE,FSTYPE,LABEL,MOUNTPOINT,FSSIZE,FSUSED,RM,HOTPLUG,TRAN,MODEL"])
        .output()
        .ok()
        .and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok())
        .and_then(|v| v["blockdevices"].as_array().cloned())
        .unwrap_or_default()
}

fn num(v: &Value) -> u64 {
    v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())).unwrap_or(0)
}

fn flag(v: &Value) -> bool {
    v.as_bool().unwrap_or_else(|| v.as_str() == Some("1") || v.as_u64() == Some(1))
}

fn partitions(disk: &Value) -> Vec<Value> {
    let children = disk["children"].as_array().cloned().unwrap_or_default();
    if children.is_empty() && !disk["fstype"].is_null() {
        vec![disk.clone()]
    } else {
        children.into_iter().filter(|c| !c["fstype"].is_null() && c["fstype"].as_str() != Some("swap")).collect()
    }
}

fn drives_group(page: &gtk4::Box) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("Drives");
    let refresh = gtk4::Button::from_icon_name("view-refresh-symbolic");
    refresh.add_css_class("flat");
    refresh.set_tooltip_text(Some("Refresh"));
    g.set_header_suffix(Some(&refresh));

    let rows: std::rc::Rc<std::cell::RefCell<Vec<gtk4::Widget>>> = Default::default();
    let fill = {
        let (g, rows, page) = (g.clone(), rows.clone(), page.clone());
        std::rc::Rc::new(move || {
            for r in rows.borrow_mut().drain(..) {
                g.remove(&r);
            }
            let disks: Vec<Value> = lsblk()
                .into_iter()
                .filter(|d| matches!(d["type"].as_str(), Some("disk")) && !d["name"].as_str().unwrap_or("").starts_with("zram"))
                .collect();
            if disks.is_empty() {
                let r = adw::ActionRow::new();
                r.set_title("No drives detected");
                g.add(&r);
                rows.borrow_mut().push(r.upcast());
            }
            for disk in disks {
                let removable = flag(&disk["rm"]) || flag(&disk["hotplug"]) || disk["tran"].as_str() == Some("usb");
                let model = disk["model"].as_str().unwrap_or("").trim().to_string();
                let exp = adw::ExpanderRow::new();
                exp.set_title(&glib::markup_escape_text(if model.is_empty() { disk["name"].as_str().unwrap_or("Drive") } else { &model }));
                exp.set_subtitle(&format!(
                    "{} · {}",
                    human(num(&disk["size"])),
                    if removable { "Removable" } else { "Internal" }
                ));
                exp.add_prefix(&gtk4::Image::from_icon_name(if removable {
                    "drive-removable-media-symbolic"
                } else {
                    "drive-harddisk-symbolic"
                }));
                exp.set_expanded(true);

                for part in partitions(&disk) {
                    let dev = part["path"].as_str().unwrap_or_default().to_string();
                    let mount = part["mountpoint"].as_str().map(str::to_string);
                    let label = part["label"].as_str().filter(|l| !l.is_empty()).map(str::to_string);
                    let row = adw::ActionRow::new();
                    row.set_title(&glib::markup_escape_text(&match (&label, &mount) {
                        (Some(l), _) => l.clone(),
                        (None, Some(m)) => m.clone(),
                        _ => dev.clone(),
                    }));
                    let fs = part["fstype"].as_str().unwrap_or("");
                    match &mount {
                        Some(m) => {
                            let (size, used) = (num(&part["fssize"]), num(&part["fsused"]));
                            row.set_subtitle(&format!(
                                "{} free of {} · {fs} · {}",
                                human(size.saturating_sub(used)),
                                human(size),
                                glib::markup_escape_text(m)
                            ));
                            if size > 0 {
                                let bar = gtk4::LevelBar::for_interval(0.0, 1.0);
                                bar.set_value(used as f64 / size as f64);
                                bar.set_size_request(120, -1);
                                bar.set_valign(gtk4::Align::Center);
                                row.add_suffix(&bar);
                            }
                            let open = gtk4::Button::from_icon_name("folder-open-symbolic");
                            open.add_css_class("flat");
                            open.set_valign(gtk4::Align::Center);
                            open.set_tooltip_text(Some("Open in file manager"));
                            let m2 = m.clone();
                            open.connect_clicked(move |_| {
                                let _ = Command::new("xdg-open").arg(&m2).spawn();
                            });
                            row.add_suffix(&open);
                        }
                        None => row.set_subtitle(&format!("{} · {fs} · Not mounted", human(num(&part["size"])))),
                    }

                    // Only offer mount/unmount for removable media; system partitions stay untouched.
                    if removable {
                        let btn = gtk4::Button::with_label(if mount.is_some() { "Unmount" } else { "Mount" });
                        btn.set_valign(gtk4::Align::Center);
                        let (dev2, mounted, fill_page) = (dev.clone(), mount.is_some(), page.clone());
                        btn.connect_clicked(move |b| {
                            b.set_sensitive(false);
                            let dev3 = dev2.clone();
                            let page2 = fill_page.clone();
                            in_background(
                                move || {
                                    let verb = if mounted { "unmount" } else { "mount" };
                                    Command::new("udisksctl")
                                        .args([verb, "-b", &dev3])
                                        .output()
                                        .map(|o| (o.status.success(), String::from_utf8_lossy(&o.stderr).trim().to_string()))
                                        .unwrap_or((false, "udisksctl is not available".into()))
                                },
                                move |(ok, err)| {
                                    if !ok {
                                        error_dialog(&page2, "Couldn't change the drive", if err.contains("busy") {
                                            "Something is still using this drive. Close any files or windows from it and try again."
                                        } else {
                                            err.as_str()
                                        });
                                    }
                                    page2.activate_action("storage.refresh", None).ok();
                                },
                            );
                        });
                        row.add_suffix(&btn);
                    }
                    exp.add_row(&row);
                }

                if removable {
                    let eject = adw::ActionRow::new();
                    eject.set_title("Safely remove");
                    eject.set_subtitle("Unmount everything on this drive and power it off");
                    let btn = gtk4::Button::from_icon_name("media-eject-symbolic");
                    btn.set_valign(gtk4::Align::Center);
                    let disk_dev = disk["path"].as_str().unwrap_or_default().to_string();
                    let mounted: Vec<String> = partitions(&disk)
                        .iter()
                        .filter(|p| p["mountpoint"].is_string())
                        .filter_map(|p| p["path"].as_str().map(str::to_string))
                        .collect();
                    let page2 = page.clone();
                    btn.connect_clicked(move |b| {
                        b.set_sensitive(false);
                        let (disk_dev, mounted, page3) = (disk_dev.clone(), mounted.clone(), page2.clone());
                        in_background(
                            move || {
                                mounted.iter().all(|p| run_ok("udisksctl", &["unmount", "-b", p]))
                                    && run_ok("udisksctl", &["power-off", "-b", &disk_dev])
                            },
                            move |ok| {
                                if !ok {
                                    error_dialog(
                                        &page3,
                                        "Couldn't remove the drive",
                                        "Something is still using it. Close any files or windows from it and try again.",
                                    );
                                }
                                page3.activate_action("storage.refresh", None).ok();
                            },
                        );
                    });
                    eject.add_suffix(&btn);
                    exp.add_row(&eject);
                }
                g.add(&exp);
                rows.borrow_mut().push(exp.upcast());
            }
        })
    };
    fill();

    let actions = gtk4::gio::SimpleActionGroup::new();
    let refresh_action = gtk4::gio::SimpleAction::new("refresh", None);
    {
        let fill = fill.clone();
        refresh_action.connect_activate(move |_, _| fill());
    }
    actions.add_action(&refresh_action);
    page.insert_action_group("storage", Some(&actions));
    refresh.connect_clicked(move |_| fill());
    g
}

// ── Home folder usage ──────────────────────────────────────────────────────

fn usage_group() -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("What's using space");
    g.set_description(Some("Your home folder"));
    let loading = adw::ActionRow::new();
    loading.set_title("Measuring…");
    g.add(&loading);

    let dirs: Vec<(&'static str, &'static str, PathBuf)> = vec![
        ("Documents", "folder-documents-symbolic", home().join("Documents")),
        ("Downloads", "folder-download-symbolic", home().join("Downloads")),
        ("Pictures", "folder-pictures-symbolic", home().join("Pictures")),
        ("Videos", "folder-videos-symbolic", home().join("Videos")),
        ("Music", "folder-music-symbolic", home().join("Music")),
        ("Desktop", "user-desktop-symbolic", home().join("Desktop")),
        ("App data and caches", "application-x-executable-symbolic", cache_dir()),
        ("Trash", "user-trash-symbolic", trash_dir()),
    ];
    let g2 = g.clone();
    in_background(
        move || {
            let mut sized: Vec<(&'static str, &'static str, PathBuf, u64)> =
                dirs.into_iter().map(|(n, i, p)| { let s = du(&p); (n, i, p, s) }).collect();
            sized.sort_by(|a, b| b.3.cmp(&a.3));
            sized
        },
        move |sized| {
            g2.remove(&loading);
            let max = sized.iter().map(|d| d.3).max().unwrap_or(1).max(1);
            for (name, icon, path, size) in sized {
                let row = adw::ActionRow::new();
                row.set_title(name);
                row.set_subtitle(&human(size));
                row.add_prefix(&gtk4::Image::from_icon_name(icon));
                let bar = gtk4::LevelBar::for_interval(0.0, 1.0);
                bar.set_value(size as f64 / max as f64);
                bar.set_size_request(140, -1);
                bar.set_valign(gtk4::Align::Center);
                row.add_suffix(&bar);
                if path.exists() && name != "Trash" {
                    row.set_activatable(true);
                    row.add_suffix(&gtk4::Image::from_icon_name("go-next-symbolic"));
                    row.connect_activated(move |_| {
                        let _ = Command::new("xdg-open").arg(&path).spawn();
                    });
                }
                g2.add(&row);
            }
        },
    );
    g
}

// ── Cleanup ────────────────────────────────────────────────────────────────

fn cleanup_row(
    title: &str,
    icon: &str,
    measure: fn() -> u64,
    confirm_title: &'static str,
    confirm_body: &'static str,
    clean: fn() -> bool,
) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(title);
    row.set_subtitle("Measuring…");
    row.add_prefix(&gtk4::Image::from_icon_name(icon));
    let btn = gtk4::Button::with_label("Clean up");
    btn.set_valign(gtk4::Align::Center);
    btn.set_sensitive(false);
    row.add_suffix(&btn);

    let update = {
        let (row, btn) = (row.clone(), btn.clone());
        move || {
            let (row, btn) = (row.clone(), btn.clone());
            in_background(measure, move |size| {
                row.set_subtitle(&human(size));
                btn.set_sensitive(size > 0);
            });
        }
    };
    update();
    btn.connect_clicked(move |b| {
        let update = update.clone();
        let b2 = b.clone();
        confirm(
            b,
            confirm_title,
            confirm_body,
            "Clean up",
            move || {
                clean();
            },
            move || {
                b2.set_sensitive(false);
                update();
            },
        );
    });
    row
}

fn pkg_cache_size() -> u64 {
    du(Path::new("/var/cache/pacman/pkg"))
}

fn clean_pkg_cache() -> bool {
    // Keep one previous version of each package so restore/rollback still works.
    // One prompt: trim installed packages to one old version, drop cached uninstalled ones.
    run_ok("pkexec", &["sh", "-c", "paccache -rk1 && paccache -ruk0"])
}

fn trash_size() -> u64 {
    du(&trash_dir().join("files"))
}

fn empty_trash() -> bool {
    if run_ok("kioclient6", &["emptytrash"]) {
        return true;
    }
    let t = trash_dir();
    for sub in ["files", "info", "expunged"] {
        let _ = std::fs::remove_dir_all(t.join(sub));
        let _ = std::fs::create_dir_all(t.join(sub));
    }
    true
}

fn cache_size() -> u64 {
    du(&cache_dir())
}

fn clear_cache() -> bool {
    // Only entries untouched for a day, so running apps don't lose files they're using.
    run_ok("find", &[&cache_dir().to_string_lossy(), "-mindepth", "1", "-type", "f", "-atime", "+1", "-delete"])
}

fn cleanup_group() -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("Clean up");
    g.add(&cleanup_row(
        "Trash",
        "user-trash-symbolic",
        trash_size,
        "Empty the trash?",
        "Everything in the trash will be deleted permanently.",
        empty_trash,
    ));
    g.add(&cleanup_row(
        "App caches",
        "application-x-executable-symbolic",
        cache_size,
        "Clear app caches?",
        "Cached files that haven't been used in the last day will be removed. Apps rebuild them as needed.",
        clear_cache,
    ));
    g.add(&cleanup_row(
        "Old package versions",
        "system-software-install-symbolic",
        pkg_cache_size,
        "Remove old package versions?",
        "Keeps the most recent previous version of each package, so you can still roll back an update.",
        clean_pkg_cache,
    ));
    g
}

// ── Storage Sense ──────────────────────────────────────────────────────────

const SENSE_DAYS: [u32; 4] = [7, 14, 30, 60];

fn sense_trash_group() -> String {
    trash_dir().to_string_lossy().to_string()
}

fn sense_days() -> Option<u32> {
    let text = std::fs::read_to_string(tmpfiles_rule()).ok()?;
    text.split_whitespace().find_map(|w| w.strip_suffix('d')?.parse().ok())
}

fn set_sense(days: Option<u32>) {
    let group = sense_trash_group();
    match days {
        Some(d) => {
            kconfig::write("ktrashrc", &[&group], "UseTimeLimit", "true");
            kconfig::write("ktrashrc", &[&group], "Days", &d.to_string());
            let rule = tmpfiles_rule();
            if let Some(dir) = rule.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            // systemd-tmpfiles-clean (user timer) ages out cache files older than `d` days.
            let _ = std::fs::write(&rule, format!("# Managed by Zohara Settings (Storage Sense)\ne %h/.cache - - - {d}d\n"));
        }
        None => {
            kconfig::write("ktrashrc", &[&group], "UseTimeLimit", "false");
            let _ = std::fs::remove_file(tmpfiles_rule());
        }
    }
}

fn sense_group() -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("Storage Sense");

    let enabled = sense_days();
    let sw = adw::SwitchRow::new();
    sw.set_title("Clean up automatically");
    sw.set_subtitle("Delete old files from the trash and app caches");
    sw.add_prefix(&gtk4::Image::from_icon_name("edit-clear-all-symbolic"));
    sw.set_active(enabled.is_some());

    let days = adw::ComboRow::new();
    days.set_title("Remove files older than");
    let labels: Vec<String> = SENSE_DAYS.iter().map(|d| format!("{d} days")).collect();
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    days.set_model(Some(&gtk4::StringList::new(&refs)));
    days.set_selected(SENSE_DAYS.iter().position(|d| Some(*d) == enabled).unwrap_or(2) as u32);
    days.set_sensitive(enabled.is_some());

    {
        let days = days.clone();
        sw.connect_active_notify(move |r| {
            days.set_sensitive(r.is_active());
            let d = r.is_active().then(|| SENSE_DAYS[days.selected() as usize]);
            kconfig::spawn(move || set_sense(d));
        });
    }
    days.connect_selected_notify(|r| {
        let d = SENSE_DAYS[r.selected() as usize];
        kconfig::spawn(move || set_sense(Some(d)));
    });
    g.add(&sw);
    g.add(&days);
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
            .label("Storage")
            .halign(gtk4::Align::Start)
            .css_classes(vec!["win11-page-title".to_string()])
            .build(),
    );
    root.append(&drives_group(&root));
    root.append(&usage_group());
    root.append(&cleanup_group());
    root.append(&sense_group());

    scroll.set_child(Some(&root));
    scroll.upcast()
}
