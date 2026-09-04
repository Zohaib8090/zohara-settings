//! Sound settings — output / input devices, master volume, mic mute.
//!
//! Talks to PipeWire (or PulseAudio) over D-Bus via the wpctl / pactl CLI.
//! We don't link libpulse or libwireplumber directly because the binaries
//! are already on every install and the protocol is stable.

use gtk4::prelude::*;
use libadwaita as adw;
use adw::prelude::*;
use std::process::Command;

fn pw_or_pa() -> &'static str {
    // Prefer wpctl (PipeWire's official CLI), fall back to pactl.
    if Command::new("sh")
        .args(["-c", "command -v wpctl >/dev/null"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
    {
        "wpctl"
    } else {
        "pactl"
    }
}

fn list_sinks() -> Vec<(String, String)> {
    let cmd = pw_or_pa();
    let out = Command::new(cmd)
        .args(["status"])
        .output()
        .or_else(|_| Command::new("pactl").args(["list", "short", "sinks"]).output());
    let stdout = match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
        _ => return Vec::new(),
    };
    if cmd == "wpctl" {
        // wpctl status format: "* id. name [vol]"
        stdout
            .lines()
            .filter_map(|l| {
                let parts: Vec<&str> = l.split_whitespace().collect();
                if parts.len() >= 2 && (parts[0].starts_with('*') || parts[0].starts_with("○")) {
                    Some((parts[1].to_string(), parts[1].to_string()))
                } else {
                    None
                }
            })
            .collect()
    } else {
        // pactl short format: id<TAB>name<TAB>...
        stdout
            .lines()
            .filter_map(|l| {
                let mut parts = l.split('\t');
                let id = parts.next()?.to_string();
                let name = parts.next()?.to_string();
                Some((id, name))
            })
            .collect()
    }
}

fn set_default_sink(name: &str) {
    let cmd = pw_or_pa();
    if cmd == "wpctl" {
        let _ = Command::new(cmd).args(["set-default", name]).spawn();
    } else {
        let _ = Command::new(cmd).args(["set-default-sink", name]).spawn();
    }
}

fn set_volume(percent: u32) {
    let cmd = pw_or_pa();
    let value = ((percent as f32 / 100.0) * 1.0) as i32; // both CLIs take 0..1
    if cmd == "wpctl" {
        let _ = Command::new(cmd)
            .args(["set-volume", "@DEFAULT_AUDIO_SINK@", &format!("{value}%")])
            .spawn();
    } else {
        let _ = Command::new(cmd)
            .args(["set-sink-volume", "@DEFAULT_SINK@", &format!("{value}%")])
            .spawn();
    }
}

fn toggle_mute(mute: bool) {
    let cmd = pw_or_pa();
    let arg = if mute { "1" } else { "0" };
    if cmd == "wpctl" {
        let _ = Command::new(cmd)
            .args(["set-mute", "@DEFAULT_AUDIO_SINK@", arg])
            .spawn();
    } else {
        let _ = Command::new(cmd)
            .args(["set-sink-mute", "@DEFAULT_SINK@", arg])
            .spawn();
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

    let title = gtk4::Label::builder()
        .label("Sound")
        .halign(gtk4::Align::Start)
        .css_classes(vec!["win11-page-title".to_string()])
        .build();
    root.append(&title);

    let rows = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    rows.set_css_classes(&["win11-card-group"]);

    // 1. Output device picker
    let out_exp = adw::ExpanderRow::new();
    out_exp.set_title("Output device");
    out_exp.set_subtitle("Choose where sound plays");
    out_exp.add_prefix(&gtk4::Image::from_icon_name("audio-speakers-symbolic"));
    let sinks = list_sinks();
    if sinks.is_empty() {
        let row = adw::ActionRow::new();
        row.set_title("No output devices detected");
        row.set_subtitle("Make sure PipeWire or PulseAudio is running");
        out_exp.add_row(&row);
    } else {
        for (id, name) in &sinks {
            let row = adw::ActionRow::new();
            row.set_title(name);
            row.set_subtitle(&format!("ID: {id}"));
            row.set_activatable(true);
            let name_owned = name.clone();
            row.connect_activated(move |_| set_default_sink(&name_owned));
            out_exp.add_row(&row);
        }
    }
    rows.append(&out_exp);

    // 2. Master volume
    let vol_exp = adw::ExpanderRow::new();
    vol_exp.set_title("Volume");
    vol_exp.set_subtitle("Master output level");
    vol_exp.add_prefix(&gtk4::Image::from_icon_name("audio-volume-high-symbolic"));

    let scale = gtk4::Scale::with_range(gtk4::Orientation::Horizontal, 0.0, 100.0, 1.0);
    scale.set_value(70.0);
    scale.set_size_request(360, -1);
    scale.set_hexpand(true);
    scale.set_draw_value(true);
    scale.set_value_pos(gtk4::PositionType::Right);
    let scale_clone = scale.clone();
    scale.connect_value_changed(move |s| {
        set_volume(s.value() as u32);
        let _ = scale_clone;
    });
    let vol_row = adw::ActionRow::new();
    vol_row.set_title("Master volume");
    vol_row.add_suffix(&scale);
    vol_row.set_activatable(false);
    vol_exp.add_row(&vol_row);
    rows.append(&vol_exp);

    // 3. Mute toggle
    let mute = adw::SwitchRow::new();
    mute.set_title("Mute");
    mute.set_subtitle("Silence all output");
    mute.connect_active_notify(|row| toggle_mute(row.is_active()));
    rows.append(&mute);

    // 4. Input device section
    let in_exp = adw::ExpanderRow::new();
    in_exp.set_title("Input device");
    in_exp.set_subtitle("Choose your microphone");
    in_exp.add_prefix(&gtk4::Image::from_icon_name("audio-input-microphone-symbolic"));
    let mic_row = adw::ActionRow::new();
    mic_row.set_title("Built-in audio (default)");
    mic_row.set_activatable(false);
    in_exp.add_row(&mic_row);
    rows.append(&in_exp);

    root.append(&rows);
    scroll.set_child(Some(&root));
    scroll.upcast()
}
