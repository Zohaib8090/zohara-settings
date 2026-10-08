//! Sound settings -- output/input devices with real volume and mute state,
//! plus a per-application volume mixer.
//!
//! Talks to PipeWire through its PulseAudio-compatible server (pipewire-pulse,
//! enabled on Zohara OS) using `pactl -f json`. State is always read back from
//! the server rather than assumed, so the sliders and switches reflect reality.

use crate::backend::kconfig;
use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::process::Command;
use std::rc::Rc;

/// The most the sliders reach when "raise maximum volume" is on, like Plasma's volume applet.
const BOOST_MAX: f64 = 150.0;

/// Plasma's own switch (plasmaparc, General/RaiseMaximumVolume), so the tray applet and this page agree.
fn boost_enabled() -> bool {
    kconfig::read("plasmaparc", &["General"], "RaiseMaximumVolume").as_deref() == Some("true")
}

fn pactl_json(kind: &str) -> Vec<Value> {
    Command::new("pactl")
        .args(["-f", "json", "list", kind])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| serde_json::from_slice::<Vec<Value>>(&o.stdout).ok())
        .unwrap_or_default()
}

fn pactl_text(args: &[&str]) -> String {
    Command::new("pactl")
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

// Fire-and-forget on a worker thread so slider drags never block the UI and
// finished children are always reaped.
fn pactl_run(args: Vec<String>) {
    std::thread::spawn(move || {
        let _ = Command::new("pactl").args(&args).status();
    });
}

fn volume_percent(v: &Value) -> u32 {
    v["volume"]
        .as_object()
        .and_then(|m| m.values().next())
        .and_then(|c| c["value_percent"].as_str())
        .and_then(|s| s.trim_end_matches('%').parse().ok())
        .unwrap_or(0)
}

fn device_label(v: &Value) -> String {
    v["description"]
        .as_str()
        .or_else(|| v["name"].as_str())
        .unwrap_or("Unknown device")
        .to_string()
}

/// A volume slider. It reaches 150% when boost is on, or when the stream is already louder than 100%.
fn volume_scale(percent: u32, boost: bool) -> gtk4::Scale {
    let max = if boost || percent > 100 { BOOST_MAX } else { 100.0 };
    let scale = gtk4::Scale::with_range(gtk4::Orientation::Horizontal, 0.0, max, 1.0);
    if max > 100.0 {
        scale.add_mark(100.0, gtk4::PositionType::Bottom, None);
    }
    scale.set_value((percent as f64).min(max));
    scale.set_size_request(240, -1);
    scale.set_hexpand(true);
    scale.set_draw_value(true);
    scale.set_value_pos(gtk4::PositionType::Right);
    scale.set_valign(gtk4::Align::Center);
    scale
}

// ── Long device names ──────────────────────────────────────────────────────

/// Slowly slides a too-long label sideways inside `scroll` to show all of it, once, while the pointer is over it.
/// `token` changes when the pointer leaves (or another slide starts), which stops the running one.
fn start_marquee(scroll: &gtk4::ScrolledWindow, token: &Rc<Cell<u32>>) {
    const SPEED: f64 = 45.0; // pixels a second: slow enough to read
    const DELAY_MS: u128 = 450;
    let adj = scroll.hadjustment();
    let travel = adj.upper() - adj.page_size();
    if travel <= 1.0 {
        return;
    }
    let mine = token.get().wrapping_add(1);
    token.set(mine);
    let (token, started) = (token.clone(), std::time::Instant::now());
    glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
        if token.get() != mine {
            return glib::ControlFlow::Break;
        }
        let ms = started.elapsed().as_millis();
        let pos = if ms < DELAY_MS { 0.0 } else { ((ms - DELAY_MS) as f64 / 1000.0 * SPEED).min(travel) };
        adj.set_value(pos);
        if pos >= travel {
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}

fn stop_marquee(scroll: &gtk4::ScrolledWindow, token: &Rc<Cell<u32>>) {
    token.set(token.get().wrapping_add(1));
    scroll.hadjustment().set_value(0.0);
}

/// Rows for a device list's pop-up: the whole name, which slides into view when the pointer rests on a long one,
/// and a tick on the chosen device. (The default rows cut a long name short with "…".)
fn marquee_factory() -> gtk4::SignalListItemFactory {
    let factory = gtk4::SignalListItemFactory::new();
    factory.connect_setup(|_, obj| {
        let Some(item) = obj.downcast_ref::<gtk4::ListItem>() else { return };
        let label = gtk4::Label::new(None);
        label.set_xalign(0.0);
        label.set_margin_top(6);
        label.set_margin_bottom(6);
        let scroll = gtk4::ScrolledWindow::new();
        scroll.set_policy(gtk4::PolicyType::External, gtk4::PolicyType::Never);
        scroll.set_hexpand(true);
        scroll.set_min_content_width(260);
        scroll.set_child(Some(&label));
        let tick = gtk4::Image::from_icon_name("object-select-symbolic");
        item.bind_property("selected", &tick, "visible").sync_create().build();
        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
        row.append(&scroll);
        row.append(&tick);

        let token = Rc::new(Cell::new(0u32));
        let motion = gtk4::EventControllerMotion::new();
        {
            let (scroll, token) = (scroll.clone(), token.clone());
            motion.connect_enter(move |_, _, _| start_marquee(&scroll, &token));
        }
        {
            let (scroll, token) = (scroll.clone(), token.clone());
            motion.connect_leave(move |_| stop_marquee(&scroll, &token));
        }
        row.add_controller(motion);
        item.set_child(Some(&row));
    });
    factory.connect_bind(|_, obj| {
        let Some(item) = obj.downcast_ref::<gtk4::ListItem>() else { return };
        let Some(text) = item.item().and_downcast::<gtk4::StringObject>() else { return };
        let label = item
            .child()
            .and_downcast::<gtk4::Box>()
            .and_then(|b| b.first_child())
            .and_downcast::<gtk4::ScrolledWindow>()
            .and_then(|sw| sw.child())
            .and_then(|c| c.first_child().or(Some(c)))
            .and_downcast::<gtk4::Label>();
        if let Some(label) = label {
            label.set_text(&text.string());
            label.set_tooltip_text(Some(&text.string()));
        }
    });
    factory
}

/// A device picker whose chosen name is shown in full under the title (wrapping if it has to), with the sliding pop-up.
fn device_combo(title: &str, labels: &[String]) -> adw::ComboRow {
    let row = adw::ComboRow::new();
    row.set_title(title);
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    row.set_model(Some(&gtk4::StringList::new(&refs)));
    row.set_use_subtitle(true);
    row.set_list_factory(Some(&marquee_factory()));
    row
}

/// Picker for "which device does this app use": the devices, the index of the one it is on now, and what to do on a pick.
fn route_row(title: &str, devices: &[Value], current: Option<u64>, on_pick: impl Fn(String) + 'static) -> adw::ComboRow {
    let labels: Vec<String> = devices.iter().map(device_label).collect();
    let row = device_combo(title, &labels);
    if let Some(pos) = devices.iter().position(|d| d["index"].as_u64() == current) {
        row.set_selected(pos as u32);
    }
    let names: Vec<String> = devices.iter().map(|d| d["name"].as_str().unwrap_or_default().to_string()).collect();
    // Connected after set_selected, so showing the current device doesn't "move" the app to it.
    row.connect_selected_notify(move |r| {
        if let Some(name) = names.get(r.selected() as usize) {
            on_pick(name.clone());
        }
    });
    row
}

/// Device picker + volume + mute for either outputs (sinks) or inputs (sources). Returns the volume slider.
fn append_device_section(rows: &gtk4::Box, output: bool, boost: bool) -> Option<gtk4::Scale> {
    let (kind, default_getter, set_default, set_volume, set_mute, default_token) = if output {
        ("sinks", "get-default-sink", "set-default-sink", "set-sink-volume", "set-sink-mute", "@DEFAULT_SINK@")
    } else {
        ("sources", "get-default-source", "set-default-source", "set-source-volume", "set-source-mute", "@DEFAULT_SOURCE@")
    };

    let devices: Vec<Value> = pactl_json(kind)
        .into_iter()
        // Every sink has a "monitor" source; hide those from the input list.
        .filter(|d| output || d["monitor_of_sink"].is_null())
        .collect();

    let picker = adw::ComboRow::new();
    picker.set_title(if output { "Output device" } else { "Input device" });
    picker.set_use_subtitle(true);
    picker.set_list_factory(Some(&marquee_factory()));
    picker.add_prefix(&gtk4::Image::from_icon_name(if output {
        "audio-speakers-symbolic"
    } else {
        "audio-input-microphone-symbolic"
    }));

    if devices.is_empty() {
        picker.set_subtitle(if output {
            "No output devices detected. Make sure PipeWire is running."
        } else {
            "No input devices detected"
        });
        picker.set_sensitive(false);
        rows.append(&picker);
        return None;
    }

    let labels: Vec<String> = devices.iter().map(device_label).collect();
    let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    picker.set_model(Some(&gtk4::StringList::new(&label_refs)));

    let default_name = pactl_text(&[default_getter]);
    let default_idx = devices
        .iter()
        .position(|d| d["name"].as_str() == Some(default_name.as_str()))
        .unwrap_or(0);
    picker.set_selected(default_idx as u32);

    let names: Vec<String> = devices
        .iter()
        .map(|d| d["name"].as_str().unwrap_or_default().to_string())
        .collect();
    // Connected after set_selected so initialising the row doesn't re-apply the default.
    picker.connect_selected_notify(move |row| {
        if let Some(name) = names.get(row.selected() as usize) {
            pactl_run(vec![set_default.into(), name.clone()]);
        }
    });
    rows.append(&picker);

    let current = &devices[default_idx];

    // The microphone stays at 100%: boosting the input only adds noise.
    let scale = volume_scale(volume_percent(current), output && boost);
    scale.connect_value_changed(move |s| {
        pactl_run(vec![set_volume.into(), default_token.into(), format!("{}%", s.value() as u32)]);
    });
    let vol_row = adw::ActionRow::new();
    vol_row.set_title(if output { "Volume" } else { "Input volume" });
    vol_row.add_prefix(&gtk4::Image::from_icon_name(if output {
        "audio-volume-high-symbolic"
    } else {
        "microphone-sensitivity-high-symbolic"
    }));
    vol_row.add_suffix(&scale);
    vol_row.set_activatable(false);
    rows.append(&vol_row);

    let mute = adw::SwitchRow::new();
    mute.set_title("Mute");
    mute.set_active(current["mute"].as_bool().unwrap_or(false));
    mute.connect_active_notify(move |row| {
        pactl_run(vec![set_mute.into(), default_token.into(), (row.is_active() as u8).to_string()]);
    });
    rows.append(&mute);
    Some(scale)
}

fn stream_signature(inputs: &[Value]) -> String {
    inputs
        .iter()
        // The device is part of it: an app moved elsewhere (here or by another program) refreshes its picker.
        .map(|i| format!("{}:{}:{}:{}", i["index"], i["properties"]["application.name"], i["sink"], i["source"]))
        .collect::<Vec<_>>()
        .join("|")
}

fn refresh_apps(list: &gtk4::Box, last_sig: &Rc<RefCell<String>>, force: bool) {
    let inputs = pactl_json("sink-inputs");
    let sig = stream_signature(&inputs);
    // Rebuilding while the set of streams is unchanged would destroy a slider mid-drag.
    if !force && *last_sig.borrow() == sig {
        return;
    }
    *last_sig.borrow_mut() = sig;
    let boost = boost_enabled();
    let sinks = pactl_json("sinks");

    while let Some(child) = list.first_child() {
        list.remove(&child);
    }

    if inputs.is_empty() {
        let row = adw::ActionRow::new();
        row.set_title("No applications are playing audio");
        row.set_subtitle("Apps appear here while they are producing sound");
        row.set_activatable(false);
        list.append(&row);
        return;
    }

    for input in &inputs {
        let idx = input["index"].as_u64().unwrap_or(0).to_string();
        let props = &input["properties"];
        let app = props["application.name"]
            .as_str()
            .or_else(|| props["media.name"].as_str())
            .unwrap_or("Unknown application");

        let row = adw::ActionRow::new();
        row.set_title(app);
        if let Some(media) = props["media.name"].as_str().filter(|m| *m != app) {
            row.set_subtitle(media);
        }
        row.set_activatable(false);
        row.add_prefix(&gtk4::Image::from_icon_name(
            props["application.icon_name"].as_str().unwrap_or("audio-x-generic-symbolic"),
        ));

        let scale = volume_scale(volume_percent(input), boost);
        let idx_v = idx.clone();
        scale.connect_value_changed(move |s| {
            pactl_run(vec!["set-sink-input-volume".into(), idx_v.clone(), format!("{}%", s.value() as u32)]);
        });

        let mute = gtk4::ToggleButton::new();
        mute.set_icon_name("audio-volume-muted-symbolic");
        mute.set_css_classes(&["flat"]);
        mute.set_valign(gtk4::Align::Center);
        mute.set_tooltip_text(Some("Mute this application"));
        mute.set_active(input["mute"].as_bool().unwrap_or(false));
        let idx_m = idx.clone();
        mute.connect_toggled(move |b| {
            pactl_run(vec!["set-sink-input-mute".into(), idx_m.clone(), (b.is_active() as u8).to_string()]);
        });

        row.add_suffix(&scale);
        row.add_suffix(&mute);
        list.append(&row);

        // Which speakers or headphones this app plays on.
        if sinks.len() > 1 {
            let idx_r = idx.clone();
            list.append(&route_row("Play on", &sinks, input["sink"].as_u64(), move |sink| {
                pactl_run(vec!["move-sink-input".into(), idx_r.clone(), sink]);
            }));
        }
    }
}

/// Apps that are recording from a microphone, with the microphone each one uses.
fn mic_streams(outputs: &[Value]) -> Vec<&Value> {
    outputs
        .iter()
        .filter(|o| {
            let p = &o["properties"];
            // Level meters (the volume applet, mixers) and "monitor" captures of speakers are not apps using a microphone.
            p["media.name"].as_str() != Some("Peak detect") && p["stream.monitor"].as_str() != Some("true")
        })
        .collect()
}

fn refresh_mic_apps(list: &gtk4::Box, last_sig: &Rc<RefCell<String>>, force: bool) {
    let all = pactl_json("source-outputs");
    let outputs = mic_streams(&all);
    let sig = outputs
        .iter()
        .map(|i| format!("{}:{}:{}", i["index"], i["properties"]["application.name"], i["source"]))
        .collect::<Vec<_>>()
        .join("|");
    if !force && *last_sig.borrow() == sig {
        return;
    }
    *last_sig.borrow_mut() = sig;
    // Real microphones only: every speaker also appears as a "monitor" source.
    let sources: Vec<Value> = pactl_json("sources").into_iter().filter(|d| d["monitor_of_sink"].is_null()).collect();

    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    if outputs.is_empty() {
        let row = adw::ActionRow::new();
        row.set_title("No applications are using a microphone");
        row.set_subtitle("Apps appear here while they are recording or in a call");
        row.set_activatable(false);
        list.append(&row);
        return;
    }
    for output in outputs {
        let idx = output["index"].as_u64().unwrap_or(0).to_string();
        let props = &output["properties"];
        let app = props["application.name"].as_str().or_else(|| props["media.name"].as_str()).unwrap_or("Unknown application");
        let row = adw::ActionRow::new();
        row.set_title(app);
        row.set_activatable(false);
        row.add_prefix(&gtk4::Image::from_icon_name(props["application.icon_name"].as_str().unwrap_or("audio-input-microphone-symbolic")));
        list.append(&row);
        if sources.len() > 1 {
            let idx_r = idx.clone();
            list.append(&route_row("Listen with", &sources, output["source"].as_u64(), move |source| {
                pactl_run(vec!["move-source-output".into(), idx_r.clone(), source]);
            }));
        }
    }
}

// ── Speaker test ───────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
enum Channel {
    Left,
    Right,
    Both,
}

/// A short 16-bit stereo WAV tone that sounds only on the chosen channel(s).
fn tone_wav(channel: Channel) -> Vec<u8> {
    const RATE: u32 = 48_000;
    const SECS: f32 = 0.9;
    const FREQ: f32 = 523.25; // C5: clear on laptop speakers, not harsh
    let frames = (RATE as f32 * SECS) as u32;
    let data_len = frames * 4;
    let mut w = Vec::with_capacity(44 + data_len as usize);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_len).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&2u16.to_le_bytes()); // stereo
    w.extend_from_slice(&RATE.to_le_bytes());
    w.extend_from_slice(&(RATE * 4).to_le_bytes());
    w.extend_from_slice(&4u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_len.to_le_bytes());
    let fade = RATE as f32 * 0.03;
    for i in 0..frames {
        let t = i as f32 / RATE as f32;
        // Fade in/out so the tone doesn't click.
        let env = (i as f32 / fade).min((frames - i) as f32 / fade).min(1.0);
        let v = ((t * FREQ * std::f32::consts::TAU).sin() * env * 0.5 * i16::MAX as f32) as i16;
        let (l, r) = match channel {
            Channel::Left => (v, 0),
            Channel::Right => (0, v),
            Channel::Both => (v, v),
        };
        w.extend_from_slice(&l.to_le_bytes());
        w.extend_from_slice(&r.to_le_bytes());
    }
    w
}

fn play_tone(channel: Channel) {
    std::thread::spawn(move || {
        let name = match channel {
            Channel::Left => "left",
            Channel::Right => "right",
            Channel::Both => "both",
        };
        let path = std::env::temp_dir().join(format!("zohara-speaker-test-{name}.wav"));
        if std::fs::write(&path, tone_wav(channel)).is_err() {
            return;
        }
        // Both play to the current default output device.
        let played = Command::new("pw-play").arg(&path).status().map(|s| s.success()).unwrap_or(false);
        if !played {
            let _ = Command::new("paplay").arg(&path).status();
        }
        let _ = std::fs::remove_file(&path);
    });
}

fn speaker_test_group() -> gtk4::Box {
    let group = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    group.set_css_classes(&["win11-card-group"]);

    let row = adw::ActionRow::new();
    row.set_title("Test speakers");
    row.set_subtitle("Plays a short tone on the selected output device");
    row.add_prefix(&gtk4::Image::from_icon_name("audio-speakers-symbolic"));
    row.set_activatable(false);

    let buttons = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    buttons.set_valign(gtk4::Align::Center);
    for (label, channel) in [("Left", Channel::Left), ("Right", Channel::Right), ("Both", Channel::Both)] {
        let b = gtk4::Button::with_label(label);
        b.set_tooltip_text(Some(match channel {
            Channel::Left => "Play a tone on the left speaker only",
            Channel::Right => "Play a tone on the right speaker only",
            Channel::Both => "Play a tone on both speakers",
        }));
        b.connect_clicked(move |_| play_tone(channel));
        buttons.append(&b);
    }
    row.add_suffix(&buttons);
    group.append(&row);
    group
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
            .label("Sound")
            .halign(gtk4::Align::Start)
            .css_classes(vec!["win11-page-title".to_string()])
            .build(),
    );

    let boost = boost_enabled();
    let output = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    output.set_css_classes(&["win11-card-group"]);
    let out_scale = append_device_section(&output, true, boost);
    let boost_row = adw::SwitchRow::new();
    boost_row.set_title("Allow volume above 100%");
    boost_row.set_subtitle("Lets the volume go up to 150%. Very loud sound can sound distorted and harm your hearing");
    boost_row.add_prefix(&gtk4::Image::from_icon_name("audio-volume-overamplified-symbolic"));
    boost_row.set_active(boost);
    output.append(&boost_row);
    root.append(&output);
    root.append(&speaker_test_group());

    let input = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    input.set_css_classes(&["win11-card-group"]);
    append_device_section(&input, false, false);
    root.append(&input);

    let mixer_header = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    mixer_header.append(
        &gtk4::Label::builder()
            .label("Volume mixer")
            .halign(gtk4::Align::Start)
            .hexpand(true)
            .css_classes(vec!["heading".to_string()])
            .build(),
    );
    let refresh_btn = gtk4::Button::from_icon_name("view-refresh-symbolic");
    refresh_btn.set_css_classes(&["flat"]);
    refresh_btn.set_tooltip_text(Some("Refresh"));
    mixer_header.append(&refresh_btn);
    root.append(&mixer_header);

    let apps = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    apps.set_css_classes(&["win11-card-group"]);
    let sig = Rc::new(RefCell::new(String::new()));
    refresh_apps(&apps, &sig, true);
    root.append(&apps);

    root.append(
        &gtk4::Label::builder()
            .label("Apps using a microphone")
            .halign(gtk4::Align::Start)
            .css_classes(vec!["heading".to_string()])
            .build(),
    );
    let mics = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    mics.set_css_classes(&["win11-card-group"]);
    let mic_sig = Rc::new(RefCell::new(String::new()));
    refresh_mic_apps(&mics, &mic_sig, true);
    root.append(&mics);

    {
        let (apps, sig, mics, mic_sig) = (apps.clone(), sig.clone(), mics.clone(), mic_sig.clone());
        refresh_btn.connect_clicked(move |_| {
            refresh_apps(&apps, &sig, true);
            refresh_mic_apps(&mics, &mic_sig, true);
        });
    }

    // The boost switch is Plasma's own, so the volume applet changes with it; the sliders here follow at once.
    {
        let (apps, sig) = (apps.clone(), sig.clone());
        boost_row.connect_active_notify(move |r| {
            let on = r.is_active();
            if let Some(scale) = &out_scale {
                scale.adjustment().set_upper(if on { BOOST_MAX } else { 100.0 });
                if !on && scale.value() > 100.0 {
                    scale.set_value(100.0); // takes the volume itself back to 100%
                }
                scale.clear_marks();
                if on {
                    scale.add_mark(100.0, gtk4::PositionType::Bottom, None);
                }
            }
            kconfig::spawn(move || kconfig::write_notify("plasmaparc", &["General"], "RaiseMaximumVolume", if on { "true" } else { "false" }));
            // Apps louder than 100% are taken back with it, then the mixer is redrawn with the new limit.
            let apps = apps.clone();
            let sig = sig.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(300), move || refresh_apps(&apps, &sig, true));
        });
    }

    // Pick up apps that start or stop playing while the page is open; stops
    // itself once the page is destroyed.
    let weak = apps.downgrade();
    glib::timeout_add_seconds_local(3, move || match weak.upgrade() {
        Some(list) => {
            if list.is_mapped() {
                refresh_apps(&list, &sig, false);
                refresh_mic_apps(&mics, &mic_sig, false);
            }
            glib::ControlFlow::Continue
        }
        None => glib::ControlFlow::Break,
    });

    scroll.set_child(Some(&root));
    scroll.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn level_meters_and_speaker_monitors_are_not_apps_using_a_microphone() {
        let all = vec![
            json!({"index": 1, "source": 57, "properties": {"application.name": "Discord", "media.name": "recStream"}}),
            json!({"index": 2, "source": 57, "properties": {"application.name": "plasmashell", "media.name": "Peak detect"}}),
            json!({"index": 3, "source": 99, "properties": {"application.name": "OBS", "media.name": "Audio", "stream.monitor": "true"}}),
        ];
        let kept: Vec<u64> = mic_streams(&all).iter().map(|o| o["index"].as_u64().unwrap()).collect();
        assert_eq!(kept, vec![1]);
    }

    #[test]
    fn moving_an_app_to_another_device_changes_the_stream_signature() {
        let before = vec![json!({"index": 4, "sink": 56, "properties": {"application.name": "Spotify"}})];
        let after = vec![json!({"index": 4, "sink": 16860, "properties": {"application.name": "Spotify"}})];
        assert_ne!(stream_signature(&before), stream_signature(&after));
    }

    #[test]
    fn a_stream_louder_than_100_percent_still_gets_a_slider_that_reaches_it() {
        // Not a widget test (no display here): the limit rule the slider uses.
        let max = |percent: u32, boost: bool| if boost || percent > 100 { BOOST_MAX } else { 100.0 };
        assert_eq!(max(80, false), 100.0);
        assert_eq!(max(80, true), 150.0);
        assert_eq!(max(130, false), 150.0);
    }
}

