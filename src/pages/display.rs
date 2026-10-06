//! Display settings -- per-display resolution/refresh, scale, rotation,
//! arrangement, and night light.
//!
//! Backends, in order of preference:
//!   * kscreen-doctor (libkscreen): what Plasma itself uses; works on X11 and
//!     Wayland Plasma sessions and is the one that ships on Zohara OS.
//!   * wlr-randr --json: wlroots compositors (Sway, Hyprland, ...).
//!   * xrandr --verbose: plain X11, best-effort text parse.
//!
//! Every control is initialised from the display's real current state before
//! its change handler is connected, so opening the page never re-applies
//! anything.

use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use std::cell::Cell;
use std::collections::HashSet;
use std::process::Command;
use std::rc::Rc;

#[derive(Clone, Copy, PartialEq)]
enum Backend {
    Kscreen,
    Wlr,
    Xrandr,
}

struct Mode {
    /// Value passed back to the backend when this mode is chosen.
    id: String,
    label: String,
}

struct DisplayOutput {
    name: String,
    modes: Vec<Mode>,
    current: usize,
    scale: f64,
    /// 0 = normal, 1 = 90° (portrait), 2 = 180°, 3 = 270°
    rotation: u32,
}

const SCALES: [f64; 7] = [1.0, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0];
const ROTATIONS: [&str; 4] = ["Landscape", "Portrait", "Landscape (flipped)", "Portrait (flipped)"];

fn command_exists(bin: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {bin} >/dev/null")])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn run_json(bin: &str, args: &[&str]) -> Option<serde_json::Value> {
    let out = Command::new(bin).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    serde_json::from_slice(&out.stdout).ok()
}

fn enumerate() -> Option<(Backend, Vec<DisplayOutput>)> {
    if command_exists("kscreen-doctor") {
        if let Some(v) = run_json("kscreen-doctor", &["-j"]) {
            let outs = parse_kscreen(&v);
            if !outs.is_empty() {
                return Some((Backend::Kscreen, outs));
            }
        }
    }
    if command_exists("wlr-randr") {
        if let Some(v) = run_json("wlr-randr", &["--json"]) {
            let outs = parse_wlr(&v);
            if !outs.is_empty() {
                return Some((Backend::Wlr, outs));
            }
        }
    }
    if command_exists("xrandr") {
        if let Ok(out) = Command::new("xrandr").arg("--verbose").output() {
            if out.status.success() {
                let outs = parse_xrandr(&String::from_utf8_lossy(&out.stdout));
                if !outs.is_empty() {
                    return Some((Backend::Xrandr, outs));
                }
            }
        }
    }
    None
}

fn parse_kscreen(v: &serde_json::Value) -> Vec<DisplayOutput> {
    let Some(arr) = v["outputs"].as_array() else {
        return Vec::new();
    };
    arr.iter()
        .filter(|o| o["enabled"].as_bool().unwrap_or(false))
        .filter_map(|o| {
            let name = o["name"].as_str()?.to_string();
            let current_id = o["currentModeId"].as_str().unwrap_or_default();
            let mut modes: Vec<(i64, f64, Mode)> = o["modes"]
                .as_array()?
                .iter()
                .filter_map(|m| {
                    let w = m["size"]["width"].as_i64()?;
                    let h = m["size"]["height"].as_i64()?;
                    let hz = m["refreshRate"].as_f64().unwrap_or(0.0);
                    Some((
                        w * h,
                        hz,
                        Mode {
                            id: m["id"].as_str()?.to_string(),
                            label: format!("{w} × {h}  ·  {hz:.2} Hz"),
                        },
                    ))
                })
                .collect();
            // Largest resolution first, then highest refresh rate.
            modes.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.total_cmp(&a.1)));
            let mut seen = HashSet::new();
            let modes: Vec<Mode> = modes
                .into_iter()
                .map(|(_, _, m)| m)
                .filter(|m| seen.insert(m.label.clone()))
                .collect();
            let current = modes.iter().position(|m| m.id == current_id).unwrap_or(0);
            // libkscreen rotation bitmask: 1 none, 2 left, 4 inverted, 8 right
            let rotation = match o["rotation"].as_i64().unwrap_or(1) {
                2 => 1,
                4 => 2,
                8 => 3,
                _ => 0,
            };
            Some(DisplayOutput {
                name,
                modes,
                current,
                scale: o["scale"].as_f64().unwrap_or(1.0),
                rotation,
            })
        })
        .collect()
}

fn parse_wlr(v: &serde_json::Value) -> Vec<DisplayOutput> {
    let Some(arr) = v.as_array() else {
        return Vec::new();
    };
    arr.iter()
        .filter(|o| o["enabled"].as_bool().unwrap_or(true))
        .filter_map(|o| {
            let name = o["name"].as_str()?.to_string();
            let mut seen = HashSet::new();
            let mut current = 0;
            let mut modes = Vec::new();
            for m in o["modes"].as_array()? {
                let (Some(w), Some(h)) = (m["width"].as_i64(), m["height"].as_i64()) else {
                    continue;
                };
                let hz = m["refresh"].as_f64().unwrap_or(0.0);
                let label = format!("{w} × {h}  ·  {hz:.2} Hz");
                if !seen.insert(label.clone()) {
                    continue;
                }
                if m["current"].as_bool().unwrap_or(false) {
                    current = modes.len();
                }
                modes.push(Mode { id: format!("{w}x{h}@{hz:.3}Hz"), label });
            }
            let rotation = match o["transform"].as_str().unwrap_or("normal") {
                "90" => 1,
                "180" => 2,
                "270" => 3,
                _ => 0,
            };
            Some(DisplayOutput { name, modes, current, scale: o["scale"].as_f64().unwrap_or(1.0), rotation })
        })
        .collect()
}

/// Best-effort parse of `xrandr --verbose`: a header line per output
/// (`eDP-1 connected primary 1920x1080+0+0 (0x4b) left ...`) followed by
/// indented mode lines (`  1920x1080 (0x4b) 148.500MHz *current +preferred`).
fn parse_xrandr(raw: &str) -> Vec<DisplayOutput> {
    let mut outputs: Vec<DisplayOutput> = Vec::new();
    let mut active = false;
    for line in raw.lines() {
        if !line.starts_with(' ') && !line.starts_with('\t') {
            active = false;
            let words: Vec<&str> = line.split_whitespace().collect();
            // "connected" with a geometry token means the output is enabled.
            if words.get(1) == Some(&"connected") && words.iter().any(|w| w.contains('+') && w.contains('x')) {
                let rotation = if words.contains(&"left") {
                    1
                } else if words.contains(&"inverted") {
                    2
                } else if words.contains(&"right") {
                    3
                } else {
                    0
                };
                outputs.push(DisplayOutput {
                    name: words[0].to_string(),
                    modes: Vec::new(),
                    current: 0,
                    scale: 1.0,
                    rotation,
                });
                active = true;
            }
            continue;
        }
        if !active {
            continue;
        }
        let Some(o) = outputs.last_mut() else { continue };
        let trimmed = line.trim_start();
        let Some(res) = trimmed.split_whitespace().next() else { continue };
        if res.contains('x') && res.starts_with(|c: char| c.is_ascii_digit()) {
            if !o.modes.iter().any(|m| m.id == res) {
                if trimmed.contains("*current") {
                    o.current = o.modes.len();
                }
                o.modes.push(Mode { id: res.to_string(), label: res.replace('x', " × ") });
            }
        }
    }
    outputs
}

/// Runs a display command off the UI thread and tells the row how it went: nothing on success, the tool's own last
/// line under the title when it fails (it used to be thrown away, so a change that did not apply looked like a
/// dropdown that simply did nothing).
fn run_checked(bin: &'static str, args: Vec<String>, row: &adw::ComboRow) {
    let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
    std::thread::spawn(move || {
        let r = match Command::new(bin).args(&args).output() {
            Ok(o) if o.status.success() => Ok(()),
            Ok(o) => {
                let text = format!("{}\n{}", String::from_utf8_lossy(&o.stderr), String::from_utf8_lossy(&o.stdout));
                let last = text.lines().map(str::trim).filter(|l| !l.is_empty()).last().unwrap_or("it was refused").to_string();
                Err(last)
            }
            Err(e) => Err(format!("{bin} could not be started ({e})")),
        };
        let _ = tx.send(r);
    });
    let row = row.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(150), move || match rx.try_recv() {
        Ok(Ok(())) => {
            row.set_subtitle("");
            glib::ControlFlow::Break
        }
        Ok(Err(e)) => {
            row.set_subtitle(&glib::markup_escape_text(&format!("Couldn't apply: {e}")));
            glib::ControlFlow::Break
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        Err(_) => glib::ControlFlow::Break,
    });
}

fn apply_mode(b: Backend, out: &str, mode: &str, row: &adw::ComboRow) {
    match b {
        Backend::Kscreen => run_checked("kscreen-doctor", vec![format!("output.{out}.mode.{mode}")], row),
        Backend::Wlr => run_checked("wlr-randr", vec!["--output".into(), out.into(), "--mode".into(), mode.into()], row),
        Backend::Xrandr => run_checked("xrandr", vec!["--output".into(), out.into(), "--mode".into(), mode.into()], row),
    }
}

fn apply_scale(b: Backend, out: &str, scale: f64, row: &adw::ComboRow) {
    match b {
        Backend::Kscreen => run_checked("kscreen-doctor", vec![format!("output.{out}.scale.{scale}")], row),
        Backend::Wlr => run_checked("wlr-randr", vec!["--output".into(), out.into(), "--scale".into(), scale.to_string()], row),
        Backend::Xrandr => {}
    }
}

fn apply_rotation(b: Backend, out: &str, rot: u32, row: &adw::ComboRow) {
    let idx = rot.min(3) as usize;
    match b {
        Backend::Kscreen => {
            let name = ["normal", "left", "inverted", "right"][idx];
            run_checked("kscreen-doctor", vec![format!("output.{out}.rotation.{name}")], row);
        }
        Backend::Wlr => {
            let t = ["normal", "90", "180", "270"][idx];
            run_checked("wlr-randr", vec!["--output".into(), out.into(), "--transform".into(), t.into()], row);
        }
        Backend::Xrandr => {
            let r = ["normal", "left", "inverted", "right"][idx];
            run_checked("xrandr", vec!["--output".into(), out.into(), "--rotate".into(), r.into()], row);
        }
    }
}

fn scale_label(s: f64) -> String {
    format!("{}%", (s * 100.0).round() as i64)
}

fn build_output_group(backend: Backend, out: &DisplayOutput) -> gtk4::Box {
    let group = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    group.set_css_classes(&["win11-card-group"]);

    let header = adw::ActionRow::new();
    header.set_title(&out.name);
    header.add_prefix(&gtk4::Image::from_icon_name("video-display-symbolic"));
    header.set_activatable(false);
    group.append(&header);

    let name = out.name.clone();

    let res = adw::ComboRow::new();
    res.set_use_markup(false);
    res.set_title("Resolution & refresh rate");
    let labels: Vec<&str> = out.modes.iter().map(|m| m.label.as_str()).collect();
    res.set_model(Some(&gtk4::StringList::new(&labels)));
    if out.modes.is_empty() {
        res.set_sensitive(false);
    } else {
        res.set_selected(out.current as u32);
        let ids: Vec<String> = out.modes.iter().map(|m| m.id.clone()).collect();
        let n = name.clone();
        guard_change(&res, move |i, r| {
            if let Some(id) = ids.get(i as usize) {
                apply_mode(backend, &n, id, r);
            }
        });
    }
    group.append(&res);

    let scale = adw::ComboRow::new();
    scale.set_title("Scale");
    let mut scales: Vec<f64> = SCALES.to_vec();
    if !scales.iter().any(|s| (s - out.scale).abs() < 0.01) {
        scales.push(out.scale);
        scales.sort_by(|a, b| a.total_cmp(b));
    }
    let scale_labels: Vec<String> = scales.iter().map(|s| scale_label(*s)).collect();
    let scale_refs: Vec<&str> = scale_labels.iter().map(String::as_str).collect();
    scale.set_model(Some(&gtk4::StringList::new(&scale_refs)));
    let cur = scales.iter().position(|s| (s - out.scale).abs() < 0.01).unwrap_or(0);
    scale.set_selected(cur as u32);
    if backend == Backend::Xrandr {
        // xrandr --scale resamples the framebuffer instead of scaling the UI.
        scale.set_subtitle("Not available in this session");
        scale.set_sensitive(false);
    } else {
        let n = name.clone();
        guard_change(&scale, move |i, r| {
            if let Some(s) = scales.get(i as usize) {
                apply_scale(backend, &n, *s, r);
            }
        });
    }
    group.append(&scale);

    let rot = adw::ComboRow::new();
    rot.set_title("Orientation");
    rot.set_model(Some(&gtk4::StringList::new(&ROTATIONS)));
    rot.set_selected(out.rotation);
    let n = name;
    guard_change(&rot, move |i, r| apply_rotation(backend, &n, i, r));
    group.append(&rot);

    group
}

// ── "Keep these display settings?" ──────────────────────────────────────────────────────────────────────────────

/// How long the person has to say "keep" before the old setting comes back by itself. A resolution the screen cannot
/// show would otherwise leave nothing on screen to click.
const CONFIRM_SECONDS: u32 = 15;

fn countdown_text(secs: u32) -> String {
    format!("If you do nothing, your previous display settings come back in {secs} second{}.", if secs == 1 { "" } else { "s" })
}

/// Makes a display picker ask for confirmation. Choosing a new value applies it, then asks "Keep these display
/// settings?" with a countdown; "Revert", closing the window or the countdown running out puts the old value back (and
/// applies it), "Keep changes" makes the new value the one to go back to next time. `apply(index, row)` applies the
/// value at `index`.
fn guard_change(row: &adw::ComboRow, apply: impl Fn(u32, &adw::ComboRow) + 'static) {
    let prev = Rc::new(Cell::new(row.selected()));
    let reverting = Rc::new(Cell::new(false));
    let apply = Rc::new(apply);
    row.connect_selected_notify(move |r| {
        // Putting the old value back moves the picker, which must not ask again.
        if reverting.get() {
            return;
        }
        let (new, old) = (r.selected(), prev.get());
        if new == old {
            return;
        }
        apply(new, r);

        let secs = Rc::new(Cell::new(CONFIRM_SECONDS));
        let finished = Rc::new(Cell::new(false));
        let d = adw::AlertDialog::new(Some("Keep these display settings?"), Some(&countdown_text(CONFIRM_SECONDS)));
        d.add_responses(&[("revert", "Revert"), ("keep", "Keep changes")]);
        d.set_response_appearance("keep", adw::ResponseAppearance::Suggested);
        d.set_default_response(Some("revert"));
        d.set_close_response("revert");

        let revert = {
            let (row, apply, reverting) = (r.clone(), apply.clone(), reverting.clone());
            Rc::new(move || {
                reverting.set(true);
                row.set_selected(old);
                reverting.set(false);
                apply(old, &row);
            })
        };
        {
            let (finished, prev, revert) = (finished.clone(), prev.clone(), revert.clone());
            d.connect_response(None, move |_, response| {
                if finished.replace(true) {
                    return;
                }
                if response == "keep" {
                    prev.set(new);
                } else {
                    revert();
                }
            });
        }
        {
            let d = d.clone();
            glib::timeout_add_seconds_local(1, move || {
                if finished.get() {
                    return glib::ControlFlow::Break;
                }
                let left = secs.get().saturating_sub(1);
                secs.set(left);
                d.set_body(&countdown_text(left));
                if left == 0 {
                    if !finished.replace(true) {
                        revert();
                        d.force_close();
                    }
                    return glib::ControlFlow::Break;
                }
                glib::ControlFlow::Continue
            });
        }
        d.present(Some(r));
    });
}

fn kreadconfig(file: &str, group: &str, key: &str) -> Option<String> {
    let out = Command::new("kreadconfig6")
        .args(["--file", file, "--group", group, "--key", key])
        .output()
        .ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn build_night_light() -> adw::SwitchRow {
    let night = adw::SwitchRow::new();
    night.set_title("Night light");
    night.add_prefix(&gtk4::Image::from_icon_name("night-light-symbolic"));

    if !command_exists("kwriteconfig6") {
        night.set_subtitle("Requires Plasma's Night Color");
        night.set_sensitive(false);
        return night;
    }

    night.set_subtitle("Warmer colours to reduce blue light at night");
    night.set_active(kreadconfig("kwinrc", "NightColor", "Active").as_deref() == Some("true"));
    night.connect_active_notify(|row| {
        let on = row.is_active();
        std::thread::spawn(move || {
            let _ = Command::new("kwriteconfig6")
                .args(["--file", "kwinrc", "--group", "NightColor", "--key", "Active", if on { "true" } else { "false" }])
                .status();
            // KWin only rereads kwinrc when asked.
            let _ = Command::new("dbus-send")
                .args(["--session", "--type=method_call", "--dest=org.kde.KWin", "/KWin", "org.kde.KWin.reconfigure"])
                .status();
        });
    });
    night
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

    root.append(
        &gtk4::Label::builder()
            .label("Display")
            .halign(gtk4::Align::Start)
            .css_classes(vec!["win11-page-title".to_string()])
            .build(),
    );

    if let Some(layout) = super::display_layout::build_section() {
        root.append(&layout);
    }

    match enumerate() {
        Some((backend, outputs)) => {
            for out in &outputs {
                root.append(&build_output_group(backend, out));
            }
        }
        None => {
            let group = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
            group.set_css_classes(&["win11-card-group"]);
            let row = adw::ActionRow::new();
            row.set_title("No displays detected");
            row.set_subtitle("Display settings need kscreen, wlr-randr or xrandr");
            row.set_activatable(false);
            group.append(&row);
            root.append(&group);
        }
    }

    let extras = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    extras.set_css_classes(&["win11-card-group"]);
    extras.append(&build_night_light());
    root.append(&extras);

    scroll.set_child(Some(&root));
    scroll.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn countdown_wording() {
        assert!(countdown_text(15).contains("in 15 seconds"));
        assert!(countdown_text(1).contains("in 1 second."));
        assert!(!countdown_text(1).contains("seconds"));
        assert!(countdown_text(0).contains("0 seconds"));
    }

    #[test]
    fn there_is_enough_time_to_read_and_click() {
        // Long enough to understand and press a button, short enough that a black screen does not feel broken.
        assert!((10..=30).contains(&CONFIRM_SECONDS));
    }
}
