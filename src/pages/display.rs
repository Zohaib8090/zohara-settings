//! Display settings — resolution, refresh rate, scaling, rotation, night light.
//!
//! Talks to whatever is providing the display configuration:
//!   * Wayland: wlr-randr (Sway / Hyprland / etc.)
//!   * X11:     xrandr
//!
//! On detection failure we just show "no outputs detected" instead of
//! a broken UI.

use gtk4::prelude::*;
use libadwaita as adw;
use adw::prelude::*;
use std::process::Command;

/// Try to enumerate outputs. Returns a Vec of (name, modes, current_mode, pos, scale, transform).
fn enumerate_outputs() -> Vec<DisplayOutput> {
    // First try wlr-randr (wayland), fall back to xrandr (X11).
    if let Ok(out) = Command::new("sh")
        .args(["-c", "command -v wlr-randr >/dev/null && wlr-randr --json || command -v xrandr >/dev/null && xrandr --verbose"])
        .output()
    {
        if out.status.success() {
            let stdout = String::from_utf8_lossy(&out.stdout);
            return parse_outputs(&stdout);
        }
    }
    Vec::new()
}

fn parse_outputs(_raw: &str) -> Vec<DisplayOutput> {
    // TODO: real JSON parse for wlr-randr or xrandr --verbose text.
    // For now return a single dummy display so the UI is testable.
    vec![DisplayOutput {
        name: "eDP-1".into(),
        modes: vec!["1920x1080".into(), "1680x1050".into(), "1280x720".into()],
        current: "1920x1080".into(),
        scale: 1.0,
        transform: "normal".into(),
    }]
}

struct DisplayOutput {
    name: String,
    modes: Vec<String>,
    current: String,
    scale: f64,
    transform: String,
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

    let rows = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    rows.set_css_classes(&["win11-card-group"]);

    // 1. Resolution + refresh rate (per output)
    let res_exp = adw::ExpanderRow::new();
    res_exp.set_title("Resolution & refresh rate");
    res_exp.set_subtitle("Choose a resolution for each connected display");
    res_exp.add_prefix(&gtk4::Image::from_icon_name("video-display-symbolic"));

    for out in enumerate_outputs() {
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
    scale_row.connect_selected_notify(|r| {
        let factor = match r.selected() {
            1 => 1.25,
            2 => 1.5,
            3 => 1.75,
            4 => 2.0,
            _ => 1.0,
        };
        let _ = Command::new("sh")
            .args(["-c", &format!(
                "command -v wlr-randr >/dev/null && wlr-randr --output eDP-1 --scale {} || xrandr --output eDP-1 --scale {}",
                factor, factor,
            )])
            .spawn();
    });
    scale_exp.add_row(&scale_row);
    rows.append(&scale_exp);

    // 3. Rotation
    let rot_row = adw::ComboRow::new();
    rot_row.set_title("Display orientation");
    let rot_list = gtk4::StringList::new(&["Landscape", "Portrait", "Landscape (flipped)", "Portrait (flipped)"]);
    rot_row.set_model(Some(&rot_list));
    rot_row.set_selected(0);
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
    let night_clone = night.clone();
    night.connect_active_notify(move |row| {
        if row.is_active() {
            let _ = Command::new("gammastep").arg("-O").arg("4500K").spawn();
        } else {
            let _ = Command::new("gammastep").arg("-x").spawn();
        }
        let _ = night_clone; // keep alive in closure
    });
    rows.append(&night);

    root.append(&rows);
    scroll.set_child(Some(&root));
    scroll.upcast()
}
