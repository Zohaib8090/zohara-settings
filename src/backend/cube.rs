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
    /// 4 fingers up opens the cube's overview instead of KWin's Overview.
    pub overview_gesture: bool,
    /// The desktop bar and window tray glide in and out of the overview (off: they appear at once).
    pub tray_animation: bool,
    /// How long that glide takes, in milliseconds.
    pub tray_time: u32,
}

impl Default for Cube {
    fn default() -> Self {
        Cube { duration: 450, open_cube: true, pullback: 0.6, tilt: 0.0, overview_gesture: true, tray_animation: true, tray_time: 350 }
    }
}

pub const DURATION: (u32, u32) = (150, 1500);
pub const PULLBACK: (f64, f64) = (0.0, 1.5);
pub const TILT: (f64, f64) = (0.0, 30.0);
pub const TRAY_TIME: (u32, u32) = (100, 1200);

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
        overview_gesture: get("OverviewGesture").map(|v| v != "false").unwrap_or(d.overview_gesture),
        tray_animation: get("TrayAnimation").map(|v| v != "false").unwrap_or(d.tray_animation),
        tray_time: get("TrayTime").and_then(|v| v.parse().ok()).map(|v: u32| v.clamp(TRAY_TIME.0, TRAY_TIME.1)).unwrap_or(d.tray_time),
    }
}

/// Saves the options and tells the running effect to read them again.
pub fn write(c: Cube) {
    let w = |k: &str, v: String| kconfig::write("kwinrc", &GROUP, k, &v);
    w("Duration", c.duration.clamp(DURATION.0, DURATION.1).to_string());
    w("OpenCube", c.open_cube.to_string());
    w("Pullback", format!("{:.2}", c.pullback.clamp(PULLBACK.0, PULLBACK.1)));
    w("Tilt", format!("{:.1}", c.tilt.clamp(TILT.0, TILT.1)));
    w("OverviewGesture", c.overview_gesture.to_string());
    w("TrayAnimation", c.tray_animation.to_string());
    w("TrayTime", c.tray_time.clamp(TRAY_TIME.0, TRAY_TIME.1).to_string());
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
        // main.xml of the effect: Duration 450, OpenCube true, Pullback 0.6, Tilt 0, OverviewGesture true, TrayAnimation true, TrayTime 350
        assert_eq!(Cube::default(), Cube { duration: 450, open_cube: true, pullback: 0.6, tilt: 0.0, overview_gesture: true, tray_animation: true, tray_time: 350 });
    }

    #[test]
    fn the_effect_package_in_this_repo_declares_every_option_used_here() {
        let xml = include_str!("../../data/cube-effect/contents/config/main.xml");
        for key in ["Duration", "OpenCube", "Pullback", "Tilt", "OverviewGesture", "TrayAnimation", "TrayTime"] {
            assert!(xml.contains(&format!("name=\"{key}\"")), "main.xml lacks {key}");
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
        let want = Cube { duration: 700, open_cube: false, pullback: 0.9, tilt: 12.0, overview_gesture: false, tray_animation: false, tray_time: 800 };
        write(want);
        let got = read();
        println!("wrote {want:?}, read {got:?}");
        assert_eq!(got, want);
        // put everything back: delete the keys we may have created
        for k in ["Duration", "OpenCube", "Pullback", "Tilt", "OverviewGesture", "TrayAnimation", "TrayTime"] {
            kconfig::delete_notify("kwinrc", &GROUP, k);
        }
        assert_eq!(read(), Cube::default());
        println!("before was {before:?}; keys removed, defaults again");
    }
}
