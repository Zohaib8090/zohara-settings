//! Offline maps: choose regions and download them for Organic Maps.
//!
//! The region list and map data version come from the installed Organic
//! Maps app itself, because the app only loads maps whose version matches
//! its own. Map files are fetched from Organic Maps' public servers into the
//! folder the app reads (see libs/platform/platform_linux.cpp upstream):
//! `$XDG_DATA_HOME/Organic Maps/OMaps/<version>/<Region>.mwm` inside the
//! Flatpak's data directory.

use crate::backend::worker::in_background;
use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

const APP_ID: &str = "app.organicmaps.desktop";
const FALLBACK_SERVER: &str = "https://cdn-us1.organicmaps.app/";

#[derive(Clone)]
struct Region {
    id: String,
    size: u64,
    children: Vec<Region>,
}

impl Region {
    fn total(&self) -> u64 {
        if self.children.is_empty() { self.size } else { self.children.iter().map(Region::total).sum() }
    }
    fn leaves(&self) -> Vec<Region> {
        if self.children.is_empty() { vec![self.clone()] } else { self.children.iter().flat_map(Region::leaves).collect() }
    }
    fn label(&self) -> String {
        self.id.replace('_', " ")
    }
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
}

fn installed() -> bool {
    Command::new("flatpak").args(["info", APP_ID]).output().map(|o| o.status.success()).unwrap_or(false)
}

/// The installed app's region index (countries.txt / countries.json).
fn index_file() -> Option<PathBuf> {
    let roots = [
        PathBuf::from("/var/lib/flatpak/app").join(APP_ID),
        home().join(".local/share/flatpak/app").join(APP_ID),
    ];
    for root in roots {
        for name in ["countries.txt", "countries.json"] {
            let p = root.join("current/active/files/share/organicmaps/data").join(name);
            if p.exists() {
                return Some(p);
            }
        }
    }
    None
}

fn parse(v: &Value) -> Region {
    Region {
        id: v["id"].as_str().unwrap_or_default().to_string(),
        size: v["s"].as_u64().unwrap_or(0),
        children: v["g"].as_array().map(|a| a.iter().map(parse).collect()).unwrap_or_default(),
    }
}

/// (data version, top-level regions)
fn load_index() -> Result<(u64, Vec<Region>), String> {
    let path = index_file().ok_or("Couldn't find Organic Maps' list of regions.")?;
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let version = v["v"].as_u64().ok_or("The region list has no version.")?;
    // World and WorldCoasts ship inside the app.
    let mut top: Vec<Region> = v["g"]
        .as_array()
        .map(|a| a.iter().map(parse).filter(|r| r.id != "World" && r.id != "WorldCoasts").collect())
        .unwrap_or_default();
    top.sort_by(|a, b| a.id.cmp(&b.id));
    Ok((version, top))
}

/// Where the app reads maps from; prefer a folder the app has already created.
fn maps_root() -> PathBuf {
    let data = home().join(".var/app").join(APP_ID).join("data");
    if let Ok(rd) = std::fs::read_dir(&data) {
        for e in rd.flatten() {
            let cand = e.path().join("OMaps");
            if cand.is_dir() {
                return cand;
            }
        }
    }
    data.join("Organic Maps").join("OMaps")
}

fn map_path(version: u64, id: &str) -> PathBuf {
    maps_root().join(version.to_string()).join(format!("{id}.mwm"))
}

fn is_downloaded(version: u64, r: &Region) -> bool {
    std::fs::metadata(map_path(version, &r.id)).map(|m| m.len() == r.size).unwrap_or(false)
}

fn server() -> String {
    Command::new("curl")
        .args(["-s", "--max-time", "5", "https://meta.omaps.app/servers"])
        .output()
        .ok()
        .and_then(|o| serde_json::from_slice::<Vec<String>>(&o.stdout).ok())
        .and_then(|v| v.into_iter().next())
        .unwrap_or_else(|| FALLBACK_SERVER.to_string())
}

fn url_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn download(version: u64, r: &Region, server: &str) -> Result<(), String> {
    let dest = map_path(version, &r.id);
    std::fs::create_dir_all(dest.parent().unwrap()).map_err(|e| e.to_string())?;
    let part = dest.with_extension("mwm.part");
    let base = if server.ends_with('/') { server.to_string() } else { format!("{server}/") };
    let url = format!("{base}maps/{version}/{}.mwm", url_encode(&r.id));
    let ok = Command::new("curl")
        .args(["-sfL", "--retry", "3", "-C", "-", "-o"])
        .arg(&part)
        .arg(&url)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let size = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    if !ok || size != r.size {
        return Err(format!("The download of {} didn't complete. Check your internet connection and try again.", r.label()));
    }
    std::fs::rename(&part, &dest).map_err(|e| e.to_string())
}

fn human(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
    } else {
        format!("{:.0} MB", (bytes as f64 / 1_048_576.0).max(1.0))
    }
}

fn message(w: &impl IsA<gtk4::Widget>, title: &str, body: &str) {
    let d = adw::AlertDialog::new(Some(title), Some(body));
    d.add_response("ok", "OK");
    d.present(Some(w));
}

// ── UI ─────────────────────────────────────────────────────────────────────

/// Button that downloads `regions` (with a live percentage) or deletes them once present.
fn action_button(win: &adw::Window, version: u64, regions: Vec<Region>) -> gtk4::Button {
    let btn = gtk4::Button::new();
    btn.set_valign(gtk4::Align::Center);
    let update = {
        let (btn, regions) = (btn.clone(), regions.clone());
        move || {
            let done = regions.iter().all(|r| is_downloaded(version, r));
            if done {
                btn.set_icon_name("user-trash-symbolic");
                btn.set_tooltip_text(Some("Delete downloaded maps"));
                btn.remove_css_class("suggested-action");
                btn.add_css_class("flat");
            } else {
                btn.set_label("Download");
                btn.set_tooltip_text(None);
                btn.remove_css_class("flat");
                btn.add_css_class("suggested-action");
            }
            done
        }
    };
    update();
    let win = win.clone();
    btn.connect_clicked(move |b| {
        let done = regions.iter().all(|r| is_downloaded(version, r));
        if done {
            for r in &regions {
                let _ = std::fs::remove_file(map_path(version, &r.id));
            }
            update();
            return;
        }
        b.set_sensitive(false);
        let todo: Vec<Region> = regions.iter().filter(|r| !is_downloaded(version, r)).cloned().collect();
        let total: u64 = todo.iter().map(|r| r.size).sum::<u64>().max(1);
        // Progress: poll the size of finished files and the current .part file.
        let (b2, todo2) = (b.clone(), todo.clone());
        let progress = glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
            let got: u64 = todo2
                .iter()
                .map(|r| {
                    let p = map_path(version, &r.id);
                    std::fs::metadata(&p)
                        .or_else(|_| std::fs::metadata(p.with_extension("mwm.part")))
                        .map(|m| m.len())
                        .unwrap_or(0)
                })
                .sum();
            b2.set_label(&format!("{}%", (got * 100 / total).min(99)));
            glib::ControlFlow::Continue
        });
        let (b3, win2, update2) = (b.clone(), win.clone(), update.clone());
        let progress = std::cell::Cell::new(Some(progress));
        in_background(
            move || {
                let srv = server();
                todo.iter().try_for_each(|r| download(version, r, &srv))
            },
            move |res| {
                if let Some(p) = progress.take() {
                    p.remove();
                }
                b3.set_sensitive(true);
                update2();
                if let Err(e) = res {
                    message(&win2, "Download incomplete", &e);
                }
            },
        );
    });
    btn
}

fn region_rows(win: &adw::Window, version: u64, regions: &[Region], group: &adw::PreferencesGroup) -> Vec<(gtk4::Widget, String)> {
    let mut out = Vec::new();
    for r in regions {
        let search = r.leaves().iter().map(|l| l.label()).collect::<Vec<_>>().join(" ").to_lowercase();
        if r.children.is_empty() {
            let row = adw::ActionRow::new();
            row.set_title(&glib::markup_escape_text(&r.label()));
            row.set_subtitle(&human(r.size));
            row.add_suffix(&action_button(win, version, vec![r.clone()]));
            group.add(&row);
            out.push((row.upcast(), format!("{} {search}", r.label().to_lowercase())));
        } else {
            let exp = adw::ExpanderRow::new();
            exp.set_title(&glib::markup_escape_text(&r.label()));
            exp.set_subtitle(&format!("{} regions · {} in total", r.leaves().len(), human(r.total())));
            exp.add_suffix(&action_button(win, version, r.leaves()));
            for leaf in r.leaves() {
                let row = adw::ActionRow::new();
                row.set_title(&glib::markup_escape_text(&leaf.label()));
                row.set_subtitle(&human(leaf.size));
                row.add_suffix(&action_button(win, version, vec![leaf.clone()]));
                exp.add_row(&row);
            }
            group.add(&exp);
            out.push((exp.upcast(), format!("{} {search}", r.label().to_lowercase())));
        }
    }
    out
}

fn install_page(win: &adw::Window, reload: std::rc::Rc<dyn Fn()>) -> adw::StatusPage {
    let btn = gtk4::Button::with_label("Install Organic Maps");
    btn.add_css_class("suggested-action");
    btn.add_css_class("pill");
    btn.set_halign(gtk4::Align::Center);
    let page = adw::StatusPage::builder()
        .icon_name("find-location-symbolic")
        .title("Install Organic Maps to use offline maps")
        .description("Organic Maps is a free, private map app that works without internet. It's installed from Flathub.")
        .child(&btn)
        .build();
    let win2 = win.clone();
    btn.connect_clicked(move |b| {
        b.set_sensitive(false);
        b.set_label("Installing…");
        let (b2, win3, reload) = (b.clone(), win2.clone(), reload.clone());
        in_background(
            || {
                Command::new("flatpak")
                    .args(["install", "-y", "--noninteractive", "flathub", APP_ID])
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false)
            },
            move |ok| {
                if ok {
                    reload();
                } else {
                    b2.set_sensitive(true);
                    b2.set_label("Install Organic Maps");
                    message(&win3, "Organic Maps not installed", "Check your internet connection and that Flathub is turned on in Apps > App sources.");
                }
            },
        );
    });
    page
}

pub fn open(parent: &gtk4::Widget) {
    let win = adw::Window::builder().default_width(640).default_height(700).title("Offline maps").build();
    if let Some(p) = parent.root().and_downcast::<gtk4::Window>() {
        win.set_transient_for(Some(&p));
        win.set_modal(true);
    }
    let header = adw::HeaderBar::new();
    let search = gtk4::SearchEntry::builder().placeholder_text("Search countries and regions").hexpand(true).build();
    header.set_title_widget(Some(&search));
    let open_app = gtk4::Button::with_label("Open maps");
    open_app.set_tooltip_text(Some("Open Organic Maps"));
    header.pack_end(&open_app);
    open_app.connect_clicked(|_| {
        let _ = Command::new("flatpak").args(["run", APP_ID]).spawn();
    });

    let stack = gtk4::Stack::new();
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&stack));
    win.set_content(Some(&view));

    let reload_cell: std::rc::Rc<std::cell::RefCell<Option<std::rc::Rc<dyn Fn()>>>> = Default::default();
    let reload: std::rc::Rc<dyn Fn()> = {
        let (win, stack, search, open_app, cell) = (win.clone(), stack.clone(), search.clone(), open_app.clone(), reload_cell.clone());
        std::rc::Rc::new(move || {
            while let Some(c) = stack.first_child() {
                stack.remove(&c);
            }
            let again = cell.borrow().clone().expect("set");
            if !installed() {
                search.set_sensitive(false);
                open_app.set_visible(false);
                stack.add_child(&install_page(&win, again));
                return;
            }
            search.set_sensitive(true);
            open_app.set_visible(true);
            match load_index() {
                Err(e) => {
                    stack.add_child(
                        &adw::StatusPage::builder()
                            .icon_name("dialog-warning-symbolic")
                            .title("Map list unavailable")
                            .description(e.as_str())
                            .build(),
                    );
                }
                Ok((version, regions)) => {
                    let page = adw::PreferencesPage::new();
                    let info = adw::PreferencesGroup::new();
                    info.set_description(Some(
                        "Choose the areas you want. Large countries are split into states or provinces, so you only download what you need.",
                    ));
                    page.add(&info);
                    let group = adw::PreferencesGroup::new();
                    page.add(&group);
                    let rows = region_rows(&win, version, &regions, &group);
                    let rows = std::rc::Rc::new(rows);
                    search.connect_search_changed(move |s| {
                        let q = s.text().to_lowercase();
                        for (w, hay) in rows.iter() {
                            w.set_visible(q.is_empty() || hay.contains(q.as_str()));
                        }
                    });
                    stack.add_child(&page);
                }
            }
        })
    };
    *reload_cell.borrow_mut() = Some(reload.clone());
    reload();
    win.present();
}
