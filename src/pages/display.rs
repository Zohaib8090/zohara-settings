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

#[derive(Default)]
struct DisplayOutput {
    name: String,
    modes: Vec<Mode>,
    current: usize,
    scale: f64,
    /// 0 = normal, 1 = 90° (portrait), 2 = 180°, 3 = 270°
    rotation: u32,
    /// libkscreen's own number for the screen; a mirrored screen points at its source by this number.
    id: i64,
    /// A screen that is plugged in but switched off still shows, so it can be switched back on.
    enabled: bool,
    /// The main display: the one that gets the panel and where new windows open.
    primary: bool,
    /// Name of the screen this one copies, when it is mirroring.
    mirror_of: Option<String>,
    /// The monitor's own name from its EDID ("DELL U2723QE"), when it can be read.
    model: Option<String>,
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
    let enabled_of = |o: &serde_json::Value| o["enabled"].as_bool().unwrap_or(false);
    let mut outs: Vec<(i64, DisplayOutput)> = arr
        .iter()
        .filter(|o| o["connected"].as_bool().unwrap_or(enabled_of(o)))
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
            let source = o["replicationSource"].as_i64().unwrap_or(0);
            Some((
                source,
                DisplayOutput {
                    model: read_edid_name(&name),
                    name,
                    modes,
                    current,
                    scale: o["scale"].as_f64().unwrap_or(1.0),
                    rotation,
                    id: o["id"].as_i64().unwrap_or(0),
                    enabled: enabled_of(o),
                    primary: enabled_of(o) && o["priority"].as_i64() == Some(1),
                    mirror_of: None,
                },
            ))
        })
        .collect();
    let names: Vec<(i64, String)> = outs.iter().map(|(_, o)| (o.id, o.name.clone())).collect();
    for (source, o) in outs.iter_mut() {
        if *source != 0 && o.enabled {
            o.mirror_of = names.iter().find(|(id, _)| id == source).map(|(_, n)| n.clone());
        }
    }
    outs.into_iter().map(|(_, o)| o).collect()
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
            Some(DisplayOutput { name, modes, current, scale: o["scale"].as_f64().unwrap_or(1.0), rotation, enabled: true, ..Default::default() })
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
                    enabled: true,
                    ..Default::default()
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

// ── Names, and the commands for main display, on/off and mirroring ──────────────────────────────────────────────

/// The laptop's own panel is `eDP-1`, `LVDS-1` or `DSI-1`; nothing else is built in.
fn is_builtin(connector: &str) -> bool {
    ["eDP", "LVDS", "DSI"].iter().any(|p| connector.starts_with(p))
}

/// What the screen is called in the list: "Built-in display", the monitor's own name, or the connector as a last resort.
fn friendly_title(connector: &str, model: Option<&str>) -> String {
    if is_builtin(connector) {
        "Built-in display".to_string()
    } else {
        model.map(str::to_string).unwrap_or_else(|| connector.to_string())
    }
}

/// The monitor's name from its EDID: one of the four 18-byte blocks after the header holds it (tag 0xFC), ended by a
/// newline and padded with spaces.
fn edid_monitor_name(edid: &[u8]) -> Option<String> {
    if edid.len() < 128 {
        return None;
    }
    (0..4).find_map(|i| {
        let b = &edid[54 + i * 18..54 + (i + 1) * 18];
        if b[0..3] != [0, 0, 0] || b[3] != 0xFC {
            return None;
        }
        let text: String = b[5..18].iter().take_while(|c| **c != 0x0A).map(|c| *c as char).collect();
        let text = text.trim().to_string();
        (!text.is_empty() && text.chars().all(|c| !c.is_control())).then_some(text)
    })
}

/// Reads the name of the monitor on `connector` (the kernel keeps each screen's EDID under `/sys/class/drm`).
fn read_edid_name(connector: &str) -> Option<String> {
    std::fs::read_dir("/sys/class/drm").ok()?.flatten().find_map(|e| {
        let dir = e.file_name().to_string_lossy().to_string();
        let (_, rest) = dir.split_once('-')?;
        (rest == connector).then(|| edid_monitor_name(&std::fs::read(e.path().join("edid")).ok()?)).flatten()
    })
}

fn primary_args(name: &str) -> Vec<String> {
    vec![format!("output.{name}.primary")]
}

fn enable_args(name: &str, on: bool) -> Vec<String> {
    vec![format!("output.{name}.{}", if on { "enable" } else { "disable" })]
}

/// Everything but `source` copies `source`, or goes back to being its own screen. One command, so it happens together.
fn mirror_args(names: &[String], source: &str, mirror: bool) -> Vec<String> {
    names
        .iter()
        .filter(|n| n.as_str() != source)
        .map(|n| format!("output.{n}.mirror.{}", if mirror { source } else { "none" }))
        .collect()
}

/// Runs a display command off the UI thread and tells the row how it went: nothing on success, the tool's own last
/// line under the title when it fails (it used to be thrown away, so a change that did not apply looked like a
/// dropdown that simply did nothing).
fn run_checked(bin: &'static str, args: Vec<String>, row: &impl IsA<adw::ActionRow>) {
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
    let row: adw::ActionRow = row.clone().upcast();
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

/// What one screen's card needs to know about the others.
#[derive(Clone)]
struct Others {
    /// More than one screen is plugged in, so on/off and main display make sense.
    several: bool,
    /// How many are switched on (the last one cannot be switched off).
    enabled: usize,
    /// The current main display, to go back to when "Main display" is reverted.
    primary: Option<String>,
    /// Redraws the page after a change that adds, removes or reorders screens.
    refresh: Rc<dyn Fn()>,
}

fn build_output_group(backend: Backend, out: &DisplayOutput, others: &Others) -> gtk4::Box {
    let group = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    group.set_css_classes(&["win11-card-group"]);

    let header = adw::ActionRow::new();
    header.set_use_markup(false);
    let title = friendly_title(&out.name, out.model.as_deref());
    // The connector name is small print under a friendlier title; when it is the title already, say it once.
    if title != out.name {
        header.set_subtitle(&out.name);
    }
    header.set_title(&title);
    header.add_prefix(&gtk4::Image::from_icon_name("video-display-symbolic"));
    header.set_activatable(false);
    group.append(&header);

    let name = out.name.clone();
    let kscreen = backend == Backend::Kscreen;

    if kscreen && others.several {
        let on = adw::SwitchRow::new();
        on.set_title("Use this display");
        on.set_active(out.enabled);
        // The last screen on cannot be switched off: nothing would be left to click "Keep".
        on.set_sensitive(!out.enabled || others.enabled > 1);
        let n = name.clone();
        let refresh = others.refresh.clone();
        guard_toggle(&on, move |active, r| run_checked("kscreen-doctor", enable_args(&n, active), r), move || refresh());
        group.append(&on);
    }
    if !out.enabled {
        return group;
    }

    if kscreen && others.several && others.enabled > 1 {
        let main = adw::SwitchRow::new();
        main.set_title("Main display");
        main.set_subtitle("Gets the panel and the first windows");
        main.set_active(out.primary);
        // You pick another screen to move the title; this one stays on until you do.
        main.set_sensitive(!out.primary);
        let n = name.clone();
        let back = others.primary.clone();
        let refresh = others.refresh.clone();
        guard_toggle(
            &main,
            move |active, r| {
                let target = if active { Some(n.clone()) } else { back.clone() };
                if let Some(t) = target {
                    run_checked("kscreen-doctor", primary_args(&t), r);
                }
            },
            move || refresh(),
        );
        group.append(&main);
    }

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

/// Asks "Keep these display settings?" with a countdown. "Keep changes" calls `keep`; "Revert", closing the window or the
/// countdown running out calls `revert`. Exactly one of the two runs, once.
fn confirm_keep(parent: &impl IsA<gtk4::Widget>, keep: impl Fn() + 'static, revert: impl Fn() + 'static) {
    let secs = Rc::new(Cell::new(CONFIRM_SECONDS));
    let finished = Rc::new(Cell::new(false));
    let revert = Rc::new(revert);
    let d = adw::AlertDialog::new(Some("Keep these display settings?"), Some(&countdown_text(CONFIRM_SECONDS)));
    d.add_responses(&[("revert", "Revert"), ("keep", "Keep changes")]);
    d.set_response_appearance("keep", adw::ResponseAppearance::Suggested);
    d.set_default_response(Some("revert"));
    d.set_close_response("revert");
    {
        let (finished, revert) = (finished.clone(), revert.clone());
        d.connect_response(None, move |_, response| {
            if finished.replace(true) {
                return;
            }
            if response == "keep" {
                keep();
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
    d.present(Some(parent));
}

/// Makes a display picker ask for confirmation. Choosing a new value applies it, then asks "Keep these display
/// settings?"; reverting puts the old value back (and applies it), keeping makes the new value the one to go back to
/// next time. `apply(index, row)` applies the value at `index`; `done()` runs once the question has been answered.
fn guard_change_then(row: &adw::ComboRow, apply: impl Fn(u32, &adw::ComboRow) + 'static, done: impl Fn() + 'static) {
    let prev = Rc::new(Cell::new(row.selected()));
    let reverting = Rc::new(Cell::new(false));
    let apply = Rc::new(apply);
    let done = Rc::new(done);
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
        let (prev, done_keep) = (prev.clone(), done.clone());
        let (row, apply, reverting, done) = (r.clone(), apply.clone(), reverting.clone(), done.clone());
        confirm_keep(
            r,
            move || {
                prev.set(new);
                done_keep();
            },
            move || {
                reverting.set(true);
                row.set_selected(old);
                reverting.set(false);
                apply(old, &row);
                done();
            },
        );
    });
}

fn guard_change(row: &adw::ComboRow, apply: impl Fn(u32, &adw::ComboRow) + 'static) {
    guard_change_then(row, apply, || {});
}

/// The same question for an on/off switch. `apply(on, row)` applies the new state; reverting flips the switch back and
/// applies the old one. `done()` runs once the question has been answered, so the page can show the new state.
fn guard_toggle(row: &adw::SwitchRow, apply: impl Fn(bool, &adw::SwitchRow) + 'static, done: impl Fn() + 'static) {
    let reverting = Rc::new(Cell::new(false));
    let (apply, done) = (Rc::new(apply), Rc::new(done));
    row.connect_active_notify(move |r| {
        if reverting.get() {
            return;
        }
        let new = r.is_active();
        apply(new, r);
        let (row, apply, reverting, done_keep, done) = (r.clone(), apply.clone(), reverting.clone(), done.clone(), done.clone());
        confirm_keep(
            r,
            move || done_keep(),
            move || {
                reverting.set(true);
                row.set_active(!new);
                reverting.set(false);
                apply(!new, &row);
                done();
            },
        );
    });
}

/// "Extend or mirror": one picker for how two or more screens work together. Mirror copies the main display onto the
/// others; extend makes them one big desktop.
fn build_multi_group(outputs: &[DisplayOutput], refresh: Rc<dyn Fn()>) -> Option<gtk4::Box> {
    let on: Vec<&DisplayOutput> = outputs.iter().filter(|o| o.enabled).collect();
    if on.len() < 2 {
        return None;
    }
    let source = on.iter().find(|o| o.primary).unwrap_or(&on[0]).name.clone();
    let names: Vec<String> = on.iter().map(|o| o.name.clone()).collect();
    let mirroring = on.iter().any(|o| o.mirror_of.is_some());

    let group = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    group.set_css_classes(&["win11-card-group"]);
    let row = adw::ComboRow::new();
    row.set_use_markup(false);
    row.set_title("Multiple displays");
    row.set_model(Some(&gtk4::StringList::new(&["Extend (one desktop)", "Mirror (same picture)"])));
    row.set_selected(mirroring as u32);
    guard_change_then(
        &row,
        move |i, r| run_checked("kscreen-doctor", mirror_args(&names, &source, i == 1), r),
        move || refresh(),
    );
    group.append(&row);
    Some(group)
}

fn kreadconfig(file: &str, group: &str, key: &str) -> Option<String> {
    let out = Command::new("kreadconfig6")
        .args(["--file", file, "--group", group, "--key", key])
        .output()
        .ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Night light: the switch, when it warms, and how warm. The line under the switch is what KWin says it is doing right now, so the
/// switch is never a silent file edit again (see `backend/night_light.rs`).
fn build_night_light() -> gtk4::Widget {
    use crate::backend::night_light as nl;
    use crate::backend::worker::in_background;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    let group = adw::PreferencesGroup::new();
    group.set_title("Night light");

    let night = adw::SwitchRow::new();
    night.set_title("Night light");
    night.add_prefix(&gtk4::Image::from_icon_name("night-light-symbolic"));
    group.add(&night);

    if !nl::available() {
        night.set_subtitle("Requires Plasma's Night Light");
        night.set_sensitive(false);
        return group.upcast();
    }

    let schedule = adw::ComboRow::new();
    schedule.set_title("When");
    schedule.set_model(Some(&gtk4::StringList::new(&["When it gets dark", "All the time"])));
    group.add(&schedule);

    let warmth = adw::SpinRow::with_range(nl::MIN_TEMPERATURE as f64, nl::MAX_TEMPERATURE as f64, 100.0);
    warmth.set_title("Warmth");
    warmth.set_subtitle("Lower is warmer. 6500 K is the normal screen colour");
    group.add(&warmth);

    let current = nl::read();
    let loading = Rc::new(Cell::new(true));
    night.set_active(current.active);
    schedule.set_selected(if current.schedule == nl::Schedule::Always { 1 } else { 0 });
    warmth.set_value(current.temperature as f64);
    night.set_subtitle(&nl::status_text(&current, None));
    loading.set(false);

    let settings = Rc::new(RefCell::new(current));
    // Saves what the controls show and tells KWin; the status line below follows within a second or two.
    let save: Rc<dyn Fn()> = {
        let (night, schedule, warmth, settings, loading) = (night.clone(), schedule.clone(), warmth.clone(), settings.clone(), loading.clone());
        Rc::new(move || {
            if loading.get() {
                return;
            }
            let s = nl::Settings {
                active: night.is_active(),
                schedule: if schedule.selected() == 1 { nl::Schedule::Always } else { nl::Schedule::DayNight },
                temperature: nl::clamp_temperature(warmth.value() as u32),
            };
            *settings.borrow_mut() = s.clone();
            std::thread::spawn(move || {
                nl::write(&s);
            });
        })
    };
    {
        let save = save.clone();
        night.connect_active_notify(move |_| save());
    }
    {
        let save = save.clone();
        schedule.connect_selected_notify(move |_| save());
    }
    {
        let save = save.clone();
        warmth.connect_value_notify(move |_| save());
    }

    // What KWin is doing, every two seconds while the page exists.
    let (night_weak, settings2) = (night.downgrade(), settings.clone());
    glib::timeout_add_seconds_local(2, move || {
        let Some(row) = night_weak.upgrade() else { return glib::ControlFlow::Break };
        let (row2, s) = (row.clone(), settings2.borrow().clone());
        in_background(nl::status, move |st| row2.set_subtitle(&nl::status_text(&s, st.as_ref())));
        glib::ControlFlow::Continue
    });

    group.upcast()
}

/// (Re)builds the list of screens: arrangement, one card per screen, and the extend/mirror picker.
fn fill_screens(screens: &gtk4::Box) {
    while let Some(child) = screens.first_child() {
        screens.remove(&child);
    }
    let weak = screens.downgrade();
    // Give the display a moment to settle before asking it again.
    let refresh: Rc<dyn Fn()> = Rc::new(move || {
        let weak = weak.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(600), move || {
            if let Some(b) = weak.upgrade() {
                fill_screens(&b);
            }
        });
    });

    if let Some(layout) = super::display_layout::build_section() {
        screens.append(&layout);
    }
    match enumerate() {
        Some((backend, outputs)) => {
            let others = Others {
                several: outputs.len() > 1,
                enabled: outputs.iter().filter(|o| o.enabled).count(),
                primary: outputs.iter().find(|o| o.primary).map(|o| o.name.clone()),
                refresh: refresh.clone(),
            };
            for out in &outputs {
                screens.append(&build_output_group(backend, out, &others));
            }
            if backend == Backend::Kscreen {
                if let Some(g) = build_multi_group(&outputs, refresh) {
                    screens.append(&g);
                }
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
            screens.append(&group);
        }
    }
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

    // The screens part is rebuilt after a change that adds, removes or reorders screens.
    let screens = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
    fill_screens(&screens);
    root.append(&screens);

    root.append(&build_night_light());

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

    #[test]
    fn builtin_screens_get_a_friendly_name() {
        assert_eq!(friendly_title("eDP-1", None), "Built-in display");
        assert_eq!(friendly_title("eDP-1", Some("LQ156")), "Built-in display");
        assert_eq!(friendly_title("DP-2", Some("DELL U2723QE")), "DELL U2723QE");
        assert_eq!(friendly_title("HDMI-A-1", None), "HDMI-A-1");
    }

    fn edid_with_name(name: &str) -> Vec<u8> {
        let mut e = vec![0u8; 128];
        e[54..57].copy_from_slice(&[0, 0, 0]);
        e[57] = 0x10; // not a name block
        let b = 72;
        e[b + 3] = 0xFC;
        let mut text = name.as_bytes().to_vec();
        text.push(0x0A);
        text.resize(13, b' ');
        e[b + 5..b + 18].copy_from_slice(&text);
        e
    }

    #[test]
    fn monitor_name_is_read_from_edid() {
        assert_eq!(edid_monitor_name(&edid_with_name("DELL U2723QE")).as_deref(), Some("DELL U2723QE"));
        assert_eq!(edid_monitor_name(&edid_with_name("LG")).as_deref(), Some("LG"));
        assert_eq!(edid_monitor_name(&[0u8; 128]), None);
        assert_eq!(edid_monitor_name(&[0u8; 20]), None);
    }

    #[test]
    fn commands_match_what_kscreen_doctor_accepts() {
        // Checked against a real two-screen KWin session on 2026-10-18.
        assert_eq!(primary_args("DP-1"), ["output.DP-1.primary"]);
        assert_eq!(enable_args("DP-1", false), ["output.DP-1.disable"]);
        assert_eq!(enable_args("DP-1", true), ["output.DP-1.enable"]);
        let names: Vec<String> = ["eDP-1", "DP-1", "HDMI-A-1"].iter().map(|s| s.to_string()).collect();
        assert_eq!(mirror_args(&names, "eDP-1", true), ["output.DP-1.mirror.eDP-1", "output.HDMI-A-1.mirror.eDP-1"]);
        assert_eq!(mirror_args(&names, "eDP-1", false), ["output.DP-1.mirror.none", "output.HDMI-A-1.mirror.none"]);
    }

    #[test]
    fn kscreen_json_gives_state_of_every_connected_screen() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"outputs":[
              {"id":1,"name":"eDP-1","enabled":true,"connected":true,"priority":1,"replicationSource":0,"currentModeId":"1","modes":[]},
              {"id":2,"name":"DP-9","enabled":true,"connected":true,"priority":2,"replicationSource":1,"currentModeId":"1","modes":[]},
              {"id":3,"name":"HDMI-A-9","enabled":false,"connected":true,"priority":0,"replicationSource":0,"modes":[]},
              {"id":4,"name":"DP-8","enabled":false,"connected":false,"priority":0,"replicationSource":0,"modes":[]}
            ]}"#,
        )
        .unwrap();
        let o = parse_kscreen(&v);
        assert_eq!(o.len(), 3, "a screen that is not plugged in is left out");
        assert!(o[0].primary && !o[1].primary && !o[2].primary);
        assert_eq!(o[1].mirror_of.as_deref(), Some("eDP-1"));
        assert!(o[0].mirror_of.is_none());
        assert!(!o[2].enabled, "a switched-off screen still shows so it can be switched on");
    }
}
