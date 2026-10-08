//! A system-wide equalizer for everything Zohara plays.
//!
//! It is PipeWire's own filter chain (built-in biquad filters, no plug-ins) running as a small separate process,
//! `pipewire -c <config>`, that provides a virtual output called `zohara_eq`. While it is on, `zohara_eq` is the default
//! output: apps play into it, it shapes the sound and hands it to the real device it was pointed at. Gains change live
//! (`pw-cli set-param`), so dragging a slider needs no restart and makes no click.
//!
//! Layout of the chain: preamp -> ten peaking bands -> bass shelf -> treble shelf.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::{Command, Stdio};

pub const BANDS: [f64; 10] = [31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];
pub const BAND_LABELS: [&str; 10] = ["31", "62", "125", "250", "500", "1k", "2k", "4k", "8k", "16k"];
pub const GAIN_LIMIT: f64 = 12.0;
pub const PREAMP_MIN: f64 = -24.0;
pub const PREAMP_MAX: f64 = 12.0;
/// The virtual output apps play into, and the stream that carries the result to the real device.
pub const SINK_NAME: &str = "zohara_eq";
pub const OUT_NAME: &str = "zohara_eq_out";
const BASS_FREQ: f64 = 120.0;
const TREBLE_FREQ: f64 = 7000.0;
const SHELF_Q: f64 = 0.7;
const SAMPLE_RATE: f64 = 48000.0;

// ── What the equalizer is set to ───────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// Gain of the whole chain, in dB.
    pub preamp: f64,
    pub gains: [f64; 10],
    /// Low shelf at 120 Hz, in dB.
    pub bass: f64,
    /// High shelf at 7 kHz, in dB.
    pub treble: f64,
    /// Width of the ten bands (higher is narrower).
    pub q: f64,
    /// Lower the preamp by the biggest boost, so boosting never clips.
    pub auto_headroom: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { preamp: 0.0, gains: [0.0; 10], bass: 0.0, treble: 0.0, q: 1.41, auto_headroom: true }
    }
}

impl Settings {
    /// The preamp actually used: the chosen one, lowered by the biggest boost when headroom is automatic.
    pub fn effective_preamp(&self) -> f64 {
        let boost = self.gains.iter().chain([&self.bass, &self.treble]).cloned().fold(0.0_f64, f64::max);
        let p = if self.auto_headroom { self.preamp - boost } else { self.preamp };
        p.clamp(PREAMP_MIN, PREAMP_MAX)
    }

    pub fn clamped(mut self) -> Settings {
        for g in self.gains.iter_mut() {
            *g = g.clamp(-GAIN_LIMIT, GAIN_LIMIT);
        }
        self.bass = self.bass.clamp(-GAIN_LIMIT, GAIN_LIMIT);
        self.treble = self.treble.clamp(-GAIN_LIMIT, GAIN_LIMIT);
        self.preamp = self.preamp.clamp(PREAMP_MIN, PREAMP_MAX);
        self.q = self.q.clamp(0.3, 6.0);
        self
    }

    /// Every control of the chain with its value, in the names the running filter graph uses.
    pub fn controls(&self) -> Vec<(String, f64)> {
        let mut c = vec![("pre:Mult".to_string(), db_to_linear(self.effective_preamp()))];
        for (i, g) in self.gains.iter().enumerate() {
            c.push((format!("eq{}:Gain", i + 1), *g));
            c.push((format!("eq{}:Q", i + 1), self.q));
        }
        c.push(("bass:Gain".into(), self.bass));
        c.push(("treble:Gain".into(), self.treble));
        c
    }
}

pub fn db_to_linear(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    pub settings: Settings,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub enabled: bool,
    /// The real output the sound is handed to.
    pub target: String,
    /// Name of the chosen preset, or "Custom".
    pub preset: String,
    pub settings: Settings,
    pub custom: Vec<Preset>,
}

pub const CUSTOM: &str = "Custom";

impl Default for State {
    fn default() -> Self {
        State { enabled: false, target: String::new(), preset: "Flat".into(), settings: Settings::default(), custom: Vec::new() }
    }
}

// ── Presets ────────────────────────────────────────────────────────────────

/// (name, ten band gains, bass, treble). Bands: 31, 62, 125, 250, 500, 1k, 2k, 4k, 8k, 16k Hz.
const BUILTIN: [(&str, [f64; 10], f64, f64); 21] = [
    ("Flat", [0.0; 10], 0.0, 0.0),
    ("Bass boost", [6.0, 5.0, 4.0, 2.5, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0], 0.0, 0.0),
    ("Bass reducer", [-6.0, -5.0, -4.0, -2.0, -1.0, 0.0, 0.0, 0.0, 0.0, 0.0], 0.0, 0.0),
    ("Treble boost", [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.5, 4.0, 5.0, 6.0], 0.0, 0.0),
    ("Treble reducer", [0.0, 0.0, 0.0, 0.0, 0.0, -1.0, -2.5, -4.0, -5.0, -6.0], 0.0, 0.0),
    ("Vocal boost", [-3.0, -2.0, -1.0, 0.0, 2.0, 4.0, 4.0, 3.0, 1.0, 0.0], 0.0, 0.0),
    ("Loudness", [6.0, 4.0, 1.0, 0.0, -1.0, -1.0, 0.0, 1.0, 3.0, 5.0], 0.0, 0.0),
    ("Rock", [5.0, 4.0, 3.0, 1.0, -1.0, -1.0, 1.0, 3.0, 4.0, 5.0], 0.0, 0.0),
    ("Pop", [-1.0, 2.0, 4.0, 5.0, 3.0, 0.0, -1.0, -1.0, -1.0, -1.0], 0.0, 0.0),
    ("Jazz", [3.0, 2.0, 1.0, 2.0, -2.0, -2.0, 0.0, 1.0, 2.0, 3.0], 0.0, 0.0),
    ("Classical", [4.0, 3.0, 2.0, 1.0, -1.0, -1.0, 0.0, 2.0, 3.0, 4.0], 0.0, 0.0),
    ("Electronic", [5.0, 4.0, 1.0, 0.0, -2.0, 2.0, 1.0, 1.0, 4.0, 5.0], 0.0, 0.0),
    ("Hip-hop", [5.0, 4.0, 1.0, 3.0, -1.0, -1.0, 1.0, -1.0, 2.0, 3.0], 0.0, 0.0),
    ("Acoustic", [4.0, 4.0, 3.0, 1.0, 2.0, 2.0, 3.0, 3.0, 3.0, 2.0], 0.0, 0.0),
    ("Movie", [4.0, 3.0, 1.0, 0.0, 0.0, 1.0, 2.0, 3.0, 3.0, 2.0], 2.0, 0.0),
    ("Gaming", [2.0, 1.0, 0.0, -1.0, -1.0, 1.0, 3.0, 4.0, 2.0, 0.0], 0.0, 0.0),
    ("Podcast and voice", [-6.0, -5.0, -3.0, -1.0, 1.0, 3.0, 4.0, 3.0, 0.0, -2.0], 0.0, 0.0),
    ("Small laptop speakers", [-5.0, -3.0, 2.0, 3.0, 2.0, 0.0, 0.0, 1.0, 2.0, 3.0], 0.0, 0.0),
    ("Earbuds clarity", [3.0, 2.0, 1.0, 0.0, 0.0, 0.0, 1.0, 2.0, 2.0, 1.0], 0.0, 0.0),
    ("Night listening", [-4.0, -3.0, -2.0, 0.0, 1.0, 1.0, 0.0, -2.0, -3.0, -4.0], 0.0, 0.0),
    ("Warm", [2.0, 2.0, 2.0, 1.0, 0.0, 0.0, -1.0, -2.0, -3.0, -4.0], 0.0, 0.0),
];

pub fn builtin_presets() -> Vec<Preset> {
    BUILTIN
        .iter()
        .map(|(name, gains, bass, treble)| Preset {
            name: name.to_string(),
            settings: Settings { gains: *gains, bass: *bass, treble: *treble, ..Settings::default() },
        })
        .collect()
}

pub fn is_builtin(name: &str) -> bool {
    BUILTIN.iter().any(|(n, ..)| *n == name)
}

impl State {
    /// Names for the preset list: built-in first, then the person's own, then "Custom".
    pub fn preset_names(&self) -> Vec<String> {
        let mut names: Vec<String> = BUILTIN.iter().map(|(n, ..)| n.to_string()).collect();
        names.extend(self.custom.iter().map(|p| p.name.clone()));
        names.push(CUSTOM.to_string());
        names
    }

    pub fn find_preset(&self, name: &str) -> Option<Settings> {
        builtin_presets().into_iter().chain(self.custom.iter().cloned()).find(|p| p.name == name).map(|p| p.settings)
    }

    /// Saves (or replaces) one of the person's own presets. A built-in name or an empty one is refused.
    pub fn save_custom(&mut self, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("Give the preset a name.".into());
        }
        if name.len() > 60 || name.contains(['\n', '\r']) {
            return Err("That name is too long.".into());
        }
        if is_builtin(name) || name == CUSTOM {
            return Err("A built-in preset already has that name.".into());
        }
        let preset = Preset { name: name.to_string(), settings: self.settings.clone() };
        match self.custom.iter_mut().find(|p| p.name == name) {
            Some(p) => *p = preset,
            None => self.custom.push(preset),
        }
        self.preset = name.to_string();
        Ok(())
    }

    pub fn delete_custom(&mut self, name: &str) -> bool {
        let before = self.custom.len();
        self.custom.retain(|p| p.name != name);
        if self.preset == name {
            self.preset = CUSTOM.into();
        }
        self.custom.len() != before
    }
}

// ── The response curve (for the graph) ─────────────────────────────────────

#[derive(Clone, Copy)]
enum Kind {
    Peaking,
    LowShelf,
    HighShelf,
}

/// RBJ biquad coefficients (b0, b1, b2, a1, a2, with a0 divided out).
fn biquad(kind: Kind, f0: f64, q: f64, gain_db: f64) -> [f64; 5] {
    let a = 10f64.powf(gain_db / 40.0);
    let w0 = 2.0 * std::f64::consts::PI * f0 / SAMPLE_RATE;
    let (sin, cos) = (w0.sin(), w0.cos());
    let alpha = sin / (2.0 * q);
    let (b0, b1, b2, a0, a1, a2) = match kind {
        Kind::Peaking => (1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * cos, 1.0 - alpha / a),
        Kind::LowShelf => {
            let t = 2.0 * a.sqrt() * alpha;
            (
                a * ((a + 1.0) - (a - 1.0) * cos + t),
                2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                a * ((a + 1.0) - (a - 1.0) * cos - t),
                (a + 1.0) + (a - 1.0) * cos + t,
                -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                (a + 1.0) + (a - 1.0) * cos - t,
            )
        }
        Kind::HighShelf => {
            let t = 2.0 * a.sqrt() * alpha;
            (
                a * ((a + 1.0) + (a - 1.0) * cos + t),
                -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                a * ((a + 1.0) + (a - 1.0) * cos - t),
                (a + 1.0) - (a - 1.0) * cos + t,
                2.0 * ((a - 1.0) - (a + 1.0) * cos),
                (a + 1.0) - (a - 1.0) * cos - t,
            )
        }
    };
    [b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0]
}

/// Level of a biquad at `freq`, in dB.
fn magnitude_db(c: &[f64; 5], freq: f64) -> f64 {
    let w = 2.0 * std::f64::consts::PI * freq / SAMPLE_RATE;
    let (c1, s1, c2, s2) = (w.cos(), w.sin(), (2.0 * w).cos(), (2.0 * w).sin());
    let (nr, ni) = (c[0] + c[1] * c1 + c[2] * c2, -(c[1] * s1 + c[2] * s2));
    let (dr, di) = (1.0 + c[3] * c1 + c[4] * c2, -(c[3] * s1 + c[4] * s2));
    10.0 * ((nr * nr + ni * ni) / (dr * dr + di * di)).log10()
}

/// The whole chain's level at `freq` in dB, preamp included.
pub fn response_db(s: &Settings, freq: f64) -> f64 {
    let mut total = s.effective_preamp();
    for (f0, g) in BANDS.iter().zip(s.gains.iter()) {
        if *g != 0.0 {
            total += magnitude_db(&biquad(Kind::Peaking, *f0, s.q, *g), freq);
        }
    }
    if s.bass != 0.0 {
        total += magnitude_db(&biquad(Kind::LowShelf, BASS_FREQ, SHELF_Q, s.bass), freq);
    }
    if s.treble != 0.0 {
        total += magnitude_db(&biquad(Kind::HighShelf, TREBLE_FREQ, SHELF_Q, s.treble), freq);
    }
    total
}

// ── Files ──────────────────────────────────────────────────────────────────

fn config_dir() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config"));
    base.join("zohara")
}

pub fn state_path() -> PathBuf {
    config_dir().join("equalizer.json")
}

pub fn conf_path() -> PathBuf {
    config_dir().join("equalizer").join("filter-chain.conf")
}

pub fn load_state() -> State {
    std::fs::read_to_string(state_path())
        .ok()
        .and_then(|t| serde_json::from_str::<State>(&t).ok())
        .map(|mut s| {
            s.settings = s.settings.clamped();
            s
        })
        .unwrap_or_default()
}

pub fn save_state(state: &State) {
    let path = state_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(text) = serde_json::to_string_pretty(state) {
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(tmp, path);
        }
    }
}

/// Settings in a file a person can share.
pub fn export_preset(name: &str, settings: &Settings) -> String {
    serde_json::to_string_pretty(&Preset { name: name.to_string(), settings: settings.clone() }).unwrap_or_default()
}

pub fn import_preset(text: &str) -> Result<Preset, String> {
    let mut p: Preset = serde_json::from_str(text).map_err(|_| "That file isn't an equalizer preset.".to_string())?;
    p.settings = p.settings.clamped();
    if p.name.trim().is_empty() {
        p.name = "Imported".into();
    }
    Ok(p)
}

/// A sink name that is safe inside the config file.
fn safe_target(t: &str) -> bool {
    !t.is_empty() && t.len() < 200 && t.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-:@+".contains(&b))
}

/// The config `pipewire -c` runs: the chain with these settings, handing its sound to `target`.
pub fn filter_chain_conf(s: &Settings, target: &str) -> String {
    let mut nodes = String::new();
    let mut links = String::new();
    nodes += &format!("          {{ type = builtin name = pre label = linear control = {{ Mult = {:.6} Add = 0.0 }} }}\n", db_to_linear(s.effective_preamp()));
    let mut prev = "pre".to_string();
    for (i, (f, g)) in BANDS.iter().zip(s.gains.iter()).enumerate() {
        let name = format!("eq{}", i + 1);
        nodes += &format!("          {{ type = builtin name = {name} label = bq_peaking control = {{ Freq = {f:.1} Q = {:.3} Gain = {g:.3} }} }}\n", s.q);
        links += &format!("          {{ output = \"{prev}:Out\" input = \"{name}:In\" }}\n");
        prev = name;
    }
    nodes += &format!("          {{ type = builtin name = bass label = bq_lowshelf control = {{ Freq = {BASS_FREQ:.1} Q = {SHELF_Q:.3} Gain = {:.3} }} }}\n", s.bass);
    links += &format!("          {{ output = \"{prev}:Out\" input = \"bass:In\" }}\n");
    nodes += &format!("          {{ type = builtin name = treble label = bq_highshelf control = {{ Freq = {TREBLE_FREQ:.1} Q = {SHELF_Q:.3} Gain = {:.3} }} }}\n", s.treble);
    links += "          { output = \"bass:Out\" input = \"treble:In\" }\n";
    let target_line = if safe_target(target) { format!("target.object = \"{target}\"") } else { String::new() };
    format!(
        "# Written by Zohara Settings (Sound > Equalizer). Changes here are overwritten.\n\
         context.modules = [\n\
         \x20 {{ name = libpipewire-module-rt args = {{ nice.level = -11 }} flags = [ ifexists nofail ] }}\n\
         \x20 {{ name = libpipewire-module-protocol-native }}\n\
         \x20 {{ name = libpipewire-module-client-node }}\n\
         \x20 {{ name = libpipewire-module-adapter }}\n\
         \x20 {{ name = libpipewire-module-filter-chain\n\
         \x20   args = {{\n\
         \x20     node.description = \"Zohara Equalizer\"\n\
         \x20     media.name = \"Zohara Equalizer\"\n\
         \x20     audio.channels = 2\n\
         \x20     audio.position = [ FL FR ]\n\
         \x20     filter.graph = {{\n\
         \x20       nodes = [\n{nodes}\x20       ]\n\
         \x20       links = [\n{links}\x20       ]\n\
         \x20       inputs = [ \"pre:In\" ]\n\
         \x20       outputs = [ \"treble:Out\" ]\n\
         \x20     }}\n\
         \x20     capture.props = {{ node.name = \"{SINK_NAME}\" media.class = Audio/Sink }}\n\
         \x20     playback.props = {{ node.name = \"{OUT_NAME}\" node.passive = true {target_line} }}\n\
         \x20   }}\n\
         \x20 }}\n\
         ]\n"
    )
}

// ── Running it ─────────────────────────────────────────────────────────────

fn run_quiet(cmd: &str, args: &[&str]) -> Option<String> {
    Command::new(cmd)
        .args(args)
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
}

pub fn is_running() -> bool {
    run_quiet("pactl", &["list", "short", "sinks"]).is_some_and(|t| t.lines().any(|l| l.split_whitespace().nth(1) == Some(SINK_NAME)))
}

fn default_sink() -> String {
    run_quiet("pactl", &["get-default-sink"]).map(|s| s.trim().to_string()).unwrap_or_default()
}

fn unit_installed() -> bool {
    std::path::Path::new("/usr/lib/systemd/user/zohara-equalizer.service").exists()
}

/// The real output to point the equalizer at: the current default, unless that is the equalizer itself.
pub fn current_real_output(state: &State) -> String {
    let d = default_sink();
    if !d.is_empty() && d != SINK_NAME {
        d
    } else {
        state.target.clone()
    }
}

/// Writes the config and starts the equalizer, then makes it the default output.
pub fn start(state: &mut State) -> Result<(), String> {
    launch(state)?;
    activate();
    if is_running() {
        Ok(())
    } else {
        Err("The equalizer didn't start. Is the sound system running?".into())
    }
}

/// Starts the equalizer without changing the default output (`start` = this + `activate`).
pub fn launch(state: &mut State) -> Result<(), String> {
    if state.target.is_empty() || state.target == SINK_NAME {
        state.target = current_real_output(state);
    }
    if !safe_target(&state.target) {
        return Err("No output device to equalize was found.".into());
    }
    let conf = conf_path();
    if let Some(dir) = conf.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&conf, filter_chain_conf(&state.settings, &state.target)).map_err(|e| e.to_string())?;
    state.enabled = true;
    save_state(state);
    if unit_installed() {
        let ok = Command::new("systemctl").args(["--user", "enable", "--now", "zohara-equalizer.service"]).status().map(|s| s.success()).unwrap_or(false);
        if !ok {
            return Err("The equalizer service couldn't be started.".into());
        }
    } else if !is_running() {
        // No service file (an older install): run it from here, detached.
        Command::new("setsid")
            .args(["pipewire", "-c"])
            .arg(&conf)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Couldn't start the equalizer ({e})"))?;
    }
    for _ in 0..40 {
        if is_running() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    Err("The equalizer didn't start. Is the sound system running?".into())
}

/// Waits for the equalizer's output to appear (up to ~8 s) and makes it the default.
pub fn activate() {
    for _ in 0..40 {
        if is_running() {
            let _ = Command::new("pactl").args(["set-default-sink", SINK_NAME]).status();
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

/// Gives the default output back to the real device (so sound never ends up on a device that is going away).
pub fn deactivate() {
    let state = load_state();
    if safe_target(&state.target) && default_sink() == SINK_NAME {
        let _ = Command::new("pactl").args(["set-default-sink", &state.target]).status();
    }
}

pub fn stop(state: &mut State) {
    deactivate();
    state.enabled = false;
    save_state(state);
    if unit_installed() {
        let _ = Command::new("systemctl").args(["--user", "disable", "--now", "zohara-equalizer.service"]).status();
    }
    // Also covers the detached fallback.
    let _ = Command::new("pkill").arg("-f").arg(conf_path().to_string_lossy().to_string()).status();
}

/// The equalizer's node in the sound system, which takes the live control changes.
pub fn find_node_id() -> Option<u32> {
    let dump = run_quiet("pw-dump", &[])?;
    let all: Vec<serde_json::Value> = serde_json::from_str(&dump).ok()?;
    all.iter()
        .find(|o| o["type"] == "PipeWire:Interface:Node" && o["info"]["props"]["node.name"] == SINK_NAME)
        .and_then(|o| o["id"].as_u64())
        .map(|i| i as u32)
}

/// `{ params = [ "name" value ... ] }`, floats always with a decimal point (the sound system types them by that).
pub fn params_text(controls: &[(String, f64)]) -> String {
    let body: Vec<String> = controls.iter().map(|(k, v)| format!("\"{k}\" {v:.4}")).collect();
    format!("{{ params = [ {} ] }}", body.join(" "))
}

/// Applies the settings to the running equalizer at once. False when it isn't there (or the id has changed).
pub fn apply_live(node: u32, settings: &Settings) -> bool {
    Command::new("pw-cli")
        .args(["set-param", &node.to_string(), "Props", &params_text(&settings.controls())])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Sends the equalizer's sound to another real output while it runs, and remembers it for next time.
pub fn retarget(state: &mut State, sink: &str) -> Result<(), String> {
    if !safe_target(sink) || sink == SINK_NAME {
        return Err("That isn't an output the equalizer can use.".into());
    }
    state.target = sink.to_string();
    save_state(state);
    let conf = conf_path();
    if conf.exists() {
        let _ = std::fs::write(&conf, filter_chain_conf(&state.settings, sink));
    }
    // The stream that carries the result is an ordinary playback stream: move it.
    let inputs = run_quiet("pactl", &["-f", "json", "list", "sink-inputs"]).unwrap_or_default();
    let list: Vec<serde_json::Value> = serde_json::from_str(&inputs).unwrap_or_default();
    for i in list {
        if i["properties"]["node.name"] == OUT_NAME {
            if let Some(idx) = i["index"].as_u64() {
                let _ = Command::new("pactl").args(["move-sink-input", &idx.to_string(), sink]).status();
            }
        }
    }
    Ok(())
}

/// For the output lists on the Sound page: the equalizer's own output is not a device to choose.
pub fn is_equalizer_device(name: &str) -> bool {
    name == SINK_NAME || name == format!("{SINK_NAME}.monitor")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_peaking_band_reaches_its_gain_at_its_own_frequency_and_leaves_far_frequencies_alone() {
        let mut s = Settings { auto_headroom: false, ..Settings::default() };
        s.gains[5] = 12.0; // 1 kHz
        assert!((response_db(&s, 1000.0) - 12.0).abs() < 0.05, "{}", response_db(&s, 1000.0));
        assert!(response_db(&s, 100.0).abs() < 0.6);
        assert!(response_db(&s, 15000.0).abs() < 0.6);
        s.gains[5] = -6.0;
        assert!((response_db(&s, 1000.0) + 6.0).abs() < 0.05);
    }

    #[test]
    fn shelves_lift_one_end_and_leave_the_other() {
        let s = Settings { bass: 6.0, auto_headroom: false, ..Settings::default() };
        assert!(response_db(&s, 40.0) > 5.0);
        assert!(response_db(&s, 5000.0).abs() < 0.5);
        let t = Settings { treble: -6.0, auto_headroom: false, ..Settings::default() };
        assert!(response_db(&t, 15000.0) < -5.0);
        assert!(response_db(&t, 200.0).abs() < 0.5);
    }

    #[test]
    fn flat_is_flat() {
        let s = Settings::default();
        for f in [20.0, 100.0, 1000.0, 10000.0, 20000.0] {
            assert!(response_db(&s, f).abs() < 1e-6, "{f}");
        }
    }

    #[test]
    fn automatic_headroom_lowers_the_preamp_by_the_biggest_boost_so_nothing_clips() {
        let mut s = Settings::default();
        s.gains[1] = 6.0;
        s.treble = 9.0;
        assert_eq!(s.effective_preamp(), -9.0);
        s.auto_headroom = false;
        assert_eq!(s.effective_preamp(), 0.0);
        let cuts = Settings { gains: [-6.0; 10], ..Settings::default() };
        assert_eq!(cuts.effective_preamp(), 0.0, "cuts need no headroom");
        s.auto_headroom = true;
        s.preamp = -20.0;
        assert_eq!(s.effective_preamp(), PREAMP_MIN, "never below the limit");
    }

    #[test]
    fn built_in_presets_are_sane_and_unique() {
        let all = builtin_presets();
        assert!(all.len() >= 20);
        let mut names: Vec<_> = all.iter().map(|p| p.name.clone()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), all.len(), "names are unique");
        assert_eq!(all[0].name, "Flat");
        assert_eq!(all[0].settings, Settings::default());
        for p in &all {
            assert!(p.settings.gains.iter().all(|g| g.abs() <= GAIN_LIMIT), "{}", p.name);
            assert_eq!(p.settings.clone().clamped(), p.settings, "{}", p.name);
        }
    }

    #[test]
    fn own_presets_can_be_saved_replaced_and_deleted_but_not_over_a_built_in_one() {
        let mut st = State::default();
        st.settings.gains[0] = 4.0;
        assert!(st.save_custom("  ").is_err());
        assert!(st.save_custom("Rock").is_err(), "built-in name");
        assert!(st.save_custom(CUSTOM).is_err());
        st.save_custom("My bass").unwrap();
        assert_eq!(st.preset, "My bass");
        st.settings.gains[0] = 5.0;
        st.save_custom("My bass").unwrap();
        assert_eq!(st.custom.len(), 1, "replaced, not added");
        assert_eq!(st.find_preset("My bass").unwrap().gains[0], 5.0);
        let names = st.preset_names();
        assert_eq!(names.last().map(String::as_str), Some(CUSTOM));
        assert!(names.contains(&"My bass".to_string()));
        assert!(st.delete_custom("My bass"));
        assert_eq!(st.preset, CUSTOM);
        assert!(!st.delete_custom("My bass"));
    }

    #[test]
    fn state_survives_a_round_trip_and_bad_values_are_clamped() {
        let mut st = State::default();
        st.enabled = true;
        st.target = "alsa_output.x".into();
        st.settings.gains[3] = 99.0;
        st.settings.q = 100.0;
        let text = serde_json::to_string(&st).unwrap();
        let back: State = serde_json::from_str(&text).unwrap();
        assert_eq!(back, st);
        let c = back.settings.clamped();
        assert_eq!(c.gains[3], GAIN_LIMIT);
        assert_eq!(c.q, 6.0);
    }

    #[test]
    fn presets_can_be_shared_as_files() {
        let mut s = Settings::default();
        s.gains[9] = 3.0;
        let text = export_preset("Bright", &s);
        let p = import_preset(&text).unwrap();
        assert_eq!(p.name, "Bright");
        assert_eq!(p.settings.gains[9], 3.0);
        assert!(import_preset("nonsense").is_err());
        let wild = r#"{"name":"","settings":{"preamp":0,"gains":[50,0,0,0,0,0,0,0,0,0],"bass":0,"treble":0,"q":1.4,"auto_headroom":true}}"#;
        let p = import_preset(wild).unwrap();
        assert_eq!(p.name, "Imported");
        assert_eq!(p.settings.gains[0], GAIN_LIMIT);
    }

    #[test]
    fn the_config_has_every_node_in_one_chain_and_the_target() {
        let mut s = Settings::default();
        s.gains[5] = 3.0;
        let conf = filter_chain_conf(&s, "alsa_output.pci-0000_00_1f.3.analog-stereo");
        for n in ["pre", "eq1", "eq5", "eq10", "bass", "treble"] {
            assert!(conf.contains(&format!("name = {n} ")), "{n}");
        }
        assert_eq!(conf.matches("label = bq_peaking").count(), 10);
        assert!(conf.contains("{ output = \"eq10:Out\" input = \"bass:In\" }"));
        assert!(conf.contains("outputs = [ \"treble:Out\" ]"));
        assert!(conf.contains("target.object = \"alsa_output.pci-0000_00_1f.3.analog-stereo\""));
        assert!(conf.contains("node.name = \"zohara_eq\" media.class = Audio/Sink"));
        // A target with something odd in it is left out rather than written into the file.
        assert!(!filter_chain_conf(&s, "x\" } evil").contains("evil"));
    }

    #[test]
    fn live_parameters_are_named_like_the_graph_and_always_have_a_decimal_point() {
        let mut s = Settings::default();
        s.gains[0] = 6.0;
        let text = params_text(&s.controls());
        assert!(text.starts_with("{ params = [ \"pre:Mult\" "));
        assert!(text.contains("\"eq1:Gain\" 6.0000"));
        assert!(text.contains("\"eq10:Q\" 1.4100"));
        assert!(text.contains("\"bass:Gain\" 0.0000") && text.contains("\"treble:Gain\" 0.0000"));
        assert!(text.ends_with(" ] }"));
        assert_eq!(s.controls().len(), 1 + 10 * 2 + 2);
    }

    #[test]
    fn the_equalizers_own_output_is_not_a_device_to_pick() {
        assert!(is_equalizer_device("zohara_eq"));
        assert!(is_equalizer_device("zohara_eq.monitor"));
        assert!(!is_equalizer_device("alsa_output.pci-0000_00_1f.3.analog-stereo"));
    }

    // ── Against the real sound system ──────────────────────────────────────

    fn wav_tone(freq: f64, secs: u32) -> Vec<u8> {
        let rate = 48000u32;
        let n = rate * secs;
        let mut data = Vec::new();
        for i in 0..n {
            let v = (0.1 * 32767.0 * (2.0 * std::f64::consts::PI * freq * i as f64 / rate as f64).sin()) as i16;
            data.extend_from_slice(&v.to_le_bytes());
            data.extend_from_slice(&v.to_le_bytes());
        }
        let mut w = Vec::new();
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        w.extend_from_slice(b"WAVEfmt ");
        w.extend_from_slice(&16u32.to_le_bytes());
        w.extend_from_slice(&1u16.to_le_bytes());
        w.extend_from_slice(&2u16.to_le_bytes());
        w.extend_from_slice(&rate.to_le_bytes());
        w.extend_from_slice(&(rate * 4).to_le_bytes());
        w.extend_from_slice(&4u16.to_le_bytes());
        w.extend_from_slice(&16u16.to_le_bytes());
        w.extend_from_slice(b"data");
        w.extend_from_slice(&(data.len() as u32).to_le_bytes());
        w.extend_from_slice(&data);
        w
    }

    /// RMS of the left and right channel of a 16-bit stereo WAV, skipping the start and end.
    fn wav_rms(bytes: &[u8]) -> (f64, f64) {
        let pcm = &bytes[44.min(bytes.len())..];
        let frames = pcm.len() / 4;
        let (from, to) = (frames * 30 / 100, frames * 90 / 100);
        let (mut l, mut r, mut n) = (0f64, 0f64, 0f64);
        for f in from..to {
            let a = i16::from_le_bytes([pcm[f * 4], pcm[f * 4 + 1]]) as f64;
            let b = i16::from_le_bytes([pcm[f * 4 + 2], pcm[f * 4 + 3]]) as f64;
            l += a * a;
            r += b * b;
            n += 1.0;
        }
        ((l / n).sqrt(), (r / n).sqrt())
    }

    /// Plays `tone` into the equalizer for a few seconds and records what arrives at `monitor`.
    fn record_through(tone: &std::path::Path, monitor: &str, out: &std::path::Path, live: Option<(u32, Settings)>) -> (f64, f64) {
        let mut rec = Command::new("parecord")
            .args([&format!("--device={monitor}"), "--file-format=wav", "--rate=48000", "--channels=2"])
            .arg(out)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(500));
        let mut play = Command::new("paplay").arg(format!("--device={SINK_NAME}")).arg(tone).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
        if let Some((node, settings)) = live {
            std::thread::sleep(std::time::Duration::from_millis(1200));
            assert!(apply_live(node, &settings), "live change refused");
        }
        let _ = play.wait();
        std::thread::sleep(std::time::Duration::from_millis(300));
        let _ = rec.kill();
        let _ = rec.wait();
        wav_rms(&std::fs::read(out).unwrap())
    }

    /// Needs a running PipeWire session. It uses a silent HDMI output of this computer and never touches the default
    /// output. `cargo test live_equalizer -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_equalizer_shapes_the_sound_changes_live_and_moves_to_another_output() {
        let sinks = run_quiet("pactl", &["list", "short", "sinks"]).unwrap();
        let hdmi: Vec<String> = sinks.lines().filter_map(|l| l.split_whitespace().nth(1)).filter(|n| n.contains("HDMI")).map(String::from).collect();
        assert!(hdmi.len() >= 2, "needs two HDMI outputs to test moving: {hdmi:?}");
        let dir = std::env::temp_dir().join(format!("zs-eq-live-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("XDG_CONFIG_HOME", &dir);
        let default_before = default_sink();

        let (t1k, t100) = (dir.join("t1k.wav"), dir.join("t100.wav"));
        std::fs::write(&t1k, wav_tone(1000.0, 4)).unwrap();
        std::fs::write(&t100, wav_tone(100.0, 4)).unwrap();

        let mut st = State { target: hdmi[0].clone(), ..State::default() };
        st.settings.auto_headroom = false;
        st.settings.gains[5] = 12.0; // 1 kHz
        launch(&mut st).expect("starts");
        assert!(is_running());
        assert_eq!(default_sink(), default_before, "the default output was not touched");
        let monitor = format!("{}.monitor", hdmi[0]);

        // Reference level: the same tone straight to the output, no equalizer.
        let straight = {
            let mut rec = Command::new("parecord").args([&format!("--device={monitor}"), "--file-format=wav", "--rate=48000", "--channels=2"]).arg(dir.join("ref.wav")).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(500));
            let _ = Command::new("paplay").arg(format!("--device={}", hdmi[0])).arg(&t1k).status();
            std::thread::sleep(std::time::Duration::from_millis(300));
            let _ = rec.kill();
            let _ = rec.wait();
            wav_rms(&std::fs::read(dir.join("ref.wav")).unwrap())
        };

        let boosted = record_through(&t1k, &monitor, &dir.join("a.wav"), None);
        let gain_l = 20.0 * (boosted.0 / straight.0).log10();
        let gain_r = 20.0 * (boosted.1 / straight.1).log10();
        let far = record_through(&t100, &monitor, &dir.join("b.wav"), None);
        let far_gain = 20.0 * (far.0 / straight.0).log10();

        // Change the 1 kHz band live while the tone plays: the tail of the recording must be back at the reference.
        let node = find_node_id().expect("node");
        let mut flat = st.settings.clone();
        flat.gains[5] = 0.0;
        let _ = record_through(&t1k, &monitor, &dir.join("c.wav"), Some((node, flat)));
        let c = std::fs::read(dir.join("c.wav")).unwrap();
        let pcm = &c[44..];
        let tail = &pcm[(pcm.len() * 80 / 100) / 4 * 4..]; // whole stereo samples
        let mut sum = 0f64;
        let mut cnt = 0f64;
        for f in 0..tail.len() / 4 {
            let a = i16::from_le_bytes([tail[f * 4], tail[f * 4 + 1]]) as f64;
            sum += a * a;
            cnt += 1.0;
        }
        let tail_gain = 20.0 * ((sum / cnt).sqrt() / straight.0).log10();
        {
            // Level over time, for the log.
            let mut line = String::new();
            let step = 48000 * 4 / 2; // 0.5 s of stereo 16-bit
            for (i, chunk) in pcm.chunks(step).enumerate() {
                let n = chunk.len() / 4;
                if n == 0 { continue; }
                let mut e = 0f64;
                for f in 0..n { let a = i16::from_le_bytes([chunk[f * 4], chunk[f * 4 + 1]]) as f64; e += a * a; }
                line += &format!(" {:.1}", 20.0 * ((e / n as f64).sqrt().max(1.0) / straight.0).log10());
                let _ = i;
            }
            println!("level over time (dB vs reference, 0.5 s steps):{line}");
        }

        // Move the equalizer's output to another HDMI output while it runs.
        retarget(&mut st, &hdmi[1]).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(800));
        let inputs = run_quiet("pactl", &["-f", "json", "list", "sink-inputs"]).unwrap();
        let list: Vec<serde_json::Value> = serde_json::from_str(&inputs).unwrap();
        let out_stream = list.iter().find(|i| i["properties"]["node.name"] == OUT_NAME).cloned();
        let sinks_json: Vec<serde_json::Value> = serde_json::from_str(&run_quiet("pactl", &["-f", "json", "list", "sinks"]).unwrap()).unwrap();
        let moved_to = out_stream.as_ref().and_then(|o| sinks_json.iter().find(|s| s["index"] == o["sink"])).and_then(|s| s["name"].as_str()).map(String::from);

        stop(&mut st);
        let _ = std::fs::remove_dir_all(&dir);
        std::thread::sleep(std::time::Duration::from_millis(500));

        println!("1 kHz +12 dB: left {gain_l:.2} dB, right {gain_r:.2} dB | 100 Hz: {far_gain:.2} dB | after live change to 0 dB: {tail_gain:.2} dB | moved to {moved_to:?}");
        assert!((gain_l - 12.0).abs() < 0.6 && (gain_r - 12.0).abs() < 0.6, "left {gain_l} right {gain_r}");
        assert!(far_gain.abs() < 0.8, "{far_gain}");
        assert!(tail_gain.abs() < 0.6, "live change did not settle: {tail_gain}");
        assert_eq!(moved_to.as_deref(), Some(hdmi[1].as_str()));
        assert!(!is_running(), "stopped");
        assert_eq!(default_sink(), default_before, "the default output was never touched");
    }
}
