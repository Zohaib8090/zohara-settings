//! The review window behind "Reset to the Zohara default look": what the default is, which extra taskbars (panels) a
//! theme left behind, and which themes the person installed (each can be uninstalled). Nothing changes until Reset is
//! pressed; uninstalling a theme is a separate, confirmed step.

use crate::backend::worker::in_background;
use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use std::path::{Component, Path, PathBuf};

// ── Panels (taskbars) ──────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct PanelInfo {
    pub id: u32,
    pub location: String,
    pub widgets: Vec<String>,
}

/// Output of `list_panels_script`: `id,location,type|type;id,location,type|type`.
pub fn parse_panels(out: &str) -> Vec<PanelInfo> {
    out.split(';')
        .filter_map(|chunk| {
            let mut it = chunk.trim().splitn(3, ',');
            let id = it.next()?.trim().parse().ok()?;
            let location = it.next()?.trim().to_string();
            let widgets = it.next().unwrap_or("").split('|').filter(|w| !w.is_empty()).map(str::to_string).collect();
            Some(PanelInfo { id, location, widgets })
        })
        .collect()
}

fn has(p: &PanelInfo, needles: &[&str]) -> bool {
    p.widgets.iter().any(|w| needles.iter().any(|n| w.starts_with(n)))
}

/// What to do with the panels.
#[derive(Debug, PartialEq)]
pub struct PanelPlan {
    pub keep: Option<u32>,
    pub remove: Vec<u32>,
    /// The panel that is kept has no app icons (task manager): one has to be added back.
    pub needs_tasks: bool,
}

/// Keep the panel that has the start menu (the original Zohara taskbar); remove the rest.
pub fn plan_panels(panels: &[PanelInfo]) -> PanelPlan {
    let keep = panels
        .iter()
        .find(|p| has(p, &["org.kde.plasma.kickoff", "org.kde.plasma.kicker", "org.kde.plasma.kickerdash"]))
        .or_else(|| panels.iter().find(|p| has(p, &["org.kde.plasma.icontasks", "org.kde.plasma.taskmanager"])))
        .or_else(|| panels.first());
    let Some(k) = keep else { return PanelPlan { keep: None, remove: vec![], needs_tasks: false } };
    PanelPlan {
        keep: Some(k.id),
        remove: panels.iter().filter(|p| p.id != k.id).map(|p| p.id).collect(),
        needs_tasks: !has(k, &["org.kde.plasma.icontasks", "org.kde.plasma.taskmanager"]),
    }
}

/// "start menu, app icons, system tray, clock" from the widget types.
pub fn describe(p: &PanelInfo) -> String {
    let mut parts: Vec<&str> = Vec::new();
    let mut add = |names: &[&str], label: &'static str| {
        if has(p, names) && !parts.contains(&label) {
            parts.push(label);
        }
    };
    add(&["org.kde.plasma.kickoff", "org.kde.plasma.kicker"], "start menu");
    add(&["org.kde.plasma.icontasks", "org.kde.plasma.taskmanager"], "app icons");
    add(&["org.kde.plasma.systemtray"], "system tray");
    add(&["org.kde.plasma.digitalclock"], "clock");
    add(&["org.kde.plasma.icon"], "launcher buttons");
    if parts.is_empty() {
        "nothing recognisable".to_string()
    } else {
        parts.join(", ")
    }
}

pub fn list_panels_script() -> &'static str {
    r#"var out = []; panels().forEach(function (p) { var t = []; p.widgets().forEach(function (w) { t.push(w.type); }); out.push(p.id + "," + p.location + "," + t.join("|")); }); print(out.join(";"));"#
}

pub fn remove_panels_script(ids: &[u32]) -> String {
    let list = ids.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
    format!(r#"var gone = [{list}]; panels().forEach(function (p) {{ if (gone.indexOf(p.id) >= 0) p.remove(); }});"#)
}

/// Adds the app-icons widget to panel `id` right after the start menu.
pub fn add_tasks_script(id: u32) -> String {
    format!(
        r#"panels().forEach(function (p) {{
  if (p.id != {id}) return;
  var at = -1, ws = p.widgets();
  for (var i = 0; i < ws.length; i++) {{ if (ws[i].type.indexOf("org.kde.plasma.kickoff") == 0 || ws[i].type.indexOf("org.kde.plasma.kicker") == 0) at = i; }}
  var t = p.addWidget("org.kde.plasma.icontasks");
  if (at >= 0) t.index = at + 1;
}});"#
    )
}

pub fn read_panels() -> Vec<PanelInfo> {
    super::start_menu::plasma_script(list_panels_script()).map(|o| parse_panels(&super::start_menu::reply_text(&o))).unwrap_or_default()
}

/// Removes the extra panels and puts the app icons back on the kept one. Returns what went wrong.
pub fn apply_panel_plan(plan: &PanelPlan) -> Vec<String> {
    let mut problems = Vec::new();
    if !plan.remove.is_empty() && super::start_menu::plasma_script(&remove_panels_script(&plan.remove)).is_none() {
        problems.push("Extra taskbars: Plasma did not answer".to_string());
    }
    if let (true, Some(id)) = (plan.needs_tasks, plan.keep) {
        if super::start_menu::plasma_script(&add_tasks_script(id)).is_none() {
            problems.push("App icons: Plasma did not answer".to_string());
        }
    }
    problems
}

// ── Themes the person installed ────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct UserTheme {
    pub kind: &'static str,
    pub name: String,
    pub path: PathBuf,
}

fn dirs_in(p: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(p).map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect()).unwrap_or_default()
}

fn file_name(p: &Path) -> String {
    p.file_name().unwrap_or_default().to_string_lossy().to_string()
}

/// The display name inside a look-and-feel `metadata.json`, else the folder name.
fn look_name(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("metadata.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.get("KPlugin")?.get("Name")?.as_str().map(str::to_string))
        .unwrap_or_else(|| file_name(dir))
}

/// Everything theme-like in the person's own folders (never the system's).
pub fn installed_by_user(home: &Path) -> Vec<UserTheme> {
    let mut out: Vec<UserTheme> = Vec::new();
    for d in dirs_in(&home.join(".local/share/plasma/look-and-feel")) {
        out.push(UserTheme { kind: "Global theme", name: look_name(&d), path: d });
    }
    for d in dirs_in(&home.join(".local/share/plasma/desktoptheme")) {
        out.push(UserTheme { kind: "Plasma style", name: file_name(&d), path: d });
    }
    for d in dirs_in(&home.join(".local/share/aurorae/themes")) {
        out.push(UserTheme { kind: "Window style", name: file_name(&d), path: d });
    }
    for d in dirs_in(&home.join(".config/Kvantum")) {
        out.push(UserTheme { kind: "Widget style", name: file_name(&d), path: d });
    }
    if let Ok(rd) = std::fs::read_dir(home.join(".local/share/color-schemes")) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().map_or(false, |x| x == "colors") {
                let name = std::fs::read_to_string(&p)
                    .ok()
                    .and_then(|t| t.lines().find_map(|l| l.strip_prefix("Name=").map(str::to_string)))
                    .unwrap_or_else(|| file_name(&p));
                out.push(UserTheme { kind: "Colour scheme", name, path: p });
            }
        }
    }
    for base in [home.join(".local/share/icons"), home.join(".icons")] {
        for d in dirs_in(&base) {
            let text = std::fs::read_to_string(d.join("index.theme")).unwrap_or_default();
            let (name, dirs, _) = super::themes::parse_index_theme(&text);
            out.push(UserTheme { kind: if dirs { "Icons" } else { "Mouse pointer" }, name: name.unwrap_or_else(|| file_name(&d)), path: d });
        }
    }
    out.sort_by(|a, b| (a.kind, a.name.to_lowercase()).cmp(&(b.kind, b.name.to_lowercase())));
    out
}

/// Only things inside the person's own theme folders may be deleted, and never a folder itself.
pub fn is_safe_theme_path(home: &Path, p: &Path) -> bool {
    if p.components().any(|c| matches!(c, Component::ParentDir)) {
        return false;
    }
    let roots = [
        home.join(".local/share/plasma/look-and-feel"),
        home.join(".local/share/plasma/desktoptheme"),
        home.join(".local/share/aurorae/themes"),
        home.join(".local/share/color-schemes"),
        home.join(".local/share/icons"),
        home.join(".icons"),
        home.join(".config/Kvantum"),
    ];
    roots.iter().any(|r| p.starts_with(r) && p != r.as_path())
}

fn uninstall(home: &Path, t: &UserTheme) -> Result<(), String> {
    if !is_safe_theme_path(home, &t.path) {
        return Err("That is not one of your own theme folders".into());
    }
    let r = if t.path.is_dir() { std::fs::remove_dir_all(&t.path) } else { std::fs::remove_file(&t.path) };
    r.map_err(|e| e.to_string())
}

// ── The window ─────────────────────────────────────────────────────────────

/// What the review window decided when the person pressed Reset.
pub struct Choice {
    pub plan: PanelPlan,
}

pub fn open_review(from: &impl IsA<gtk4::Widget>, on_reset: impl Fn(Choice) + 'static) {
    let Some(parent) = from.root().and_downcast::<gtk4::Window>() else { return };
    let win = gtk4::Window::builder().title("Reset the look").transient_for(&parent).modal(true).default_width(600).default_height(720).build();
    let content = super::personalization::dialog_content_box();

    let intro = gtk4::Label::new(Some("Look through this first. Nothing changes until you press Reset."));
    intro.set_halign(gtk4::Align::Start);
    intro.set_wrap(true);
    intro.add_css_class("dim-label");
    content.append(&intro);

    // 1. What you get.
    let def = adw::PreferencesGroup::new();
    def.set_title("This is the default look you will get");
    def.set_description(Some("Exactly what Zohara OS comes with."));
    for (t, s) in [
        ("Theme", "Breeze Dark"),
        ("Colours", "Breeze Dark"),
        ("Icons", "Fluent Dark"),
        ("Widgets and buttons", "Materia Dark (Kvantum)"),
        ("Mouse pointer", "Breeze"),
        ("Taskbar", "One bar at the bottom: start menu, app icons, system tray and clock"),
    ] {
        let r = adw::ActionRow::new();
        r.set_title(t);
        r.set_subtitle(s);
        def.add(&r);
    }
    content.append(&def);

    // 2. Panels.
    let panels_group = adw::PreferencesGroup::new();
    panels_group.set_title("Taskbars found");
    let loading = adw::ActionRow::new();
    loading.set_title("Looking…");
    panels_group.add(&loading);
    content.append(&panels_group);

    // 3. Installed themes.
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
    let themes_group = adw::PreferencesGroup::new();
    themes_group.set_title("Themes you installed");
    themes_group.set_description(Some("After the reset these are switched off (they stay on the disk, so you can pick one again later). Uninstall deletes it for good."));
    let found = installed_by_user(&home);
    if found.is_empty() {
        let r = adw::ActionRow::new();
        r.set_title("None found");
        r.set_subtitle("Nothing was installed in your own folders.");
        themes_group.add(&r);
    }
    let status = gtk4::Label::new(None);
    status.set_halign(gtk4::Align::Start);
    status.set_wrap(true);
    status.add_css_class("dim-label");
    for t in found {
        let r = adw::ActionRow::new();
        r.set_title(&glib::markup_escape_text(&t.name));
        r.set_subtitle(t.kind);
        let b = gtk4::Button::with_label("Uninstall");
        b.set_valign(gtk4::Align::Center);
        let (home2, themes2, row2, status2) = (home.clone(), themes_group.clone(), r.clone(), status.clone());
        b.connect_clicked(move |b| {
            let d = adw::AlertDialog::new(
                Some(&format!("Uninstall {}?", t.name)),
                Some("It is deleted from your folders. If you are using it right now, press Reset afterwards."),
            );
            d.add_responses(&[("cancel", "Cancel"), ("go", "Uninstall")]);
            d.set_response_appearance("go", adw::ResponseAppearance::Destructive);
            d.set_close_response("cancel");
            let (t, home3, themes3, row3, status3) = (t.clone(), home2.clone(), themes2.clone(), row2.clone(), status2.clone());
            d.connect_response(None, move |_, resp| {
                if resp != "go" {
                    return;
                }
                match uninstall(&home3, &t) {
                    Ok(()) => {
                        themes3.remove(&row3);
                        status3.set_text(&format!("Uninstalled {}.", t.name));
                    }
                    Err(e) => status3.set_text(&format!("Couldn't uninstall {}: {e}", t.name)),
                }
            });
            d.present(Some(b));
        });
        r.add_suffix(&b);
        themes_group.add(&r);
    }
    content.append(&themes_group);
    content.append(&status);

    // Buttons.
    let buttons = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    buttons.set_halign(gtk4::Align::End);
    let cancel = gtk4::Button::with_label("Cancel");
    let reset = gtk4::Button::with_label("Reset to default");
    reset.add_css_class("destructive-action");
    reset.set_sensitive(false);
    buttons.append(&cancel);
    buttons.append(&reset);
    content.append(&buttons);
    {
        let w = win.clone();
        cancel.connect_clicked(move |_| w.close());
    }

    // Fill in the panels once Plasma answers.
    let plan: std::rc::Rc<std::cell::RefCell<Option<PanelPlan>>> = Default::default();
    {
        let (group, loading, reset, plan) = (panels_group.clone(), loading.clone(), reset.clone(), plan.clone());
        in_background(read_panels, move |panels| {
            group.remove(&loading);
            if panels.is_empty() {
                let r = adw::ActionRow::new();
                r.set_title("Couldn't read the taskbars");
                r.set_subtitle("Plasma didn't answer. The rest of the reset still works.");
                group.add(&r);
                *plan.borrow_mut() = Some(PanelPlan { keep: None, remove: vec![], needs_tasks: false });
            } else {
                let p = plan_panels(&panels);
                for pi in &panels {
                    let r = adw::ActionRow::new();
                    r.set_title(&format!("Taskbar at the {}", pi.location));
                    r.set_subtitle(&describe(pi));
                    let tag = gtk4::Label::new(Some(if Some(pi.id) == p.keep { "Kept" } else { "Will be removed" }));
                    tag.add_css_class(if Some(pi.id) == p.keep { "success" } else { "error" });
                    r.add_suffix(&tag);
                    group.add(&r);
                }
                if p.remove.is_empty() {
                    group.set_description(Some("Just one taskbar. Good."));
                } else {
                    group.set_description(Some("The taskbar with the start menu is kept. The others were most likely added by a theme and are removed."));
                }
                *plan.borrow_mut() = Some(p);
            }
            reset.set_sensitive(true);
        });
    }
    {
        let w = win.clone();
        reset.connect_clicked(move |_| {
            if let Some(p) = plan.borrow_mut().take() {
                on_reset(Choice { plan: p });
            }
            w.close();
        });
    }

    super::adopt_orphan_rows(content.upcast_ref());
    let scroll = gtk4::ScrolledWindow::builder().hscrollbar_policy(gtk4::PolicyType::Never).build();
    scroll.set_child(Some(&content));
    win.set_child(Some(&scroll));
    win.present();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(id: u32, loc: &str, w: &[&str]) -> PanelInfo {
        PanelInfo { id, location: loc.into(), widgets: w.iter().map(|s| s.to_string()).collect() }
    }

    #[test]
    fn panels_are_parsed() {
        let v = parse_panels("2,bottom,org.kde.plasma.kickoff|org.kde.plasma.systemtray;5,bottom,org.kde.plasma.icontasks");
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].id, 2);
        assert_eq!(v[1].widgets, ["org.kde.plasma.icontasks"]);
        assert!(parse_panels("").is_empty());
        assert!(parse_panels("x,y").is_empty());
    }

    #[test]
    fn the_panel_with_the_start_menu_is_kept() {
        // like the owner's laptop: a lower bar with start menu and tray, and a floating dock with the app icons
        let v = vec![p(5, "bottom", &["org.kde.plasma.icontasks", "org.kde.plasma.icon"]), p(2, "bottom", &["org.kde.plasma.kickoff", "org.kde.plasma.systemtray", "org.kde.plasma.digitalclock"])];
        let plan = plan_panels(&v);
        assert_eq!(plan, PanelPlan { keep: Some(2), remove: vec![5], needs_tasks: true });
    }

    #[test]
    fn a_normal_single_panel_is_left_alone() {
        let v = vec![p(2, "bottom", &["org.kde.plasma.kickoff", "org.kde.plasma.icontasks", "org.kde.plasma.systemtray"])];
        assert_eq!(plan_panels(&v), PanelPlan { keep: Some(2), remove: vec![], needs_tasks: false });
        assert_eq!(plan_panels(&[]), PanelPlan { keep: None, remove: vec![], needs_tasks: false });
    }

    #[test]
    fn panels_are_described_in_words() {
        assert_eq!(describe(&p(1, "bottom", &["org.kde.plasma.kickoff", "org.kde.plasma.systemtray"])), "start menu, system tray");
        assert_eq!(describe(&p(1, "top", &["org.kde.plasma.marginsseparator"])), "nothing recognisable");
        assert!(remove_panels_script(&[5, 7]).contains("[5,7]"));
        assert!(add_tasks_script(2).contains("p.id != 2"));
    }

    #[test]
    fn only_your_own_theme_folders_can_be_deleted() {
        let home = Path::new("/home/me");
        assert!(is_safe_theme_path(home, Path::new("/home/me/.local/share/icons/Tela")));
        assert!(is_safe_theme_path(home, Path::new("/home/me/.local/share/color-schemes/Nice.colors")));
        assert!(!is_safe_theme_path(home, Path::new("/home/me/.local/share/icons"))); // the folder itself
        assert!(!is_safe_theme_path(home, Path::new("/usr/share/icons/Tela")));
        assert!(!is_safe_theme_path(home, Path::new("/home/me/.local/share/icons/../../../Documents")));
        assert!(!is_safe_theme_path(home, Path::new("/home/me/Documents")));
    }

    #[test]
    fn installed_themes_are_found_in_the_users_folders() {
        let root = std::env::temp_dir().join(format!("zs-themes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let w = |rel: &str, text: &str| {
            let f = root.join(rel);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, text).unwrap();
        };
        w(".local/share/plasma/look-and-feel/com.x.fancy/metadata.json", "{\"KPlugin\":{\"Name\":\"Fancy Glass\"}}");
        w(".local/share/icons/Tela/index.theme", "[Icon Theme]\nName=Tela\nDirectories=16\n");
        w(".local/share/icons/Bibata/index.theme", "[Icon Theme]\nName=Bibata\n");
        w(".local/share/color-schemes/Nice.colors", "[General]\nName=Nice Night\n");
        w(".config/Kvantum/Glass/Glass.kvconfig", "x");
        w(".config/Kvantum/kvantum.kvconfig", "[General]\ntheme=Glass\n");
        let got: Vec<(String, String)> = installed_by_user(&root).into_iter().map(|t| (t.kind.to_string(), t.name)).collect();
        for want in [("Global theme", "Fancy Glass"), ("Icons", "Tela"), ("Mouse pointer", "Bibata"), ("Colour scheme", "Nice Night"), ("Widget style", "Glass")] {
            assert!(got.contains(&(want.0.to_string(), want.1.to_string())), "missing {want:?} in {got:?}");
        }
        assert_eq!(got.len(), 5); // the kvantum.kvconfig file is not a theme
        let t = installed_by_user(&root).into_iter().find(|t| t.name == "Tela").unwrap();
        assert!(uninstall(&root, &t).is_ok());
        assert!(!root.join(".local/share/icons/Tela").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
