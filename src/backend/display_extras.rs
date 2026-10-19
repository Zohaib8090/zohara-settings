//! The per-screen options beyond mode, scale and rotation: brightness, variable refresh rate (VRR), HDR and wide colour
//! gamut, colour profile, RGB range, colour accuracy and overscan, plus the two global switches that matter for games
//! (screen tearing, scaling of old X11 programs).
//!
//! `kscreen-doctor -j` has no flags for what a screen can do, so the capabilities come from `kscreen-doctor -o` (text):
//! `Vrr: incapable`, `HDR: incapable`, `Brightness control: supported, set to 100% ...`. A control is shown only when the
//! screen says it can do it. Values are changed with the same tool, one `output.NAME.setting.value` argument each.
//! Spellings were checked against `kscreen-doctor` 6.7 on 2026-10-18 (see docs/DISPLAYS.md).

use std::collections::HashMap;
use std::process::{Command, Stdio};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vrr {
    Never,
    Automatic,
    Always,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RgbRange {
    Automatic,
    Full,
    Limited,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorSource {
    Srgb,
    Edid,
    Icc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tradeoff {
    Accuracy,
    Efficiency,
}

/// What one screen reports. `None` means the screen cannot do it (or the tool did not say), so the control stays hidden.
#[derive(Clone, Debug, PartialEq)]
pub struct Extras {
    /// Percent, 0 to 100.
    pub brightness: Option<u32>,
    pub vrr: Option<Vrr>,
    pub hdr: Option<bool>,
    pub wide_gamut: Option<bool>,
    pub rgb_range: Option<RgbRange>,
    pub color_source: Option<ColorSource>,
    pub tradeoff: Option<Tradeoff>,
    pub overscan: u32,
    /// Brightness of normal content on an HDR screen, in nits (shown under the HDR line).
    pub sdr_brightness: Option<u32>,
}

impl Default for Extras {
    fn default() -> Self {
        Extras { brightness: None, vrr: None, hdr: None, wide_gamut: None, rgb_range: None, color_source: None, tradeoff: None, overscan: 0, sdr_brightness: None }
    }
}

/// The lowest brightness the slider reaches: a laptop panel at 0 % is a black screen with nothing to click.
pub const MIN_BRIGHTNESS: u32 = 5;

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            // ESC [ ... letter
            if chars.peek() == Some(&'[') {
                chars.next();
                for n in chars.by_ref() {
                    if n.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

fn capable(value: &str) -> bool {
    let v = value.trim().to_lowercase();
    !(v.is_empty() || v.contains("incapable") || v.contains("unsupported") || v.contains("not supported"))
}

fn parse_vrr(value: &str) -> Option<Vrr> {
    if !capable(value) {
        return None;
    }
    let v = value.trim().to_lowercase();
    Some(if v.contains("never") || v.contains("disabled") || v == "off" {
        Vrr::Never
    } else if v.contains("always") {
        Vrr::Always
    } else {
        Vrr::Automatic
    })
}

/// "enabled" / "disabled" after the capability is known; `incapable` hides the control.
fn parse_switch(value: &str) -> Option<bool> {
    if !capable(value) {
        return None;
    }
    let v = value.trim().to_lowercase();
    Some(!(v.contains("disabled") || v == "off" || v == "false"))
}

/// Reads `kscreen-doctor -o`: one block per output, headed `Output: <id> <name> <uuid>`.
pub fn parse_outputs_text(raw: &str) -> HashMap<String, Extras> {
    let mut map: HashMap<String, Extras> = HashMap::new();
    let mut current: Option<String> = None;
    for line in strip_ansi(raw).lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("Output:") {
            let name = rest.split_whitespace().nth(1).map(str::to_string);
            if let Some(n) = &name {
                map.insert(n.clone(), Extras::default());
            }
            current = name;
            continue;
        }
        let Some(e) = current.as_ref().and_then(|n| map.get_mut(n)) else { continue };
        let Some((key, value)) = t.split_once(':') else { continue };
        match key.trim() {
            "Vrr" => e.vrr = parse_vrr(value),
            "HDR" => e.hdr = parse_switch(value),
            "Wide Color Gamut" => e.wide_gamut = parse_switch(value),
            "SDR brightness" => e.sdr_brightness = value.trim().split(|c: char| !c.is_ascii_digit()).next().and_then(|n| n.parse().ok()),
            "Overscan" => e.overscan = value.trim().parse().unwrap_or(0),
            "RgbRange" => {
                e.rgb_range = match value.trim().to_lowercase().as_str() {
                    "full" => Some(RgbRange::Full),
                    "limited" => Some(RgbRange::Limited),
                    "automatic" | "auto" => Some(RgbRange::Automatic),
                    _ => None,
                }
            }
            "Color profile source" => {
                e.color_source = match value.trim().to_lowercase().as_str() {
                    "srgb" => Some(ColorSource::Srgb),
                    "edid" => Some(ColorSource::Edid),
                    "icc" => Some(ColorSource::Icc),
                    _ => None,
                }
            }
            "Color power preference" => {
                let v = value.to_lowercase();
                e.tradeoff = if v.contains("accuracy") {
                    Some(Tradeoff::Accuracy)
                } else if v.contains("efficiency") {
                    Some(Tradeoff::Efficiency)
                } else {
                    None
                }
            }
            "Brightness control" => {
                // "supported, set to 100% and dimming to 100%"
                e.brightness = if capable(value) && value.to_lowercase().contains("supported") {
                    value.split("set to").nth(1).and_then(|r| r.trim().split('%').next()).and_then(|n| n.trim().parse::<u32>().ok())
                } else {
                    None
                };
            }
            _ => {}
        }
    }
    map
}

/// Runs `kscreen-doctor -o` and reads it.
pub fn load() -> HashMap<String, Extras> {
    match Command::new("kscreen-doctor").arg("-o").stdin(Stdio::null()).output() {
        Ok(o) if o.status.success() => parse_outputs_text(&String::from_utf8_lossy(&o.stdout)),
        _ => HashMap::new(),
    }
}

// ── Commands ────────────────────────────────────────────────────────────────────────────────────────────────────

pub fn brightness_args(name: &str, percent: u32) -> Vec<String> {
    vec![format!("output.{name}.brightness.{}", percent.clamp(MIN_BRIGHTNESS, 100))]
}

pub fn vrr_args(name: &str, v: Vrr) -> Vec<String> {
    let w = match v {
        Vrr::Never => "never",
        Vrr::Automatic => "automatic",
        Vrr::Always => "always",
    };
    vec![format!("output.{name}.vrrpolicy.{w}")]
}

pub fn hdr_args(name: &str, on: bool) -> Vec<String> {
    vec![format!("output.{name}.hdr.{}", if on { "enable" } else { "disable" })]
}

pub fn wide_gamut_args(name: &str, on: bool) -> Vec<String> {
    vec![format!("output.{name}.wcg.{}", if on { "enable" } else { "disable" })]
}

pub fn rgb_range_args(name: &str, r: RgbRange) -> Vec<String> {
    let w = match r {
        RgbRange::Automatic => "automatic",
        RgbRange::Full => "full",
        RgbRange::Limited => "limited",
    };
    vec![format!("output.{name}.rgbrange.{w}")]
}

/// `icc` is the profile file, needed (and only used) for `ColorSource::Icc`.
pub fn color_source_args(name: &str, s: ColorSource, icc: Option<&str>) -> Vec<String> {
    match s {
        ColorSource::Srgb => vec![format!("output.{name}.colorProfileSource.sRGB")],
        ColorSource::Edid => vec![format!("output.{name}.colorProfileSource.EDID")],
        ColorSource::Icc => {
            let mut v = Vec::new();
            if let Some(p) = icc {
                v.push(format!("output.{name}.iccprofile.{p}"));
            }
            v.push(format!("output.{name}.colorProfileSource.ICC"));
            v
        }
    }
}

pub fn tradeoff_args(name: &str, t: Tradeoff) -> Vec<String> {
    let w = match t {
        Tradeoff::Accuracy => "preferAccuracy",
        Tradeoff::Efficiency => "preferEfficiency",
    };
    vec![format!("output.{name}.colorPowerTradeoff.{w}")]
}

pub fn overscan_args(name: &str, percent: u32) -> Vec<String> {
    vec![format!("output.{name}.overscan.{}", percent.min(100))]
}

// ── Game switches (kwinrc) ──────────────────────────────────────────────────────────────────────────────────────

/// Screen tearing in fullscreen windows: lower delay for games, at the price of a torn picture now and then. KWin default: on.
pub fn tearing_allowed() -> bool {
    kread("Compositing", "AllowTearing").map(|v| v != "false").unwrap_or(true)
}

/// Old X11 programs scale themselves (sharp, but some ignore the scale) or the system scales them (works everywhere, a little
/// blurry). KWin default: they scale themselves.
pub fn x11_scale_themselves() -> bool {
    kread("Xwayland", "XwaylandClientsScale").map(|v| v != "false").unwrap_or(true)
}

fn kread(group: &str, key: &str) -> Option<String> {
    let out = Command::new("kreadconfig6").args(["--file", "kwinrc", "--group", group, "--key", key]).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// `kwriteconfig6` arguments for a boolean in kwinrc (`--notify` so KWin hears about it).
pub fn kwin_bool_args(group: &str, key: &str, value: bool) -> Vec<String> {
    ["--file", "kwinrc", "--group", group, "--key", key, "--notify", if value { "true" } else { "false" }]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

pub fn set_tearing(allowed: bool) -> bool {
    write_kwin("Compositing", "AllowTearing", allowed)
}

pub fn set_x11_scale_themselves(yes: bool) -> bool {
    write_kwin("Xwayland", "XwaylandClientsScale", yes)
}

fn write_kwin(group: &str, key: &str, value: bool) -> bool {
    let ok = Command::new("kwriteconfig6").args(kwin_bool_args(group, key, value)).status().map(|s| s.success()).unwrap_or(false);
    // KWin re-reads its settings on this call; Xwayland scaling still needs a new session to take effect.
    let _ = Command::new("qdbus6").args(["org.kde.KWin", "/KWin", "reconfigure"]).stdout(Stdio::null()).stderr(Stdio::null()).status();
    ok
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real output of `kscreen-doctor -o` on the owner's laptop (colour codes included, one output trimmed).
    const LAPTOP: &str = "\u{1b}[01;32mOutput: \u{1b}[0;32m1 eDP-1 fe002a57-5f1a-424f-a3f7-ae95df53f10b\n\
\tenabled\n\tconnected\n\tpriority 1\n\tPanel\n\treplication source:0\n\tGeometry: 0,0 1536x960\n\tScale: 1.25\n\tRotation: 1\n\
\tOverscan: 0\n\tVrr: incapable\n\tRgbRange: Full\n\tHDR: incapable\n\tWide Color Gamut: incapable\n\tICC profile: none\n\
\tColor profile source: sRGB\n\tColor power preference: prefer accuracy\n\
\tBrightness control: supported, set to 80% and dimming to 100%\n\tColor resolution: 10 bits per color, range: [6; 12] bits per color\n";

    #[test]
    fn laptop_panel_has_brightness_and_colour_but_no_hdr_or_vrr() {
        let m = parse_outputs_text(LAPTOP);
        let e = &m["eDP-1"];
        assert_eq!(e.brightness, Some(80));
        assert_eq!(e.vrr, None);
        assert_eq!(e.hdr, None);
        assert_eq!(e.wide_gamut, None);
        assert_eq!(e.rgb_range, Some(RgbRange::Full));
        assert_eq!(e.color_source, Some(ColorSource::Srgb));
        assert_eq!(e.tradeoff, Some(Tradeoff::Accuracy));
        assert_eq!(e.overscan, 0);
    }

    #[test]
    fn capable_screens_show_their_state() {
        let raw = "Output: 2 DP-2 abc\n\tVrr: Automatic\n\tHDR: enabled\n\t\tSDR brightness: 350\n\tWide Color Gamut: disabled\n\tRgbRange: Limited\n\
\tColor profile source: EDID\n\tColor power preference: prefer efficiency\n\tOverscan: 3\n\tBrightness control: unsupported\n\
Output: 3 HDMI-A-1 def\n\tVrr: Always\n\tHDR: disabled\n";
        let m = parse_outputs_text(raw);
        let a = &m["DP-2"];
        assert_eq!(a.vrr, Some(Vrr::Automatic));
        assert_eq!(a.hdr, Some(true));
        assert_eq!(a.wide_gamut, Some(false));
        assert_eq!(a.rgb_range, Some(RgbRange::Limited));
        assert_eq!(a.color_source, Some(ColorSource::Edid));
        assert_eq!(a.tradeoff, Some(Tradeoff::Efficiency));
        assert_eq!(a.overscan, 3);
        assert_eq!(a.sdr_brightness, Some(350));
        assert_eq!(a.brightness, None, "unsupported brightness hides the slider");
        let b = &m["HDMI-A-1"];
        assert_eq!(b.vrr, Some(Vrr::Always));
        assert_eq!(b.hdr, Some(false));
    }

    #[test]
    fn unknown_or_missing_lines_do_not_invent_controls() {
        let m = parse_outputs_text("Output: 1 X-1 u\n\tenabled\n");
        assert_eq!(m["X-1"], Extras::default());
        assert!(parse_outputs_text("").is_empty());
    }

    #[test]
    fn commands_use_the_spellings_kscreen_doctor_accepts() {
        assert_eq!(brightness_args("eDP-1", 60), ["output.eDP-1.brightness.60"]);
        assert_eq!(brightness_args("eDP-1", 0), ["output.eDP-1.brightness.5"], "never a black screen");
        assert_eq!(brightness_args("eDP-1", 300), ["output.eDP-1.brightness.100"]);
        assert_eq!(vrr_args("DP-1", Vrr::Never), ["output.DP-1.vrrpolicy.never"]);
        assert_eq!(vrr_args("DP-1", Vrr::Automatic), ["output.DP-1.vrrpolicy.automatic"]);
        assert_eq!(hdr_args("DP-1", true), ["output.DP-1.hdr.enable"]);
        assert_eq!(wide_gamut_args("DP-1", false), ["output.DP-1.wcg.disable"]);
        assert_eq!(rgb_range_args("DP-1", RgbRange::Limited), ["output.DP-1.rgbrange.limited"]);
        assert_eq!(color_source_args("DP-1", ColorSource::Srgb, None), ["output.DP-1.colorProfileSource.sRGB"]);
        assert_eq!(
            color_source_args("DP-1", ColorSource::Icc, Some("/h/p.icc")),
            ["output.DP-1.iccprofile./h/p.icc", "output.DP-1.colorProfileSource.ICC"]
        );
        assert_eq!(tradeoff_args("DP-1", Tradeoff::Efficiency), ["output.DP-1.colorPowerTradeoff.preferEfficiency"]);
        assert_eq!(overscan_args("HDMI-A-1", 5), ["output.HDMI-A-1.overscan.5"]);
    }

    #[test]
    fn kwin_switches_are_notified() {
        let a = kwin_bool_args("Compositing", "AllowTearing", false);
        assert!(a.contains(&"--notify".to_string()));
        assert_eq!(a.last().unwrap(), "false");
    }
}
