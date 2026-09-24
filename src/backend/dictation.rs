//! Voice typing, like Windows' Win+H: press the shortcut, speak, and the words
//! are typed where the cursor is.
//!
//! Runs as `zohara-settings --dictate`, which the Meta+H global shortcut starts
//! (`zohara-dictation.desktop`). Pressing the shortcut again while listening
//! finishes early. Everything happens on this computer: audio is recorded with
//! `parecord`, transcribed by whisper.cpp using the model shipped in the ISO,
//! then pasted into the focused window (wl-copy, then Ctrl+V from ydotool).

use crate::backend::kconfig;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const MODEL: &str = "/usr/share/zohara/dictation/ggml-base-q8_0.bin";
const WHISPER_DIR: &str = "/usr/lib/zohara/whisper";
pub const CONFIG: &str = "zoharadictationrc";
const DESKTOP_ID: &str = "zohara-dictation.desktop";
const DEFAULT_SHORTCUT: &str = "Meta+H";

const RATE: usize = 16_000;
/// 30 ms of audio.
const FRAME: usize = RATE * 30 / 1000;
const MAX_LENGTH: Duration = Duration::from_secs(60);
const START_TIMEOUT: Duration = Duration::from_secs(8);
const END_SILENCE: Duration = Duration::from_millis(1500);

/// (label, whisper language code)
pub const LANGUAGES: &[(&str, &str)] = &[
    ("Detect automatically", "auto"),
    ("English", "en"),
    ("Hindi", "hi"),
    ("Urdu", "ur"),
    ("Arabic", "ar"),
    ("Bengali", "bn"),
    ("Punjabi", "pa"),
    ("Spanish", "es"),
    ("French", "fr"),
    ("German", "de"),
    ("Italian", "it"),
    ("Portuguese", "pt"),
    ("Russian", "ru"),
    ("Turkish", "tr"),
    ("Indonesian", "id"),
    ("Chinese", "zh"),
    ("Japanese", "ja"),
    ("Korean", "ko"),
];

pub struct Settings {
    pub enabled: bool,
    pub language: String,
    pub auto_stop: bool,
}

pub fn settings() -> Settings {
    let get = |k: &str| kconfig::read(CONFIG, &["General"], k);
    Settings {
        enabled: get("Enabled").as_deref() != Some("false"),
        language: get("Language").unwrap_or_else(|| "auto".into()),
        auto_stop: get("AutoStop").as_deref() != Some("false"),
    }
}

pub fn set(key: &str, value: &str) {
    kconfig::write(CONFIG, &["General"], key, value);
}

/// The shortcut kglobalaccel has for voice typing, as Plasma writes it.
pub fn shortcut_text() -> String {
    match kconfig::read("kglobalshortcutsrc", &["services", DESKTOP_ID], "_launch") {
        None => DEFAULT_SHORTCUT.into(),
        Some(v) => {
            let first = v.split('\t').next().unwrap_or("").trim();
            if first.is_empty() || first == "none" {
                "None".into()
            } else {
                first.into()
            }
        }
    }
}

fn which(cmd: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(cmd).is_file()))
        .unwrap_or(false)
}

/// whisper.cpp is built twice: for CPUs with AVX2 and a baseline for older ones.
fn whisper_binary() -> Option<PathBuf> {
    let flags = std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|c| c.lines().find(|l| l.starts_with("flags")).map(str::to_string))
        .unwrap_or_default();
    let has = |f: &str| flags.split_whitespace().any(|x| x == f);
    let fast = Path::new(WHISPER_DIR).join("zohara-whisper-avx2");
    let base = Path::new(WHISPER_DIR).join("zohara-whisper");
    if ["avx", "avx2", "bmi2", "fma", "f16c"].iter().all(|f| has(*f)) && fast.exists() {
        return Some(fast);
    }
    base.exists().then_some(base)
}

/// Why voice typing can't run on this system, if it can't.
pub fn missing() -> Option<String> {
    if whisper_binary().is_none() || !Path::new(MODEL).exists() {
        return Some("The speech recognition engine is not installed".into());
    }
    if !which("parecord") {
        return Some("The audio recording tools (libpulse) are not installed".into());
    }
    None
}

/// Whether text can be typed into other apps. Otherwise it's left on the clipboard.
pub fn can_type() -> bool {
    which("wl-copy")
        && which("ydotool")
        && Command::new("systemctl")
            .args(["--user", "is-active", "--quiet", "ydotool.service"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
}

// ── Notifications ──────────────────────────────────────────────────────────

#[derive(Default)]
struct Note {
    id: Option<String>,
}

impl Note {
    /// Shows or replaces the notification. `timeout_ms` 0 keeps it up until closed.
    fn show(&mut self, title: &str, body: &str, timeout_ms: u32) {
        let mut c = Command::new("notify-send");
        c.args(["-a", "Voice typing", "-i", "audio-input-microphone", "-p", "-h", "boolean:transient:true"])
            .args(["-t", &timeout_ms.to_string()]);
        if let Some(id) = &self.id {
            c.args(["-r", id.as_str()]);
        }
        c.args([title, body]);
        match c.output() {
            Ok(o) => {
                let id = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if !id.is_empty() {
                    self.id = Some(id);
                }
            }
            Err(e) => log::warn!("dictation: notify-send failed: {e}"),
        }
    }

    fn close(&mut self) {
        if let Some(id) = self.id.take() {
            let _ = Command::new("gdbus")
                .args([
                    "call", "--session",
                    "--dest", "org.freedesktop.Notifications",
                    "--object-path", "/org/freedesktop/Notifications",
                    "--method", "org.freedesktop.Notifications.CloseNotification",
                    id.as_str(),
                ])
                .output();
        }
    }
}

// ── Recording ──────────────────────────────────────────────────────────────

#[derive(PartialEq)]
enum Stop {
    /// The speaker paused.
    Silence,
    /// The shortcut was pressed again.
    User,
    /// Hit the length limit.
    Limit,
    /// Nobody spoke.
    NothingHeard,
}

struct Recording {
    samples: Vec<i16>,
    stop: Stop,
    heard: bool,
}

fn level(frame: &[i16]) -> f64 {
    (frame.iter().map(|&s| (s as f64) * (s as f64)).sum::<f64>() / frame.len() as f64).sqrt()
}

fn record(dir: &Path, auto_stop: bool) -> Result<Recording, String> {
    let mut child = Command::new("parecord")
        .args([
            "--raw", "--rate=16000", "--channels=1", "--format=s16le", "--latency-msec=30",
            "--client-name=Zohara voice typing", "--stream-name=Voice typing",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Couldn't start recording: {e}"))?;
    let Some(mut out) = child.stdout.take() else {
        let _ = child.kill();
        return Err("Couldn't read from the microphone".into());
    };

    let stop_file = dir.join("stop");
    let started = Instant::now();
    let mut samples: Vec<i16> = Vec::with_capacity(RATE * 10);
    let mut buf = vec![0u8; FRAME * 2];
    // Background noise level: follows quiet stretches quickly and loud ones
    // slowly, so speech stands out from a fan or hum.
    let mut floor: Option<f64> = None;
    let mut last_speech: Option<Instant> = None;

    let stop = loop {
        if let Err(e) = out.read_exact(&mut buf) {
            let _ = child.kill();
            let _ = child.wait();
            if samples.is_empty() {
                return Err(format!("The microphone stopped: {e}"));
            }
            break Stop::User;
        }
        let frame: Vec<i16> = buf.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
        let lvl = level(&frame);
        samples.extend_from_slice(&frame);

        let f = floor.get_or_insert(lvl);
        *f = if lvl < *f { lvl } else { *f * 0.995 + lvl * 0.005 };
        if lvl > (*f * 3.0).max(300.0) {
            last_speech = Some(Instant::now());
        }

        if stop_file.exists() {
            break Stop::User;
        }
        let elapsed = started.elapsed();
        if elapsed >= MAX_LENGTH {
            break Stop::Limit;
        }
        if auto_stop {
            match last_speech {
                Some(t) if t.elapsed() >= END_SILENCE => break Stop::Silence,
                None if elapsed >= START_TIMEOUT => break Stop::NothingHeard,
                _ => {}
            }
        }
    };
    let _ = child.kill();
    let _ = child.wait();
    Ok(Recording { samples, stop, heard: last_speech.is_some() })
}

fn write_wav(path: &Path, s: &[i16]) -> std::io::Result<()> {
    let data_len = (s.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + data_len as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&1u16.to_le_bytes()); // mono
    b.extend_from_slice(&(RATE as u32).to_le_bytes());
    b.extend_from_slice(&(RATE as u32 * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    for x in s {
        b.extend_from_slice(&x.to_le_bytes());
    }
    std::fs::write(path, b)
}

// ── Recognition ────────────────────────────────────────────────────────────

/// Joins whisper's output lines and drops its non-speech markers such as
/// `[BLANK_AUDIO]` or `(music)`.
fn clean(raw: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for ch in raw.chars() {
        match ch {
            '[' | '(' => depth += 1,
            ']' | ')' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn transcribe(wav: &Path, language: &str) -> Result<String, String> {
    let bin = whisper_binary().ok_or("The speech recognition engine is not installed")?;
    let threads = std::thread::available_parallelism().map(|n| n.get().min(8)).unwrap_or(4).to_string();
    let o = Command::new(&bin)
        .args(["-m", MODEL, "-l", language, "-t", &threads, "-nt", "-np", "-f"])
        .arg(wav)
        .output()
        .map_err(|e| format!("Couldn't start {}: {e}", bin.display()))?;
    if !o.status.success() {
        let err = String::from_utf8_lossy(&o.stderr);
        return Err(err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("Speech recognition failed").to_string());
    }
    Ok(clean(&String::from_utf8_lossy(&o.stdout)))
}

// ── Typing ─────────────────────────────────────────────────────────────────

enum Typed {
    Pasted,
    Copied,
    Failed,
}

fn wl_copy(text: &str) -> bool {
    let Ok(mut c) = Command::new("wl-copy")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    if let Some(mut stdin) = c.stdin.take() {
        let _ = stdin.write_all(text.as_bytes());
    }
    c.wait().map(|s| s.success()).unwrap_or(false)
}

/// The clipboard's contents if they are plain text, so they can be put back after pasting.
fn clipboard_text() -> Option<String> {
    let types = Command::new("wl-paste").arg("--list-types").output().ok()?;
    let types = String::from_utf8_lossy(&types.stdout);
    if !types.lines().any(|t| t.starts_with("text/plain")) || types.lines().any(|t| t.starts_with("image/")) {
        return None;
    }
    let o = Command::new("wl-paste").args(["--no-newline", "--type", "text/plain"]).output().ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).into_owned())
}

fn insert(text: &str) -> Typed {
    if !which("wl-copy") {
        return Typed::Failed;
    }
    let typing = can_type();
    let previous = if typing { clipboard_text() } else { None };
    if !wl_copy(text) {
        return Typed::Failed;
    }
    if !typing {
        return Typed::Copied;
    }
    std::thread::sleep(Duration::from_millis(150));
    // Left Ctrl (29) + V (47), as evdev key codes.
    let pasted = Command::new("ydotool")
        .args(["key", "29:1", "47:1", "47:0", "29:0"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !pasted {
        return Typed::Copied;
    }
    if let Some(p) = previous {
        std::thread::sleep(Duration::from_millis(500));
        wl_copy(&p);
    }
    Typed::Pasted
}

// ── Entry point ────────────────────────────────────────────────────────────

fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("zohara-dictation")
}

/// The pid of a voice typing session that is already listening.
fn running_session(dir: &Path) -> Option<u32> {
    let pid: u32 = std::fs::read_to_string(dir.join("pid")).ok()?.trim().parse().ok()?;
    let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    (pid != std::process::id() && String::from_utf8_lossy(&cmdline).contains("--dictate")).then_some(pid)
}

/// `zohara-settings --dictate`: listen, recognise, type. Returns the exit code.
pub fn run() -> i32 {
    let dir = runtime_dir();
    let _ = std::fs::create_dir_all(&dir);
    if running_session(&dir).is_some() {
        // Second press of the shortcut: tell the listening session to finish.
        let _ = std::fs::write(dir.join("stop"), b"");
        return 0;
    }

    let mut note = Note::default();
    let s = settings();
    if !s.enabled {
        note.show("Voice typing is off", "Turn it on in Settings › Accessibility › Voice typing.", 5000);
        return 0;
    }
    if let Some(why) = missing() {
        log::warn!("dictation: {why}");
        note.show("Voice typing isn't available", &why, 8000);
        return 1;
    }

    let _ = std::fs::remove_file(dir.join("stop"));
    let _ = std::fs::write(dir.join("pid"), std::process::id().to_string());
    let code = session(&dir, &s, &mut note);
    for f in ["pid", "stop", "speech.wav"] {
        let _ = std::fs::remove_file(dir.join(f));
    }
    code
}

fn session(dir: &Path, s: &Settings, note: &mut Note) -> i32 {
    let shortcut = shortcut_text();
    let hint = if s.auto_stop {
        format!("Speak now. It stops when you pause, or press {shortcut} to finish.")
    } else {
        format!("Speak now. Press {shortcut} again when you're done.")
    };
    note.show("Listening…", &hint, 0);

    let rec = match record(dir, s.auto_stop) {
        Ok(r) => r,
        Err(e) => {
            log::error!("dictation: {e}");
            note.show("Voice typing stopped", &e, 6000);
            return 1;
        }
    };
    if !rec.heard && rec.stop != Stop::User {
        note.show("Didn't hear anything", "Check that your microphone is on and not muted in Settings › Sound.", 6000);
        return 0;
    }
    if rec.stop == Stop::Limit {
        log::info!("dictation: stopped at the {}s limit", MAX_LENGTH.as_secs());
    }

    note.show("Writing it down…", "", 0);
    let mut audio = rec.samples;
    // whisper.cpp wants at least a second of audio.
    if audio.len() < RATE + RATE / 10 {
        audio.resize(RATE + RATE / 10, 0);
    }
    let wav = dir.join("speech.wav");
    if let Err(e) = write_wav(&wav, &audio) {
        log::error!("dictation: writing {}: {e}", wav.display());
        note.show("Voice typing stopped", &e.to_string(), 6000);
        return 1;
    }
    let started = Instant::now();
    let text = match transcribe(&wav, &s.language) {
        Ok(t) => t,
        Err(e) => {
            log::error!("dictation: recognition failed: {e}");
            note.show("Couldn't recognise speech", &e, 8000);
            return 1;
        }
    };
    // The words themselves are never logged.
    log::info!(
        "dictation: {:.1}s of audio recognised in {:.1}s ({} characters)",
        audio.len() as f64 / RATE as f64,
        started.elapsed().as_secs_f64(),
        text.chars().count()
    );
    if text.is_empty() {
        note.show("Didn't catch that", "Try again, a little closer to the microphone.", 5000);
        return 0;
    }

    note.close();
    match insert(&text) {
        Typed::Pasted => {}
        Typed::Copied => note.show("Copied to the clipboard", "Press Ctrl+V to paste what you said.", 6000),
        Typed::Failed => note.show("Couldn't type the text", &text, 15000),
    }
    0
}

#[cfg(test)]
mod tests {
    use super::clean;

    #[test]
    fn drops_markers() {
        assert_eq!(clean(" [BLANK_AUDIO]\n"), "");
        assert_eq!(clean(" Hello there.\n How are you? (music)\n"), "Hello there. How are you?");
    }
}
