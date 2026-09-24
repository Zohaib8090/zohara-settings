//! Display settings — resolution, refresh rate, scaling, rotation, night light.
//!
//! Talks to whatever is providing the display configuration:
//!   * Wayland: wlr-randr (Sway / Hyprland / etc.) — `--json` gives a stable,
//!     exact schema, so this is the reliable path.
//!   * X11:     xrandr --verbose — no machine-readable output exists, so
//!     this is a best-effort line parser of the human-readable text.
//!
//! On detection failure we just show "no outputs detected" instead of a
//! broken UI (previously this returned one hardcoded fake `eDP-1` entry
//! regardless of what was actually connected).

use gtk4::prelude::*;
use libadwaita as adw;
use adw::prelude::*;
use std::collections::HashSet;
use std::process::Command;

struct DisplayOutput {
    name: String,
    modes: Vec<String>,
    current: String,
    #[allow(dead_code)]
    scale: f64,
    #[allow(dead_code)]
    transform: String,
}

fn command_exists(bin: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {bin}")])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn enumerate_outputs() -> Vec<DisplayOutput> {
    if command_exists("wlr-randr") {
        if let Ok(out) = Command::new("wlr-randr").arg("--json").output() {
            if out.status.success() {
                let outputs = parse_wlr_randr_json(&String::from_utf8_lossy(&out.stdout));
                if !outputs.is_empty() {
                    return outputs;
                }
            }
        }
    }
    if command_exists("xrandr") {
        if let Ok(out) = Command::new("xrandr").arg("--verbose").output() {
            if out.status.success() {
                return parse_xrandr_verbose(&String::from_utf8_lossy(&out.stdout));
            }
        }
    }
    Vec::new()
}

fn parse_wlr_randr_json(raw: &str) -> Vec<DisplayOutput> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) else {
        return Vec::new();
    };
    let Some(arr) = v.as_array() else {
        return Vec::new();
    };
    arr.iter()
        .filter(|o| o.get("enabled").and_then(|e| e.as_bool()).unwrap_or(true))
        .filter_map(|o| {
            let name = o.get("name")?.as_str()?.to_string();
            let modes = o.get("modes")?.as_array()?;

            let mode_str = |m: &serde_json::Value| -> Option<String> {
                let w = m.get("width")?.as_i64()?;
                let h = m.get("height")?.as_i64()?;
                Some(format!("{w}x{h}"))
            };

            let mut seen = HashSet::new();
            let mode_strs: Vec<String> = modes
                .iter()
                .filter_map(mode_str)
                .filter(|m| seen.insert(m.clone()))
                .collect();

            let current = modes
                .iter()
                .find(|m| m.get("current").and_then(|c| c.as_bool()).unwrap_or(false))
                .and_then(mode_str)
                .or_else(|| mode_strs.first().cloned())
                .unwrap_or_default();

            let scale = o.get("scale").and_then(|s| s.as_f64()).unwrap_or(1.0);
            let transform = o
                .get("transform")
                .and_then(|t| t.as_str())
                .unwrap_or("normal")
                .to_string();

            Some(DisplayOutput {
                name,
                modes: mode_strs,
                current,
                scale,
                transform,
            })
        })
        .collect()
}

/// Best-effort parse of `xrandr --verbose`: a header line per output
/// (`eDP-1 connected primary 1920x1080+0+0 ...`) followed by indented mode
/// lines (`   1920x1080     60.00*+  59.94`), `*` marking the active mode.
fn parse_xrandr_verbose(raw: &str) -> Vec<DisplayOutput> {
    let mut outputs = Vec::new();
    let mut cur: Option<DisplayOutput> = None;

    for line in raw.lines() {
        let indented = line.starts_with(' ') || line.starts_with('\t');
        if !indented {
            if let Some(o) = cur.take() {
                outputs.push(o);
            }
            if line.contains(" connected") {
                if let Some(name) = line.split_whitespace().next() {
                    cur = Some(DisplayOutput {
                        name: name.to_string(),
                        modes: Vec::new(),
                        current: String::new(),
                        scale: 1.0,
                        transform: "normal".into(),
                    });
                }
            }
            continue;
        }
        if let Some(o) = cur.as_mut() {
            let trimmed = line.trim_start();
            if let Some(res) = trimmed.split_whitespace().next() {
                let looks_like_mode = res.contains('x')
                    && res.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false);
                if looks_like_mode {
                    if !o.modes.contains(&res.to_string()) {
                        o.modes.push(res.to_string());
                    }
                    if trimmed.contains('*') && o.current.is_empty() {
                        o.current = res.to_string();
                    }
                }
            }
        }
    }
    if let Some(o) = cur.take() {
        outputs.push(o);
    }
    for o in outputs.iter_mut() {
        if o.current.is_empty() {
            o.current = o.modes.first().cloned().unwrap_or_default();
        }
    }
    outputs
}

pub fn build() -> gtk4::Widget {
    let scroll = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .build();

    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
    root.set_margin_start(28);
    root.set_margin_end(28);
    root.set_margin_top(20);
    root.set_margin_bottom(32);

    let title = gtk4::Label::builder()
        .label("Display")
        .halign(gtk4::Align::Start)
        .css_classes(vec!["win11-page-title".to_string()])
        .build();
    root.append(&title);

    if let Some(layout) = super::display_layout::build_section() {
        root.append(&layout);
    }

    let rows = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    rows.set_css_classes(&["win11-card-group"]);

    let outputs = enumerate_outputs();
    // Primary output for the single global Scale/Rotation controls below —
    // real displays vary in name (eDP-1, DP-1, HDMI-A-1, ...), so this must
    // come from what was actually detected, not a hardcoded "eDP-1".
    let primary_name = outputs.first().map(|o| o.name.clone());

    // 1. Resolution + refresh rate (per output)
    let res_exp = adw::ExpanderRow::new();
    res_exp.set_title("Resolution & refresh rate");
    if outputs.is_empty() {
        res_exp.set_subtitle("No displays detected");
    } else {
        res_exp.set_subtitle("Choose a resolution for each connected display");
    }
    res_exp.add_prefix(&gtk4::Image::from_icon_name("video-display-symbolic"));

    for out in &outputs {
        let row = adw::ComboRow::new();
        row.set_title(&out.name);
        row.set_subtitle(&format!("Current: {}", out.current));
        let model = gtk4::StringList::new(
            &out.modes.iter().map(|m| m.as_str()).collect::<Vec<_>>(),
        );
        row.set_model(Some(&model));
        if let Some(idx) = out.modes.iter().position(|m| m == &out.current) {
            row.set_selected(idx as u32);
        }
        let out_name = out.name.clone();
        row.connect_selected_notify(move |r| {
            if let Some(mode) = r.model().and_then(|m| m.downcast::<gtk4::StringList>().ok()) {
                let sel = mode.string(r.selected()).map(|s| s.to_string()).unwrap_or_default();
                if !sel.is_empty() {
                    let _ = Command::new("sh")
                        .args(["-c", &format!(
                            "command -v wlr-randr >/dev/null && wlr-randr --output {} --mode {} || xrandr --output {} --mode {}",
                            out_name, sel, out_name, sel,
                        )])
                        .spawn();
                }
            }
        });
        res_exp.add_row(&row);
    }
    rows.append(&res_exp);

    // 2. Scale
    let scale_exp = adw::ExpanderRow::new();
    scale_exp.set_title("Scale");
    scale_exp.set_subtitle("Make text and UI larger or smaller");
    scale_exp.add_prefix(&gtk4::Image::from_icon_name("zoom-symbolic"));

    let scale_row = adw::ComboRow::new();
    scale_row.set_title("Display scale");
    let scale_list = gtk4::StringList::new(&["100% (Recommended)", "125%", "150%", "175%", "200%"]);
    scale_row.set_model(Some(&scale_list));
    scale_row.set_selected(0);
    if let Some(name) = primary_name.clone() {
        scale_row.connect_selected_notify(move |r| {
            let factor = match r.selected() {
                1 => 1.25,
                2 => 1.5,
                3 => 1.75,
                4 => 2.0,
                _ => 1.0,
            };
            let _ = Command::new("sh")
                .args(["-c", &format!(
                    "command -v wlr-randr >/dev/null && wlr-randr --output {} --scale {} || xrandr --output {} --scale {}",
                    name, factor, name, factor,
                )])
                .spawn();
        });
    } else {
        scale_row.set_sensitive(false);
    }
    scale_exp.add_row(&scale_row);
    rows.append(&scale_exp);

    // 3. Rotation — previously had no click handler at all, so choosing an
    // orientation here did nothing.
    let rot_row = adw::ComboRow::new();
    rot_row.set_title("Display orientation");
    let rot_list = gtk4::StringList::new(&["Landscape", "Portrait", "Landscape (flipped)", "Portrait (flipped)"]);
    rot_row.set_model(Some(&rot_list));
    rot_row.set_selected(0);
    if let Some(name) = primary_name.clone() {
        rot_row.connect_selected_notify(move |r| {
            let (wlr_transform, xrandr_rotate) = match r.selected() {
                1 => ("90", "left"),
                2 => ("180", "inverted"),
                3 => ("270", "right"),
                _ => ("normal", "normal"),
            };
            let _ = Command::new("sh")
                .args(["-c", &format!(
                    "command -v wlr-randr >/dev/null && wlr-randr --output {} --transform {} || xrandr --output {} --rotate {}",
                    name, wlr_transform, name, xrandr_rotate,
                )])
                .spawn();
        });
    } else {
        rot_row.set_sensitive(false);
    }
    let rot_exp = adw::ExpanderRow::new();
    rot_exp.set_title("Rotation");
    rot_exp.set_subtitle("Rotate the screen");
    rot_exp.add_prefix(&gtk4::Image::from_icon_name("object-rotate-right-symbolic"));
    rot_exp.add_row(&rot_row);
    rows.append(&rot_exp);

    // 4. Night light toggle
    let night = adw::SwitchRow::new();
    night.set_title("Night light");
    night.set_subtitle("Reduce blue light at night (uses gammastep if installed)");
    night.set_active(false);
    night.connect_active_notify(move |row| {
        if row.is_active() {
            let _ = Command::new("gammastep").arg("-O").arg("4500K").spawn();
        } else {
            let _ = Command::new("gammastep").arg("-x").spawn();
        }
    });
    rows.append(&night);

    root.append(&rows);
    scroll.set_child(Some(&root));
    scroll.upcast()
}
