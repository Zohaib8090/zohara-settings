//! Zohara Cube: the 3D desktop cube (a QML KWin effect shipped in `/usr/share/kwin/effects/zoharacube`). It replaces KWin's
//! Slide animation while it is on, and by default also takes the 4-finger-up gesture from KWin's own Overview (the cube
//! has its own overview; the effect's `OverviewGesture` option says which one owns the gesture).
//! Its options live in kwinrc `[Effect-zoharacube]` (read by the effect as `effect.configuration`).

use super::{effects, kconfig};
use std::path::Path;

pub const ID: &str = "zoharacube";
const GROUP: [&str; 1] = ["Effect-zoharacube"];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cube {
    /// How long the cube takes to turn after you let go, in milliseconds.
    pub duration: u32,
    /// The faces cover half a circle with a gap between the last and first desktop (like Zorin); off = a closed prism.
    pub open_cube: bool,
    /// How far the cube moves away from you while it turns, 0 to 1.5.
    pub pullback: f64,
    /// Vertical tilt while it turns, in degrees.
    pub tilt: f64,
    /// How strongly the windows float in front of their face, 0 to 1.
    pub parallax: f64,
    /// 4 fingers up opens the cube's overview instead of KWin's Overview.
    pub overview_gesture: bool,
    /// The desktop bar and window tray glide in and out of the overview (off: they appear at once).
    pub tray_animation: bool,
    /// How long that glide takes, in milliseconds.
    pub tray_time: u32,
    /// Scrolling in the overview: 0 up/down and left/right, 1 up/down only, 2 left/right only.
    pub scroll_mode: u32,
    /// Scroll the other way round.
    pub scroll_reverse: bool,
    /// The overview's side panel: every open window of every desktop down the left side.
    pub side_panel: bool,
    /// Each window in the panel shows its picture (off: only its icon and name).
    pub panel_preview: bool,
    /// The panel's width in pixels.
    pub panel_width: u32,
    /// How solid the panel is, 0.3 to 1.
    pub panel_opacity: f64,
    /// 0 dark, 1 light.
    pub panel_style: u32,
    /// The panel glides in from the left (off: it appears at once).
    pub panel_animation: bool,
    /// The window under the pointer grows a little and lights up.
    pub panel_hover: bool,
}

impl Default for Cube {
    fn default() -> Self {
        Cube { duration: 450, open_cube: true, pullback: 0.6, tilt: 0.0, parallax: 0.5, overview_gesture: true, tray_animation: true, tray_time: 350, scroll_mode: 0, scroll_reverse: false,
            side_panel: true, panel_preview: true, panel_width: 280, panel_opacity: 0.85, panel_style: 0, panel_animation: true, panel_hover: true }
    }
}

pub const DURATION: (u32, u32) = (150, 1500);
pub const PULLBACK: (f64, f64) = (0.0, 1.5);
pub const TILT: (f64, f64) = (0.0, 30.0);
pub const PARALLAX: (f64, f64) = (0.0, 1.0);
pub const SCROLL_MODES: [&str; 3] = ["Up/down and left/right", "Up/down only", "Left/right only"];
pub const TRAY_TIME: (u32, u32) = (100, 1200);
pub const PANEL_WIDTH: (u32, u32) = (200, 420);
pub const PANEL_OPACITY: (f64, f64) = (0.3, 1.0);
pub const PANEL_STYLES: [&str; 2] = ["Dark", "Light"];

/// Whether the effect's files are on this computer.
pub fn installed() -> bool {
    let home = std::env::var("HOME").unwrap_or_default();
    Path::new("/usr/share/kwin/effects/zoharacube").exists() || Path::new(&format!("{home}/.local/share/kwin/effects/zoharacube")).exists()
}

pub fn read() -> Cube {
    let d = Cube::default();
    let get = |k: &str| kconfig::read("kwinrc", &GROUP, k);
    Cube {
        duration: get("Duration").and_then(|v| v.parse().ok()).map(|v: u32| v.clamp(DURATION.0, DURATION.1)).unwrap_or(d.duration),
        open_cube: get("OpenCube").map(|v| v != "false").unwrap_or(d.open_cube),
        pullback: get("Pullback").and_then(|v| v.parse().ok()).map(|v: f64| v.clamp(PULLBACK.0, PULLBACK.1)).unwrap_or(d.pullback),
        tilt: get("Tilt").and_then(|v| v.parse().ok()).map(|v: f64| v.clamp(TILT.0, TILT.1)).unwrap_or(d.tilt),
        parallax: get("Parallax").and_then(|v| v.parse().ok()).map(|v: f64| v.clamp(0.0, 1.0)).unwrap_or(d.parallax),
        overview_gesture: get("OverviewGesture").map(|v| v != "false").unwrap_or(d.overview_gesture),
        tray_animation: get("TrayAnimation").map(|v| v != "false").unwrap_or(d.tray_animation),
        tray_time: get("TrayTime").and_then(|v| v.parse().ok()).map(|v: u32| v.clamp(TRAY_TIME.0, TRAY_TIME.1)).unwrap_or(d.tray_time),
        scroll_mode: get("ScrollMode").and_then(|v| v.parse().ok()).map(|v: u32| v.min(2)).unwrap_or(d.scroll_mode),
        scroll_reverse: get("ScrollReverse").map(|v| v == "true").unwrap_or(d.scroll_reverse),
        side_panel: get("SidePanel").map(|v| v != "false").unwrap_or(d.side_panel),
        panel_preview: get("SidePanelPreview").map(|v| v != "false").unwrap_or(d.panel_preview),
        panel_width: get("SidePanelWidth").and_then(|v| v.parse().ok()).map(|v: u32| v.clamp(PANEL_WIDTH.0, PANEL_WIDTH.1)).unwrap_or(d.panel_width),
        panel_opacity: get("SidePanelOpacity").and_then(|v| v.parse().ok()).map(|v: f64| v.clamp(PANEL_OPACITY.0, PANEL_OPACITY.1)).unwrap_or(d.panel_opacity),
        panel_style: get("SidePanelStyle").and_then(|v| v.parse().ok()).map(|v: u32| v.min(1)).unwrap_or(d.panel_style),
        panel_animation: get("SidePanelAnimation").map(|v| v != "false").unwrap_or(d.panel_animation),
        panel_hover: get("SidePanelHover").map(|v| v != "false").unwrap_or(d.panel_hover),
    }
}

/// Saves the options and tells the running effect to read them again.
pub fn write(c: Cube) {
    let w = |k: &str, v: String| kconfig::write("kwinrc", &GROUP, k, &v);
    w("Duration", c.duration.clamp(DURATION.0, DURATION.1).to_string());
    w("OpenCube", c.open_cube.to_string());
    w("Pullback", format!("{:.2}", c.pullback.clamp(PULLBACK.0, PULLBACK.1)));
    w("Tilt", format!("{:.1}", c.tilt.clamp(TILT.0, TILT.1)));
    w("Parallax", format!("{:.2}", c.parallax.clamp(0.0, 1.0)));
    w("OverviewGesture", c.overview_gesture.to_string());
    w("TrayAnimation", c.tray_animation.to_string());
    w("TrayTime", c.tray_time.clamp(TRAY_TIME.0, TRAY_TIME.1).to_string());
    w("ScrollMode", c.scroll_mode.min(2).to_string());
    w("ScrollReverse", c.scroll_reverse.to_string());
    w("SidePanel", c.side_panel.to_string());
    w("SidePanelPreview", c.panel_preview.to_string());
    w("SidePanelWidth", c.panel_width.clamp(PANEL_WIDTH.0, PANEL_WIDTH.1).to_string());
    w("SidePanelOpacity", format!("{:.2}", c.panel_opacity.clamp(PANEL_OPACITY.0, PANEL_OPACITY.1)));
    w("SidePanelStyle", c.panel_style.min(1).to_string());
    w("SidePanelAnimation", c.panel_animation.to_string());
    w("SidePanelHover", c.panel_hover.to_string());
    let _ = std::process::Command::new("qdbus6").args(["org.kde.KWin", "/Effects", "org.kde.kwin.Effects.reconfigureEffect", ID]).status();
}

/// Effects to turn on or off, in order, to make the cube `on` or off. The cube replaces Slide (both move desktops).
/// KWin's Overview only gives way when the cube takes the 4-finger-up gesture.
pub fn plan(on: bool, overview_gesture: bool) -> Vec<(&'static str, bool)> {
    let mut p = Vec::new();
    if on {
        p.push(("slide", false));
        if overview_gesture {
            p.push(("overview", false));
        }
        p.push((ID, true));
    } else {
        p.push((ID, false));
        p.push(("slide", true));
        p.push(("overview", true));
    }
    p
}

pub fn is_on() -> bool {
    effects::is_on(ID)
}

/// Turns the cube on or off for good and right now.
pub fn set_on(on: bool, overview_gesture: bool) -> bool {
    let mut ok = true;
    for (id, state) in plan(on, overview_gesture) {
        ok &= effects::set(id, state);
    }
    ok
}

/// The Overview effect follows who owns the gesture while the cube is on.
pub fn set_overview_gesture(c: Cube, cube_on: bool) -> bool {
    write(c);
    if cube_on {
        effects::set("overview", !c.overview_gesture)
    } else {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turning_the_cube_on_gives_slide_up_and_takes_the_overview_gesture() {
        assert_eq!(plan(true, true), vec![("slide", false), ("overview", false), (ID, true)]);
    }

    #[test]
    fn the_cube_can_leave_kwins_overview_alone() {
        assert_eq!(plan(true, false), vec![("slide", false), (ID, true)]);
    }

    #[test]
    fn turning_it_off_puts_slide_and_overview_back() {
        assert_eq!(plan(false, true), vec![(ID, false), ("slide", true), ("overview", true)]);
    }

    #[test]
    fn defaults_match_the_effects_own_defaults() {
        // main.xml of the effect: Duration 450, OpenCube true, Pullback 0.6, Tilt 0, Parallax 0.5, OverviewGesture true, TrayAnimation true, TrayTime 350, ScrollMode 0, ScrollReverse false,
        // SidePanel true, SidePanelPreview true, SidePanelWidth 280, SidePanelOpacity 0.85, SidePanelStyle 0, SidePanelAnimation true, SidePanelHover true
        assert_eq!(Cube::default(), Cube {
            duration: 450, open_cube: true, pullback: 0.6, tilt: 0.0, parallax: 0.5, overview_gesture: true, tray_animation: true, tray_time: 350, scroll_mode: 0, scroll_reverse: false,
            side_panel: true, panel_preview: true, panel_width: 280, panel_opacity: 0.85, panel_style: 0, panel_animation: true, panel_hover: true,
        });
    }

    #[test]
    fn the_side_panel_files_are_in_the_effect_package() {
        // the effect loads these by name; a missing file leaves the overview without its panel
        let tray = include_str!("../../data/cube-effect/contents/ui/WindowTray.qml");
        assert!(tray.contains("SidePanel {"));
        assert!(include_str!("../../data/cube-effect/contents/ui/SidePanel.qml").contains("DesktopGroup"));
        assert!(include_str!("../../data/cube-effect/contents/ui/DesktopGroup.qml").contains("keys: [\"zwin\"]"));
    }

    #[test]
    fn side_panel_defaults_in_the_xml_match_the_ones_here() {
        let xml = include_str!("../../data/cube-effect/contents/config/main.xml");
        let default_of = |key: &str| {
            let at = xml.find(&format!("name=\"{key}\"")).unwrap();
            let rest = &xml[at..];
            let from = rest.find("<default>").unwrap() + 9;
            rest[from..rest.find("</default>").unwrap()].to_string()
        };
        let d = Cube::default();
        assert_eq!(default_of("SidePanel"), d.side_panel.to_string());
        assert_eq!(default_of("SidePanelPreview"), d.panel_preview.to_string());
        assert_eq!(default_of("SidePanelWidth"), d.panel_width.to_string());
        assert_eq!(default_of("SidePanelOpacity"), d.panel_opacity.to_string());
        assert_eq!(default_of("SidePanelStyle"), d.panel_style.to_string());
        assert_eq!(default_of("SidePanelAnimation"), d.panel_animation.to_string());
        assert_eq!(default_of("SidePanelHover"), d.panel_hover.to_string());
    }

    #[test]
    fn the_effect_package_in_this_repo_declares_every_option_used_here() {
        let xml = include_str!("../../data/cube-effect/contents/config/main.xml");
        for key in ["Duration", "OpenCube", "Pullback", "Tilt", "Parallax", "OverviewGesture", "TrayAnimation", "TrayTime", "ScrollMode", "ScrollReverse", "SidePanel", "SidePanelPreview", "SidePanelWidth", "SidePanelOpacity", "SidePanelStyle", "SidePanelAnimation", "SidePanelHover"] {
            assert!(xml.contains(&format!("name=\"{key}\"")));
        }
    }
}

#[cfg(test)]
mod live {
    use super::*;

    /// Writes cube options to the real kwinrc and reads them back, then removes them again. Does not turn the cube on
    /// or off (KWin keeps the QML of an effect it has loaded until the next login, so that is not safe to test live).
    #[test]
    #[ignore]
    fn live_cube_options_round_trip_through_kwinrc() {
        let before = read();
        let want = Cube { duration: 700, open_cube: false, pullback: 0.9, tilt: 12.0, parallax: 0.7, overview_gesture: false, tray_animation: false, tray_time: 800, scroll_mode: 2, scroll_reverse: true,
            side_panel: false, panel_preview: false, panel_width: 340, panel_opacity: 0.6, panel_style: 1, panel_animation: false, panel_hover: false };
        write(want);
        let got = read();
        println!("wrote {want:?}, read {got:?}");
        assert_eq!(got, want);
        // put everything back: delete the keys we may have created
        for k in ["Duration", "OpenCube", "Pullback", "Tilt", "Parallax", "OverviewGesture", "TrayAnimation", "TrayTime", "ScrollMode", "ScrollReverse", "SidePanel", "SidePanelPreview", "SidePanelWidth", "SidePanelOpacity", "SidePanelStyle", "SidePanelAnimation", "SidePanelHover"] {
            kconfig::delete_notify("kwinrc", &GROUP, k);
        }
        assert_eq!(read(), Cube::default());
        println!("before was {before:?}; keys removed, defaults again");
    }
}
