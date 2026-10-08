//! Sound > Equalizer: a system-wide equalizer with ten bands, bass and treble, a preamp with automatic headroom,
//! twenty-one presets plus the person's own (saved, shared as files), and a live graph of what the sound is being
//! shaped to. The sound engine and its tests are in `backend::equalizer`; this is the screen.

use crate::backend::equalizer::{self as eq, LiveApplier, Settings, State, BANDS, BAND_LABELS, CUSTOM, GAIN_LIMIT};
use crate::backend::worker::in_background;
use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

struct Ctx {
    state: RefCell<State>,
    running: Cell<bool>,
    updating: Cell<bool>,
    save_pending: Cell<bool>,
    applier: LiveApplier,
    enable: adw::SwitchRow,
    preset: adw::ComboRow,
    names: RefCell<Vec<String>>,
    delete_btn: gtk4::Button,
    bands: Vec<gtk4::Scale>,
    value_labels: Vec<gtk4::Label>,
    preamp: gtk4::Scale,
    preamp_row: adw::ActionRow,
    headroom: adw::SwitchRow,
    bass: gtk4::Scale,
    treble: gtk4::Scale,
    q: adw::SpinRow,
    graph: gtk4::DrawingArea,
}

fn gain_text(g: f64) -> String {
    if g.abs() < 0.05 {
        "0".into()
    } else {
        format!("{g:+.1}")
    }
}

fn window_of(w: &impl IsA<gtk4::Widget>) -> Option<gtk4::Window> {
    w.root().and_downcast::<gtk4::Window>()
}

fn error_dialog(from: &impl IsA<gtk4::Widget>, title: &str, body: &str) {
    let d = adw::AlertDialog::new(Some(title), Some(body));
    d.add_response("ok", "OK");
    d.present(Some(from));
}

impl Ctx {
    /// What the controls say now.
    fn read(&self) -> Settings {
        let mut s = self.state.borrow().settings.clone();
        for (i, b) in self.bands.iter().enumerate() {
            s.gains[i] = b.value();
        }
        s.preamp = self.preamp.value();
        s.bass = self.bass.value();
        s.treble = self.treble.value();
        s.q = self.q.value();
        s.auto_headroom = self.headroom.is_active();
        s.clamped()
    }

    /// Shows `s` in the controls, without that counting as a change made by the person.
    fn show(&self, s: &Settings) {
        self.updating.set(true);
        for (i, b) in self.bands.iter().enumerate() {
            b.set_value(s.gains[i]);
        }
        self.preamp.set_value(s.preamp);
        self.bass.set_value(s.bass);
        self.treble.set_value(s.treble);
        self.q.set_value(s.q);
        self.headroom.set_active(s.auto_headroom);
        self.updating.set(false);
        self.refresh_labels();
    }

    fn refresh_labels(&self) {
        let s = self.read();
        for (i, l) in self.value_labels.iter().enumerate() {
            l.set_text(&gain_text(s.gains[i]));
        }
        let eff = s.effective_preamp();
        self.preamp_row.set_subtitle(&if s.auto_headroom {
            format!("{} dB in use: lowered by the biggest boost so the sound doesn't distort", gain_text(eff))
        } else {
            format!("{} dB", gain_text(eff))
        });
        self.graph.queue_draw();
    }

    fn select_preset(&self, name: &str) {
        let names = self.names.borrow();
        let pos = names.iter().position(|n| n == name).unwrap_or(names.len().saturating_sub(1));
        self.updating.set(true);
        self.preset.set_selected(pos as u32);
        self.updating.set(false);
        let removable = !eq::is_builtin(name) && name != CUSTOM && names.iter().any(|n| n == name);
        self.delete_btn.set_sensitive(removable);
    }

    fn rebuild_preset_list(&self) {
        let names = self.state.borrow().preset_names();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        self.updating.set(true);
        self.preset.set_model(Some(&gtk4::StringList::new(&refs)));
        self.updating.set(false);
        *self.names.borrow_mut() = names;
        let current = self.state.borrow().preset.clone();
        self.select_preset(&current);
    }

    /// Remembers the settings a moment after the last change, not on every step of a drag.
    fn save_soon(self: &Rc<Self>) {
        if self.save_pending.replace(true) {
            return;
        }
        let ctx = self.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(600), move || {
            ctx.save_pending.set(false);
            eq::save_state(&ctx.state.borrow());
        });
    }

    /// Sound the new settings: to the running equalizer and, for next time, to the saved config.
    fn push(self: &Rc<Self>, s: &Settings) {
        if self.running.get() {
            self.applier.send(s.clone());
        }
        self.save_soon();
    }

    /// A control was moved by the person.
    fn changed(self: &Rc<Self>) {
        if self.updating.get() {
            return;
        }
        let s = self.read();
        {
            let mut st = self.state.borrow_mut();
            st.settings = s.clone();
            st.preset = CUSTOM.into();
        }
        self.select_preset(CUSTOM);
        self.refresh_labels();
        self.push(&s);
    }

    /// Loads settings (a preset, a file, a reset) into the controls and the sound.
    fn load(self: &Rc<Self>, s: Settings, preset: &str) {
        {
            let mut st = self.state.borrow_mut();
            st.settings = s.clone();
            st.preset = preset.to_string();
        }
        self.show(&s);
        self.select_preset(preset);
        self.push(&s);
    }
}

// ── The graph ──────────────────────────────────────────────────────────────

fn draw_graph(ctx: &Ctx, cr: &gtk4::cairo::Context, w: i32, h: i32, fg: gtk4::gdk::RGBA, accent: gtk4::gdk::RGBA) {
    const DB_RANGE: f64 = 18.0;
    let (left, right, top, bottom) = (34.0, 8.0, 8.0, 20.0);
    let (pw, ph) = (w as f64 - left - right, h as f64 - top - bottom);
    if pw < 10.0 || ph < 10.0 {
        return;
    }
    let x_of = |f: f64| left + pw * ((f / 20.0).ln() / (20000.0_f64 / 20.0).ln());
    let y_of = |db: f64| top + ph * (0.5 - db.clamp(-DB_RANGE, DB_RANGE) / (2.0 * DB_RANGE));
    let rgba = |c: &gtk4::gdk::RGBA, a: f64| cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, a);

    cr.set_line_width(1.0);
    cr.select_font_face("sans", gtk4::cairo::FontSlant::Normal, gtk4::cairo::FontWeight::Normal);
    cr.set_font_size(10.0);
    // dB grid
    for db in [-12.0, -6.0, 0.0, 6.0, 12.0] {
        let y = y_of(db);
        rgba(&fg, if db == 0.0 { 0.45 } else { 0.15 });
        cr.move_to(left, y);
        cr.line_to(left + pw, y);
        let _ = cr.stroke();
        rgba(&fg, 0.6);
        cr.move_to(4.0, y + 3.5);
        let _ = cr.show_text(&format!("{db:+.0}"));
    }
    // frequency grid
    for (f, label) in BANDS.iter().zip(BAND_LABELS.iter()) {
        let x = x_of(*f);
        rgba(&fg, 0.1);
        cr.move_to(x, top);
        cr.line_to(x, top + ph);
        let _ = cr.stroke();
        rgba(&fg, 0.6);
        cr.move_to(x - 6.0, h as f64 - 6.0);
        let _ = cr.show_text(label);
    }

    // the curve
    let s = ctx.read();
    let points: Vec<(f64, f64)> = (0..=240)
        .map(|i| {
            let f = 20.0 * (1000.0_f64).powf(i as f64 / 240.0);
            (x_of(f), y_of(eq::shape_db(&s, f)))
        })
        .collect();
    let zero = y_of(0.0);
    cr.move_to(points[0].0, zero);
    for (x, y) in &points {
        cr.line_to(*x, *y);
    }
    cr.line_to(points[points.len() - 1].0, zero);
    cr.close_path();
    rgba(&accent, 0.18);
    let _ = cr.fill();
    cr.move_to(points[0].0, points[0].1);
    for (x, y) in &points[1..] {
        cr.line_to(*x, *y);
    }
    cr.set_line_width(2.2);
    rgba(&accent, 1.0);
    let _ = cr.stroke();
    // a dot on each band's own frequency
    for f in BANDS {
        let (x, y) = (x_of(f), y_of(eq::shape_db(&s, f)));
        cr.arc(x, y, 3.0, 0.0, std::f64::consts::TAU);
        rgba(&accent, 1.0);
        let _ = cr.fill();
    }
}

// ── The screen ─────────────────────────────────────────────────────────────

fn slider(min: f64, max: f64, step: f64) -> gtk4::Scale {
    let s = gtk4::Scale::with_range(gtk4::Orientation::Horizontal, min, max, step);
    s.set_draw_value(true);
    s.set_value_pos(gtk4::PositionType::Right);
    s.set_hexpand(true);
    s.set_size_request(260, -1);
    s.set_valign(gtk4::Align::Center);
    s.add_mark(0.0, gtk4::PositionType::Bottom, None);
    s
}

pub fn section() -> gtk4::Widget {
    let state = eq::load_state();
    let running = eq::is_running();

    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    let title = gtk4::Label::builder().label("Equalizer").halign(gtk4::Align::Start).css_classes(vec!["heading".to_string()]).build();
    root.append(&title);

    // On / off, and which preset.
    let top = adw::PreferencesGroup::new();
    let enable = adw::SwitchRow::new();
    enable.set_title("Equalizer");
    enable.set_subtitle("Shape the sound of everything you play");
    enable.add_prefix(&gtk4::Image::from_icon_name("audio-x-generic-symbolic"));
    enable.set_active(running);
    top.add(&enable);
    let preset = adw::ComboRow::new();
    preset.set_title("Preset");
    preset.add_prefix(&gtk4::Image::from_icon_name("view-list-symbolic"));
    top.add(&preset);
    let actions = adw::ActionRow::new();
    actions.set_title("Your presets");
    actions.set_subtitle("Save the sliders as a preset, share one as a file, or start again");
    let button = |label: &str, tip: &str| {
        let b = gtk4::Button::with_label(label);
        b.set_valign(gtk4::Align::Center);
        b.set_tooltip_text(Some(tip));
        b
    };
    let save_btn = button("Save", "Save the current sliders as a preset");
    let delete_btn = button("Delete", "Delete the chosen preset of yours");
    delete_btn.set_sensitive(false);
    let import_btn = button("Import", "Load a preset from a file");
    let export_btn = button("Export", "Save the current sliders to a file");
    let reset_btn = button("Reset", "Back to flat");
    let box_btns = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    for b in [&save_btn, &delete_btn, &import_btn, &export_btn, &reset_btn] {
        box_btns.append(b);
    }
    actions.add_suffix(&box_btns);
    top.add(&actions);
    root.append(&top);

    // The graph.
    let graph = gtk4::DrawingArea::new();
    graph.set_content_height(170);
    graph.set_hexpand(true);
    let graph_card = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    graph_card.add_css_class("card");
    graph_card.append(&graph);
    root.append(&graph_card);

    // Ten bands.
    let bands_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    bands_box.set_homogeneous(true);
    bands_box.set_margin_top(10);
    bands_box.set_margin_bottom(10);
    let mut bands = Vec::new();
    let mut value_labels = Vec::new();
    for (i, label) in BAND_LABELS.iter().enumerate() {
        let col = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        let value = gtk4::Label::new(Some("0"));
        value.add_css_class("caption");
        let scale = gtk4::Scale::with_range(gtk4::Orientation::Vertical, -GAIN_LIMIT, GAIN_LIMIT, 0.5);
        scale.set_inverted(true);
        scale.set_draw_value(false);
        scale.set_vexpand(true);
        scale.set_size_request(-1, 150);
        scale.set_halign(gtk4::Align::Center);
        scale.add_mark(0.0, gtk4::PositionType::Right, None);
        scale.set_tooltip_text(Some(&format!("{} Hz. Double-click to set it back to 0", BANDS[i])));
        let hz = gtk4::Label::new(Some(label));
        hz.add_css_class("caption");
        hz.add_css_class("dim-label");
        col.append(&value);
        col.append(&scale);
        col.append(&hz);
        bands_box.append(&col);
        bands.push(scale);
        value_labels.push(value);
    }
    let bands_card = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    bands_card.add_css_class("card");
    bands_card.append(&bands_box);
    root.append(&bands_card);

    // Preamp, tone, width.
    let tone = adw::PreferencesGroup::new();
    let preamp = slider(eq::PREAMP_MIN, eq::PREAMP_MAX, 0.5);
    let preamp_row = adw::ActionRow::new();
    preamp_row.set_title("Preamp");
    preamp_row.add_prefix(&gtk4::Image::from_icon_name("audio-volume-high-symbolic"));
    preamp_row.add_suffix(&preamp);
    tone.add(&preamp_row);
    let headroom = adw::SwitchRow::new();
    headroom.set_title("Automatic headroom");
    headroom.set_subtitle("Lower the preamp when you boost a band, so loud music doesn't distort");
    tone.add(&headroom);
    let bass = slider(-GAIN_LIMIT, GAIN_LIMIT, 0.5);
    let bass_row = adw::ActionRow::new();
    bass_row.set_title("Bass");
    bass_row.set_subtitle("Everything below about 120 Hz");
    bass_row.add_suffix(&bass);
    tone.add(&bass_row);
    let treble = slider(-GAIN_LIMIT, GAIN_LIMIT, 0.5);
    let treble_row = adw::ActionRow::new();
    treble_row.set_title("Treble");
    treble_row.set_subtitle("Everything above about 7 kHz");
    treble_row.add_suffix(&treble);
    tone.add(&treble_row);
    let q = adw::SpinRow::with_range(0.3, 6.0, 0.1);
    q.set_title("Band width");
    q.set_subtitle("Lower is wider and smoother, higher is narrower and sharper");
    q.set_digits(2);
    tone.add(&q);
    root.append(&tone);

    let ctx = Rc::new(Ctx {
        state: RefCell::new(state),
        running: Cell::new(running),
        updating: Cell::new(false),
        save_pending: Cell::new(false),
        applier: LiveApplier::new(),
        enable: enable.clone(),
        preset: preset.clone(),
        names: RefCell::new(Vec::new()),
        delete_btn: delete_btn.clone(),
        bands,
        value_labels,
        preamp,
        preamp_row,
        headroom,
        bass,
        treble,
        q,
        graph: graph.clone(),
    });

    // Draw the graph in the theme's colours.
    {
        let ctx = ctx.clone();
        graph.set_draw_func(move |area, cr, w, h| {
            let fg = area.color();
            let accent = adw::StyleManager::default().accent_color_rgba();
            draw_graph(&ctx, cr, w, h, fg, accent);
        });
    }

    let initial = ctx.state.borrow().settings.clone();
    ctx.rebuild_preset_list();
    ctx.show(&initial);

    // Moving a control.
    let controls: Vec<gtk4::Scale> = ctx.bands.iter().cloned().chain([ctx.preamp.clone(), ctx.bass.clone(), ctx.treble.clone()]).collect();
    for s in &controls {
        let ctx = ctx.clone();
        s.connect_value_changed(move |_| ctx.changed());
    }
    {
        let ctx = ctx.clone();
        ctx.clone().q.connect_value_notify(move |_| ctx.changed());
    }
    {
        let ctx = ctx.clone();
        ctx.clone().headroom.connect_active_notify(move |_| ctx.changed());
    }
    // Double-click a band to set it back to zero.
    for s in ctx.bands.iter().chain([&ctx.bass, &ctx.treble]) {
        let click = gtk4::GestureClick::new();
        let s2 = s.clone();
        click.connect_pressed(move |_, n, _, _| {
            if n == 2 {
                s2.set_value(0.0);
            }
        });
        s.add_controller(click);
    }

    // Choosing a preset.
    {
        let ctx = ctx.clone();
        preset.connect_selected_notify(move |row| {
            if ctx.updating.get() {
                return;
            }
            let Some(name) = ctx.names.borrow().get(row.selected() as usize).cloned() else { return };
            if name == CUSTOM {
                ctx.select_preset(CUSTOM);
                return;
            }
            let Some(mut s) = ctx.state.borrow().find_preset(&name) else { return };
            // A built-in preset sets the shape; the person's width and headroom choices stay.
            if eq::is_builtin(&name) {
                let cur = ctx.read();
                s.q = cur.q;
                s.auto_headroom = cur.auto_headroom;
                s.preamp = cur.preamp;
            }
            ctx.load(s, &name);
        });
    }

    // Presets: save, delete, reset, import, export.
    {
        let ctx = ctx.clone();
        save_btn.connect_clicked(move |b| {
            let entry = gtk4::Entry::new();
            entry.set_placeholder_text(Some("Name of the preset"));
            entry.set_activates_default(true);
            let d = adw::AlertDialog::new(Some("Save this sound as a preset"), Some("It will be in the list with the others."));
            d.set_extra_child(Some(&entry));
            d.add_responses(&[("cancel", "Cancel"), ("save", "Save")]);
            d.set_response_appearance("save", adw::ResponseAppearance::Suggested);
            d.set_default_response(Some("save"));
            let (ctx, b2) = (ctx.clone(), b.clone());
            d.connect_response(None, move |_, resp| {
                if resp != "save" {
                    return;
                }
                let current = ctx.read();
                let mut st = ctx.state.borrow_mut();
                st.settings = current;
                match st.save_custom(&entry.text()) {
                    Ok(()) => {
                        drop(st);
                        ctx.rebuild_preset_list();
                        ctx.save_soon();
                    }
                    Err(e) => {
                        drop(st);
                        error_dialog(&b2, "Couldn't save the preset", &e);
                    }
                }
            });
            d.present(Some(b));
        });
    }
    {
        let ctx = ctx.clone();
        delete_btn.connect_clicked(move |_| {
            let name = ctx.state.borrow().preset.clone();
            if ctx.state.borrow_mut().delete_custom(&name) {
                ctx.rebuild_preset_list();
                ctx.save_soon();
            }
        });
    }
    {
        let ctx = ctx.clone();
        reset_btn.connect_clicked(move |_| {
            let cur = ctx.read();
            ctx.load(Settings { q: cur.q, auto_headroom: cur.auto_headroom, ..Settings::default() }, "Flat");
        });
    }
    {
        let ctx = ctx.clone();
        export_btn.connect_clicked(move |b| {
            let dialog = gtk4::FileDialog::new();
            dialog.set_title("Export this preset");
            let name = ctx.state.borrow().preset.clone();
            dialog.set_initial_name(Some(&format!("{}.zohara-eq.json", name.replace(['/', '\\'], "-"))));
            let (settings, b2) = (ctx.read(), b.clone());
            dialog.save(window_of(b).as_ref(), gtk4::gio::Cancellable::NONE, move |res| {
                if let Ok(file) = res {
                    if let Some(path) = file.path() {
                        if let Err(e) = std::fs::write(&path, eq::export_preset(&name, &settings)) {
                            error_dialog(&b2, "Couldn't save the file", &e.to_string());
                        }
                    }
                }
            });
        });
    }
    {
        let ctx = ctx.clone();
        import_btn.connect_clicked(move |b| {
            let dialog = gtk4::FileDialog::new();
            dialog.set_title("Import a preset");
            let (ctx, b2) = (ctx.clone(), b.clone());
            dialog.open(window_of(b).as_ref(), gtk4::gio::Cancellable::NONE, move |res| {
                let Ok(file) = res else { return };
                let Some(path) = file.path() else { return };
                let result = std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| eq::import_preset(&t));
                match result {
                    Ok(mut p) => {
                        if eq::is_builtin(&p.name) || p.name == CUSTOM {
                            p.name = format!("{} (imported)", p.name);
                        }
                        ctx.load(p.settings.clone(), CUSTOM);
                        let mut st = ctx.state.borrow_mut();
                        let _ = st.save_custom(&p.name);
                        drop(st);
                        ctx.rebuild_preset_list();
                        ctx.save_soon();
                    }
                    Err(e) => error_dialog(&b2, "Couldn't import the preset", &e),
                }
            });
        });
    }

    // On / off.
    {
        let ctx = ctx.clone();
        enable.connect_active_notify(move |row| {
            if ctx.updating.get() {
                return;
            }
            let on = row.is_active();
            if on == ctx.running.get() {
                return;
            }
            row.set_sensitive(false);
            let mut st = ctx.state.borrow().clone();
            st.settings = ctx.read();
            let (ctx2, row2) = (ctx.clone(), row.clone());
            in_background(
                move || {
                    let result = if on { eq::start(&mut st) } else { eq::stop(&mut st); Ok(()) };
                    (st, result)
                },
                move |(st, result)| {
                    row2.set_sensitive(true);
                    let ok = result.is_ok();
                    {
                        let mut mine = ctx2.state.borrow_mut();
                        mine.enabled = st.enabled;
                        mine.target = st.target.clone();
                    }
                    ctx2.running.set(if on { ok } else { false });
                    if on && ok {
                        ctx2.applier.send(ctx2.read()); // the controls may have moved while it started
                        row2.set_subtitle("On. Everything you play goes through it");
                    } else {
                        row2.set_subtitle("Shape the sound of everything you play");
                    }
                    if let Err(e) = result {
                        ctx2.updating.set(true);
                        row2.set_active(false);
                        ctx2.updating.set(false);
                        error_dialog(&row2, "The equalizer couldn't start", &e);
                    }
                },
            );
        });
    }
    if running {
        enable.set_subtitle("On. Everything you play goes through it");
    }

    root.upcast()
}
