//! Multi-monitor arrangement: drag monitors to position them relative to each
//! other, then Apply.
//!
//! Uses `kscreen-doctor` (libkscreen), which is what Plasma itself uses and
//! works on both X11 and Wayland sessions. The section is only shown when
//! kscreen-doctor exists and at least two outputs are enabled.

use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use std::cell::RefCell;
use std::process::Command;
use std::rc::Rc;

#[derive(Clone)]
struct Mon {
    name: String,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

struct State {
    mons: Vec<Mon>,
    selected: Option<usize>,
    // layout -> canvas transform, fixed while dragging so rectangles don't jump
    scale: f64,
    ox: f64,
    oy: f64,
    fitted_for: (i32, i32),
    drag_start: Option<(f64, f64)>,
    dirty: bool,
}

const SNAP_PX: f64 = 14.0;

fn load_monitors() -> Vec<Mon> {
    let Ok(out) = Command::new("kscreen-doctor").arg("-j").output() else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else {
        return Vec::new();
    };
    v["outputs"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter(|o| o["enabled"].as_bool().unwrap_or(false))
                .filter_map(|o| {
                    let scale = o["scale"].as_f64().filter(|s| *s > 0.0).unwrap_or(1.0);
                    let mut w = o["size"]["width"].as_f64()? / scale;
                    let mut h = o["size"]["height"].as_f64()? / scale;
                    // rotation bitmask: 2 = left, 8 = right swap the axes
                    if matches!(o["rotation"].as_i64().unwrap_or(1), 2 | 8) {
                        std::mem::swap(&mut w, &mut h);
                    }
                    Some(Mon {
                        name: o["name"].as_str()?.to_string(),
                        x: o["pos"]["x"].as_f64().unwrap_or(0.0),
                        y: o["pos"]["y"].as_f64().unwrap_or(0.0),
                        w,
                        h,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn fit(st: &mut State, width: i32, height: i32) {
    if st.mons.is_empty() {
        return;
    }
    let min_x = st.mons.iter().map(|m| m.x).fold(f64::MAX, f64::min);
    let min_y = st.mons.iter().map(|m| m.y).fold(f64::MAX, f64::min);
    let max_x = st.mons.iter().map(|m| m.x + m.w).fold(f64::MIN, f64::max);
    let max_y = st.mons.iter().map(|m| m.y + m.h).fold(f64::MIN, f64::max);
    let max_w = st.mons.iter().map(|m| m.w).fold(0.0, f64::max);
    let max_h = st.mons.iter().map(|m| m.h).fold(0.0, f64::max);
    // Leave a monitor's worth of free space on each side so there's room to drag.
    let span_w = (max_x - min_x) + 2.0 * max_w;
    let span_h = (max_y - min_y) + 2.0 * max_h;
    st.scale = (width as f64 / span_w).min(height as f64 / span_h);
    let cx = (min_x + max_x) / 2.0;
    let cy = (min_y + max_y) / 2.0;
    st.ox = width as f64 / 2.0 - cx * st.scale;
    st.oy = height as f64 / 2.0 - cy * st.scale;
    st.fitted_for = (width, height);
}

fn overlaps(a: &Mon, b: &Mon) -> bool {
    a.x < b.x + b.w - 0.5 && a.x + a.w > b.x + 0.5 && a.y < b.y + b.h - 0.5 && a.y + a.h > b.y + 0.5
}

fn snap(st: &State, idx: usize, mut x: f64, mut y: f64) -> (f64, f64) {
    let me = &st.mons[idx];
    let tol = SNAP_PX / st.scale;
    let (mut best_dx, mut best_dy) = (tol, tol);
    let (mut sx, mut sy) = (x, y);
    for (i, o) in st.mons.iter().enumerate() {
        if i == idx {
            continue;
        }
        for cand in [o.x + o.w, o.x - me.w, o.x, o.x + o.w - me.w] {
            if (x - cand).abs() < best_dx {
                best_dx = (x - cand).abs();
                sx = cand;
            }
        }
        for cand in [o.y + o.h, o.y - me.h, o.y, o.y + o.h - me.h] {
            if (y - cand).abs() < best_dy {
                best_dy = (y - cand).abs();
                sy = cand;
            }
        }
    }
    x = sx;
    y = sy;
    (x, y)
}

/// Returns None when arrangement isn't applicable (no kscreen-doctor, or fewer than two outputs).
pub fn build_section() -> Option<gtk4::Widget> {
    let mons = load_monitors();
    if mons.len() < 2 {
        return None;
    }

    let accent = gtk4::gdk::RGBA::parse(crate::theme::load().accent.as_str())
        .unwrap_or_else(|_| gtk4::gdk::RGBA::new(0.3, 0.55, 1.0, 1.0));

    let state = Rc::new(RefCell::new(State {
        mons,
        selected: None,
        scale: 1.0,
        ox: 0.0,
        oy: 0.0,
        fitted_for: (0, 0),
        drag_start: None,
        dirty: false,
    }));

    let outer = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    let heading = gtk4::Label::builder()
        .label("Arrange displays")
        .halign(gtk4::Align::Start)
        .css_classes(vec!["heading".to_string()])
        .build();
    let hint = gtk4::Label::builder()
        .label("Drag a display to change where it sits relative to the others, then apply.")
        .halign(gtk4::Align::Start)
        .wrap(true)
        .css_classes(vec!["dim-label".to_string()])
        .build();
    outer.append(&heading);
    outer.append(&hint);

    let area = gtk4::DrawingArea::new();
    area.set_content_height(220);
    area.set_hexpand(true);
    area.set_css_classes(&["win11-card-group"]);

    {
        let state = state.clone();
        area.set_draw_func(move |area, cr, w, h| {
            let mut st = state.borrow_mut();
            if st.fitted_for != (w, h) && st.drag_start.is_none() {
                fit(&mut st, w, h);
            }
            let fg = area.color();
            for (i, m) in st.mons.iter().enumerate() {
                let (x, y) = (st.ox + m.x * st.scale, st.oy + m.y * st.scale);
                let (rw, rh) = (m.w * st.scale, m.h * st.scale);
                let sel = st.selected == Some(i);
                if sel {
                    cr.set_source_rgba(accent.red() as f64, accent.green() as f64, accent.blue() as f64, 0.35);
                } else {
                    cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.10);
                }
                cr.rectangle(x, y, rw, rh);
                let _ = cr.fill_preserve();
                if sel {
                    cr.set_source_rgba(accent.red() as f64, accent.green() as f64, accent.blue() as f64, 1.0);
                } else {
                    cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.5);
                }
                cr.set_line_width(if sel { 2.0 } else { 1.0 });
                let _ = cr.stroke();

                cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.9);
                cr.set_font_size(13.0);
                let label = format!("{}  {}", i + 1, m.name);
                let ext = cr.text_extents(&label).ok();
                let tw = ext.map(|e| e.width()).unwrap_or(0.0);
                cr.move_to(x + (rw - tw) / 2.0, y + rh / 2.0 + 5.0);
                let _ = cr.show_text(&label);
            }
        });
    }

    let apply_btn = gtk4::Button::with_label("Apply arrangement");
    apply_btn.set_css_classes(&["suggested-action"]);
    apply_btn.set_halign(gtk4::Align::End);
    apply_btn.set_sensitive(false);

    let drag = gtk4::GestureDrag::new();
    {
        let state = state.clone();
        let area = area.clone();
        drag.connect_drag_begin(move |_, px, py| {
            let mut st = state.borrow_mut();
            let hit = st.mons.iter().position(|m| {
                let (x, y) = (st.ox + m.x * st.scale, st.oy + m.y * st.scale);
                px >= x && px <= x + m.w * st.scale && py >= y && py <= y + m.h * st.scale
            });
            st.selected = hit;
            st.drag_start = hit.map(|i| (st.mons[i].x, st.mons[i].y));
            area.queue_draw();
        });
    }
    {
        let state = state.clone();
        let area = area.clone();
        drag.connect_drag_update(move |_, dx, dy| {
            let mut st = state.borrow_mut();
            if let (Some(i), Some((sx, sy))) = (st.selected, st.drag_start) {
                let (nx, ny) = snap(&st, i, sx + dx / st.scale, sy + dy / st.scale);
                st.mons[i].x = nx;
                st.mons[i].y = ny;
                area.queue_draw();
            }
        });
    }
    {
        let state = state.clone();
        let area = area.clone();
        let apply_btn = apply_btn.clone();
        drag.connect_drag_end(move |_, _, _| {
            let mut st = state.borrow_mut();
            if let (Some(i), Some((sx, sy))) = (st.selected, st.drag_start.take()) {
                let me = st.mons[i].clone();
                // Overlapping displays are not a valid layout: put it back.
                if st.mons.iter().enumerate().any(|(j, o)| j != i && overlaps(&me, o)) {
                    st.mons[i].x = sx;
                    st.mons[i].y = sy;
                } else if (me.x - sx).abs() > 0.5 || (me.y - sy).abs() > 0.5 {
                    st.dirty = true;
                    apply_btn.set_sensitive(true);
                }
            }
            area.queue_draw();
        });
    }
    area.add_controller(drag);
    outer.append(&area);

    {
        let state = state.clone();
        let area = area.clone();
        apply_btn.connect_clicked(move |btn| {
            let mut st = state.borrow_mut();
            // kscreen wants a layout anchored at the origin.
            let min_x = st.mons.iter().map(|m| m.x).fold(f64::MAX, f64::min);
            let min_y = st.mons.iter().map(|m| m.y).fold(f64::MAX, f64::min);
            for m in st.mons.iter_mut() {
                m.x -= min_x;
                m.y -= min_y;
            }
            let args: Vec<String> = st
                .mons
                .iter()
                .map(|m| format!("output.{}.position.{},{}", m.name, m.x.round() as i64, m.y.round() as i64))
                .collect();
            std::thread::spawn(move || {
                let _ = Command::new("kscreen-doctor").args(&args).status();
            });
            st.dirty = false;
            st.fitted_for = (0, 0);
            btn.set_sensitive(false);
            area.queue_draw();
        });
    }
    outer.append(&apply_btn);

    Some(outer.upcast())
}
