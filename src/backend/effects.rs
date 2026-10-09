//! Desktop effects (KWin): jelly windows, the minimize animation, how windows open and close, dimming, mouse finders.
//! An effect is turned on for good by `[Plugins] <id>Enabled` in kwinrc and right now by KWin's `loadEffect` /
//! `unloadEffect`; asking `isEffectLoaded` tells what is really running.

use super::kconfig;
use std::process::Command;

pub struct Toggle {
    /// Effects that come in a separate package are only usable once their files are there.
    pub needs_files: bool,
    pub id: &'static str,
    pub title: &'static str,
    pub subtitle: &'static str,
}

pub const TOGGLES: [Toggle; 9] = [
    Toggle { needs_files: false, id: "wobblywindows", title: "Jelly windows", subtitle: "Windows wobble like jelly when you move them" },
    Toggle { needs_files: false, id: "blur", title: "Blur behind windows", subtitle: "Frosted glass behind see-through windows and menus" },
    Toggle { needs_files: false, id: "diminactive", title: "Dim inactive windows", subtitle: "Windows you are not using get a little darker" },
    Toggle { needs_files: false, id: "translucency", title: "See-through while moving", subtitle: "A window becomes see-through while you drag it" },
    Toggle { needs_files: false, id: "mouseclick", title: "Show mouse clicks", subtitle: "A ring appears where you click" },
    Toggle { needs_files: false, id: "shakecursor", title: "Shake to find the pointer", subtitle: "Move the mouse quickly and the pointer grows for a moment" },
    Toggle { needs_files: false, id: "highlightwindow", title: "Highlight window on hover", subtitle: "Point at a window's name in the overview to see where it is" },
    Toggle { needs_files: true, id: "cube", title: "Desktop cube", subtitle: "Press Meta+C to see your desktops as the faces of a 3D cube and pick one (needs 2 or more desktops)" },
    Toggle { needs_files: false, id: "zoom", title: "Screen zoom", subtitle: "Zoom in with Meta and + or -" },
];

/// False for an effect whose files are not installed (the cube comes with the extra desktop add-ons).
pub fn available(t: &Toggle) -> bool {
    !t.needs_files || std::path::Path::new(&format!("/usr/share/kwin/effects/{}", t.id)).exists()
}

/// Choices where only one effect may run: `(title, effect ids)` and the options as `(label, id or "" for none)`.
pub struct Group {
    pub title: &'static str,
    pub subtitle: &'static str,
    pub options: &'static [(&'static str, &'static str)],
}

pub const GROUPS: [Group; 2] = [
    Group {
        title: "When a window is minimized",
        subtitle: "How it leaves the screen",
        options: &[("Shrink (default)", "squash"), ("Magic lamp (like a Mac)", "magiclamp"), ("No animation", "")],
    },
    Group {
        title: "When a window opens or closes",
        subtitle: "How it appears and disappears",
        options: &[("Fade", "fade"), ("Scale up (default)", "scale"), ("Glide in", "glide"), ("No animation", "")],
    },
];

/// What to do to pick `chosen` in `group`: `(turn off, turn on)`.
pub fn plan_choice(group: &Group, chosen: &str) -> (Vec<&'static str>, Option<&'static str>) {
    let off = group.options.iter().map(|o| o.1).filter(|id| !id.is_empty() && *id != chosen).collect();
    let on = group.options.iter().map(|o| o.1).find(|id| !id.is_empty() && *id == chosen);
    (off, on)
}

/// Index of the option that is running now (the last one, "No animation", when none is).
pub fn chosen_index(group: &Group, loaded: impl Fn(&str) -> bool) -> u32 {
    group.options.iter().position(|o| !o.1.is_empty() && loaded(o.1)).unwrap_or_else(|| group.options.iter().position(|o| o.1.is_empty()).unwrap_or(0)) as u32
}

fn kwin(method: &str, id: &str) -> Option<String> {
    let o = Command::new("gdbus")
        .args(["call", "--session", "--dest", "org.kde.KWin", "--object-path", "/Effects", "--method"])
        .arg(format!("org.kde.kwin.Effects.{method}"))
        .arg(id)
        .output()
        .ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).into_owned())
}

pub fn is_on(id: &str) -> bool {
    kwin("isEffectLoaded", id).map(|a| a.contains("true")).unwrap_or(false)
}

pub fn set(id: &str, on: bool) -> bool {
    kconfig::write("kwinrc", &["Plugins"], &format!("{id}Enabled"), if on { "true" } else { "false" });
    if on {
        kwin("loadEffect", id).is_some()
    } else {
        kwin("unloadEffect", id).is_some()
    }
}

pub fn choose(group: &Group, chosen: &str) -> bool {
    let (off, on) = plan_choice(group, chosen);
    let mut ok = true;
    for id in off {
        ok &= set(id, false);
    }
    if let Some(id) = on {
        ok &= set(id, true);
    }
    ok
}

/// How strongly windows wobble: `(label, Stiffness, Drag, MoveFactor)` for kwinrc `[Effect-wobblywindows]`.
pub const JELLY: [(&str, u32, u32, u32); 3] = [("Gentle", 15, 80, 5), ("Normal", 10, 85, 10), ("Very jelly", 4, 94, 30)];

pub fn jelly_index() -> u32 {
    let g = |k: &str| kconfig::read("kwinrc", &["Effect-wobblywindows"], k).and_then(|v| v.parse::<u32>().ok());
    match (g("Stiffness"), g("Drag"), g("MoveFactor")) {
        (Some(s), Some(d), Some(m)) => JELLY.iter().position(|j| (j.1, j.2, j.3) == (s, d, m)).unwrap_or(1) as u32,
        _ => 1,
    }
}

pub fn set_jelly(index: u32) -> bool {
    let j = JELLY[(index as usize).min(JELLY.len() - 1)];
    for (k, v) in [("Stiffness", j.1), ("Drag", j.2), ("MoveFactor", j.3)] {
        kconfig::write("kwinrc", &["Effect-wobblywindows"], k, &v.to_string());
    }
    kwin("reconfigureEffect", "wobblywindows").is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choosing_one_turns_the_others_of_its_group_off() {
        let (off, on) = plan_choice(&GROUPS[0], "magiclamp");
        assert_eq!(off, vec!["squash"]);
        assert_eq!(on, Some("magiclamp"));
    }

    #[test]
    fn no_animation_turns_everything_in_the_group_off() {
        let (off, on) = plan_choice(&GROUPS[1], "");
        assert_eq!(off, vec!["fade", "scale", "glide"]);
        assert_eq!(on, None);
    }

    #[test]
    fn running_effect_is_shown_as_the_choice() {
        assert_eq!(chosen_index(&GROUPS[0], |id| id == "magiclamp"), 1);
        assert_eq!(chosen_index(&GROUPS[0], |_| false), 2);
    }

    #[test]
    fn every_effect_named_here_exists_in_kwin() {
        // the effect ids KWin 6.7 lists (checked on a real session)
        let known = "fade scale glide squash magiclamp wobblywindows blur diminactive translucency mouseclick shakecursor highlightwindow zoom cube";
        for t in &TOGGLES {
            assert!(known.split(' ').any(|k| k == t.id), "{}", t.id);
        }
        for g in &GROUPS {
            for o in g.options.iter().filter(|o| !o.1.is_empty()) {
                assert!(known.split(' ').any(|k| k == o.1), "{}", o.1);
            }
        }
    }

    #[test]
    fn normal_jelly_is_kdes_own_default() {
        assert_eq!((JELLY[1].1, JELLY[1].2, JELLY[1].3), (10, 85, 10));
    }

    /// Turns real effects on and off for a moment and puts them back.
    #[test]
    #[ignore]
    fn live_effects_load_and_unload_in_the_running_kwin_and_come_back() {
        for id in ["wobblywindows", "mouseclick", "diminactive"] {
            let was = is_on(id);
            assert!(set(id, !was));
            std::thread::sleep(std::time::Duration::from_millis(500));
            assert_eq!(is_on(id), !was, "{id} did not change");
            assert!(set(id, was));
            std::thread::sleep(std::time::Duration::from_millis(500));
            assert_eq!(is_on(id), was, "{id} did not come back");
            println!("{id}: toggled and restored (was {was})");
        }
        let (was_idx, group) = (chosen_index(&GROUPS[0], is_on), &GROUPS[0]);
        assert!(choose(group, "magiclamp"));
        assert!(is_on("magiclamp") && !is_on("squash"));
        assert!(choose(group, group.options[was_idx as usize].1));
        assert_eq!(chosen_index(group, is_on), was_idx);
        println!("minimize style switched and restored");
    }
}
