//! Left-right balance of an output device. A device has one volume per channel (front-left, front-right, ...). Balance is
//! how those volumes differ: centred when the left and the right are equal, to the right when the left ones are quieter.
//! The master volume (the loudest channel) is kept when the balance changes, and the balance is kept when the volume changes,
//! the way PulseAudio and Plasma's own volume applet treat it. Each device keeps its own, as the volumes belong to the device.

use serde_json::Value;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Left,
    Right,
    Middle,
}

pub fn side_of(channel: &str) -> Side {
    let c = channel.to_lowercase();
    if c.contains("left") {
        Side::Left
    } else if c.contains("right") {
        Side::Right
    } else {
        Side::Middle
    }
}

/// A device can be balanced when it has at least one channel on each side (a mono or multi-"aux" device cannot).
pub fn can_balance(channels: &[String]) -> bool {
    channels.iter().any(|c| side_of(c) == Side::Left) && channels.iter().any(|c| side_of(c) == Side::Right)
}

/// The loudest channel, in percent.
pub fn master(volumes: &[f64]) -> f64 {
    volumes.iter().cloned().fold(0.0, f64::max)
}

/// -1 (all to the left) .. 0 (centred) .. 1 (all to the right).
pub fn current(channels: &[String], volumes: &[f64]) -> f64 {
    let side = |s: Side| {
        channels.iter().zip(volumes).filter(|(c, _)| side_of(c) == s).map(|(_, v)| *v).fold(0.0, f64::max)
    };
    let (l, r) = (side(Side::Left), side(Side::Right));
    if l <= 0.0 && r <= 0.0 {
        0.0
    } else if (l - r).abs() < 0.5 {
        0.0 // within rounding: the percentages are whole numbers
    } else if r > l {
        1.0 - l / r
    } else {
        r / l - 1.0
    }
}

/// The volume of each channel for a master volume and a balance. The quieter side is turned down, the other keeps the master.
pub fn volumes_for(channels: &[String], master_percent: f64, balance: f64) -> Vec<f64> {
    let b = balance.clamp(-1.0, 1.0);
    channels
        .iter()
        .map(|c| match side_of(c) {
            Side::Left => master_percent * (1.0 - b.max(0.0)),
            Side::Right => master_percent * (1.0 + b.min(0.0)),
            Side::Middle => master_percent,
        })
        .collect()
}

/// `pactl` arguments that set a sink to these channel volumes (one percentage per channel, in the channel map's order).
pub fn set_volume_args(sink: &str, volumes: &[f64]) -> Vec<String> {
    let mut a = vec!["set-sink-volume".to_string(), sink.to_string()];
    a.extend(volumes.iter().map(|v| format!("{}%", v.round().max(0.0) as u32)));
    a
}

/// What a sink looks like for balancing: its channels in order and their volumes in percent.
#[derive(Clone, Debug, PartialEq)]
pub struct Sink {
    pub name: String,
    pub channels: Vec<String>,
    pub volumes: Vec<f64>,
}

impl Sink {
    pub fn balance(&self) -> f64 {
        current(&self.channels, &self.volumes)
    }
    pub fn master(&self) -> f64 {
        master(&self.volumes)
    }
}

/// From one entry of `pactl -f json list sinks`.
pub fn parse_sink(v: &Value) -> Option<Sink> {
    let name = v["name"].as_str()?.to_string();
    let channels: Vec<String> = v["channel_map"].as_str()?.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    let vol = v["volume"].as_object()?;
    let volumes: Vec<f64> = channels
        .iter()
        .map(|c| vol.get(c).and_then(|e| e["value_percent"].as_str()).and_then(|s| s.trim_end_matches('%').parse().ok()).unwrap_or(0.0))
        .collect();
    (!channels.is_empty() && volumes.len() == channels.len()).then_some(Sink { name, channels, volumes })
}

/// "Centred", "Left 30%", "Right 5%".
pub fn describe(balance: f64) -> String {
    let pct = (balance.abs() * 100.0).round() as i32;
    if pct == 0 {
        "Centered".to_string()
    } else if balance < 0.0 {
        format!("Left {pct}%")
    } else {
        format!("Right {pct}%")
    }
}

/// The arguments that set `sink` to a new balance, keeping its master volume.
pub fn args_for_balance(sink: &Sink, balance: f64) -> Vec<String> {
    set_volume_args(&sink.name, &volumes_for(&sink.channels, sink.master(), balance))
}

/// The arguments that set `sink` to a new master volume, keeping its balance.
pub fn args_for_master(sink: &Sink, master_percent: f64) -> Vec<String> {
    set_volume_args(&sink.name, &volumes_for(&sink.channels, master_percent, sink.balance()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ch(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn centred_when_both_sides_are_equal() {
        assert_eq!(current(&ch(&["front-left", "front-right"]), &[80.0, 80.0]), 0.0);
    }

    #[test]
    fn balance_is_how_much_the_quieter_side_is_turned_down() {
        let c = ch(&["front-left", "front-right"]);
        assert!((current(&c, &[50.0, 100.0]) - 0.5).abs() < 1e-9);
        assert!((current(&c, &[100.0, 25.0]) + 0.75).abs() < 1e-9);
        assert_eq!(current(&c, &[0.0, 0.0]), 0.0);
    }

    #[test]
    fn rounding_of_whole_percentages_is_not_a_balance() {
        assert_eq!(current(&ch(&["front-left", "front-right"]), &[80.0, 80.4]), 0.0);
    }

    #[test]
    fn volumes_for_keep_the_louder_side_at_the_master_volume() {
        let c = ch(&["front-left", "front-right"]);
        assert_eq!(volumes_for(&c, 90.0, 0.0), vec![90.0, 90.0]);
        assert_eq!(volumes_for(&c, 90.0, 0.5), vec![45.0, 90.0]);
        assert_eq!(volumes_for(&c, 90.0, -0.5), vec![90.0, 45.0]);
        assert_eq!(volumes_for(&c, 90.0, 1.0), vec![0.0, 90.0]);
        assert_eq!(volumes_for(&c, 90.0, 5.0), vec![0.0, 90.0], "out of range is clamped");
    }

    #[test]
    fn setting_a_balance_and_reading_it_back_agree() {
        let c = ch(&["front-left", "front-right"]);
        for b in [-1.0, -0.6, -0.1, 0.0, 0.25, 0.8, 1.0] {
            let v = volumes_for(&c, 100.0, b);
            assert!((current(&c, &v) - b).abs() < 0.011, "balance {b}: {v:?}");
        }
    }

    #[test]
    fn surround_devices_move_every_left_and_right_channel_and_leave_the_centre_alone() {
        let c = ch(&["front-left", "front-right", "front-center", "lfe", "rear-left", "rear-right"]);
        assert_eq!(volumes_for(&c, 100.0, 0.5), vec![50.0, 100.0, 100.0, 100.0, 50.0, 100.0]);
        assert!(can_balance(&c));
    }

    #[test]
    fn devices_without_a_left_and_a_right_cannot_be_balanced() {
        assert!(!can_balance(&ch(&["mono"])));
        assert!(!can_balance(&ch(&["aux0", "aux1", "aux2", "aux3"])));
        assert!(can_balance(&ch(&["front-left", "front-right"])));
    }

    #[test]
    fn changing_the_volume_keeps_the_balance_and_changing_the_balance_keeps_the_volume() {
        let sink = Sink { name: "dev".into(), channels: ch(&["front-left", "front-right"]), volumes: vec![40.0, 80.0] };
        assert_eq!(args_for_master(&sink, 60.0), ["set-sink-volume", "dev", "30%", "60%"]);
        assert_eq!(args_for_balance(&sink, 0.0), ["set-sink-volume", "dev", "80%", "80%"]);
        // a quarter to the left: the right side is turned down
        assert_eq!(args_for_balance(&sink, -0.25), ["set-sink-volume", "dev", "80%", "60%"]);
    }

    #[test]
    fn a_sink_is_read_from_pactls_json() {
        let v = json!({"name": "alsa_output.x", "channel_map": "front-left,front-right",
            "volume": {"front-left": {"value": 1, "value_percent": "100%"}, "front-right": {"value": 1, "value_percent": "50%"}}});
        let s = parse_sink(&v).unwrap();
        assert_eq!(s.volumes, vec![100.0, 50.0]);
        assert!((s.balance() - 0.5).abs() < 1e-9 || (s.balance() + 0.5).abs() < 1e-9);
        assert!(parse_sink(&json!({"name": "x"})).is_none());
    }

    #[test]
    fn balance_is_described_in_words() {
        assert_eq!(describe(0.0), "Centered");
        assert_eq!(describe(-0.3), "Left 30%");
        assert_eq!(describe(0.004), "Centered");
        assert_eq!(describe(0.05), "Right 5%");
    }
}

#[cfg(test)]
mod live {
    use super::*;
    use std::process::Command;

    fn sink(name_part: &str) -> Option<Sink> {
        let out = Command::new("pactl").args(["-f", "json", "list", "sinks"]).output().ok()?;
        let all: Vec<Value> = serde_json::from_slice(&out.stdout).ok()?;
        all.iter().filter(|d| d["name"].as_str().map(|n| n.contains(name_part)).unwrap_or(false)).find_map(parse_sink)
    }

    /// Sets the balance of a silent HDMI output on the real PipeWire, checks it reads back with the volume kept, checks that a
    /// volume change keeps the balance, and puts the original volumes back. (Never touches the speakers or the earbuds.)
    #[test]
    #[ignore]
    fn live_balance_on_a_real_output_is_set_read_back_and_restored() {
        let before = sink("HDMI3__sink").expect("a silent HDMI sink");
        println!("before: {before:?} balance {:.2}", before.balance());
        let name = before.name.clone();
        let run = |args: Vec<String>| assert!(Command::new("pactl").args(&args).status().unwrap().success());

        run(args_for_balance(&before, 0.6));
        let after = sink("HDMI3__sink").unwrap();
        println!("balance +0.6: {:?} -> balance {:.2}", after.volumes, after.balance());
        assert!((after.balance() - 0.6).abs() < 0.03, "balance read back wrongly");
        assert!((after.master() - before.master()).abs() <= 1.0, "master volume must stay: {} -> {}", before.master(), after.master());

        run(args_for_master(&after, 50.0));
        let quieter = sink("HDMI3__sink").unwrap();
        println!("volume 50: {:?} -> balance {:.2}", quieter.volumes, quieter.balance());
        assert!((quieter.master() - 50.0).abs() <= 1.0);
        assert!((quieter.balance() - 0.6).abs() < 0.03, "a volume change must keep the balance");

        run(args_for_balance(&quieter, -0.4));
        let left = sink("HDMI3__sink").unwrap();
        println!("balance -0.4: {:?} -> balance {:.2}", left.volumes, left.balance());
        assert!((left.balance() + 0.4).abs() < 0.03);

        // back to exactly what it was
        run(set_volume_args(&name, &before.volumes));
        let restored = sink("HDMI3__sink").unwrap();
        println!("restored: {:?}", restored.volumes);
        assert_eq!(restored.volumes, before.volumes);
    }
}
