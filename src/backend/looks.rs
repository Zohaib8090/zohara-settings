//! Quick looks: a whole desktop feel in one click, made only of the single options Settings already has (window
//! buttons, taskbar behaviour, desktop effects). Colours, icons and wallpaper are left to the person.

use super::desktop_style::{self as ds, Bar, ButtonSide, Hiding, Opacity};
use super::effects;

#[derive(Clone, Debug, PartialEq)]
pub struct Look {
    pub side: ButtonSide,
    pub bar: Bar,
    /// Every toggle in `effects::TOGGLES`, in that order.
    pub toggles: Vec<bool>,
    /// The id chosen in each of `effects::GROUPS` ("" is no animation).
    pub choices: Vec<String>,
    pub jelly: u32,
}

pub struct Preset {
    pub name: &'static str,
    pub about: &'static str,
    pub look: fn() -> Look,
}

fn toggles(on: &[&str]) -> Vec<bool> {
    effects::TOGGLES.iter().map(|t| on.contains(&t.id)).collect()
}

fn choices(a: &str, b: &str) -> Vec<String> {
    vec![a.to_string(), b.to_string()]
}

pub const PRESETS: [Preset; 4] = [
    Preset {
        name: "Zohara default",
        about: "Buttons on the right, a floating see-through taskbar, simple effects",
        look: || Look {
            side: ButtonSide::Right,
            bar: Bar::default(),
            toggles: toggles(&["blur", "highlightwindow", "zoom", "shakecursor"]),
            choices: choices("squash", "scale"),
            jelly: 1,
        },
    },
    Preset {
        name: "Mac style",
        about: "Buttons on the left, a taskbar that hides until you reach the edge, jelly windows, magic lamp",
        look: || Look {
            side: ButtonSide::Left,
            bar: Bar { hiding: Hiding::Auto, floating: true, opacity: Opacity::Clear, thickness: 52 },
            toggles: toggles(&["wobblywindows", "blur", "highlightwindow", "zoom", "shakecursor"]),
            choices: choices("magiclamp", "scale"),
            jelly: 0,
        },
    },
    Preset {
        name: "Windows style",
        about: "Buttons on the right, a solid taskbar that stays, quick animations",
        look: || Look {
            side: ButtonSide::Right,
            bar: Bar { hiding: Hiding::Always, floating: false, opacity: Opacity::Solid, thickness: 44 },
            toggles: toggles(&["blur", "translucency", "highlightwindow", "zoom"]),
            choices: choices("squash", "scale"),
            jelly: 1,
        },
    },
    Preset {
        name: "Plain and fast",
        about: "No animations or blur, a solid taskbar. Best for slow computers",
        look: || Look {
            side: ButtonSide::Right,
            bar: Bar { hiding: Hiding::Always, floating: false, opacity: Opacity::Solid, thickness: 44 },
            toggles: toggles(&[]),
            choices: choices("", ""),
            jelly: 1,
        },
    },
];

/// What the desktop looks like now, so it can be put back.
pub fn capture() -> Look {
    Look {
        side: ds::button_side(),
        bar: ds::bar(),
        toggles: effects::TOGGLES.iter().map(|t| effects::is_on(t.id)).collect(),
        choices: effects::GROUPS
            .iter()
            .map(|g| g.options[effects::chosen_index(g, effects::is_on) as usize].1.to_string())
            .collect(),
        jelly: effects::jelly_index(),
    }
}

/// Applies every part; true when KWin and Plasma accepted all of them.
pub fn apply(look: &Look) -> bool {
    let mut ok = ds::set_button_side(look.side);
    ok &= ds::apply_bar(look.bar);
    for (t, on) in effects::TOGGLES.iter().zip(&look.toggles) {
        ok &= effects::set(t.id, *on);
    }
    for (g, id) in effects::GROUPS.iter().zip(&look.choices) {
        ok &= effects::choose(g, id);
    }
    ok &= effects::set_jelly(look.jelly);
    ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_fills_every_part() {
        for p in &PRESETS {
            let l = (p.look)();
            assert_eq!(l.toggles.len(), effects::TOGGLES.len(), "{}", p.name);
            assert_eq!(l.choices.len(), effects::GROUPS.len(), "{}", p.name);
            for (g, c) in effects::GROUPS.iter().zip(&l.choices) {
                assert!(g.options.iter().any(|o| o.1 == c), "{}: {c}", p.name);
            }
            assert!((l.jelly as usize) < effects::JELLY.len());
            assert!((ds::MIN_THICKNESS..=ds::MAX_THICKNESS).contains(&l.bar.thickness), "{}", p.name);
        }
    }

    #[test]
    fn the_default_look_matches_the_zohara_taskbar_default() {
        assert_eq!((PRESETS[0].look)().bar, Bar::default());
    }

    #[test]
    fn mac_style_has_buttons_left_and_jelly_on() {
        let l = (PRESETS[1].look)();
        assert_eq!(l.side, ButtonSide::Left);
        let i = effects::TOGGLES.iter().position(|t| t.id == "wobblywindows").unwrap();
        assert!(l.toggles[i]);
    }

    /// Applies each preset to the real desktop for a moment, checks it took, then puts the original back.
    #[test]
    #[ignore]
    fn live_every_look_applies_to_the_real_desktop_and_the_original_comes_back() {
        let original = capture();
        println!("original: {original:?}");
        for p in &PRESETS {
            let want = (p.look)();
            assert!(apply(&want), "{} was not fully accepted", p.name);
            std::thread::sleep(std::time::Duration::from_millis(1500));
            let got = capture();
            assert_eq!(got.side, want.side, "{} buttons", p.name);
            assert_eq!(got.toggles, want.toggles, "{} effects", p.name);
            assert_eq!(got.choices, want.choices, "{} styles", p.name);
            assert_eq!((got.bar.hiding, got.bar.floating, got.bar.opacity), (want.bar.hiding, want.bar.floating, want.bar.opacity), "{} bar", p.name);
            println!("{}: applied and read back", p.name);
        }
        assert!(apply(&original));
        std::thread::sleep(std::time::Duration::from_millis(2000));
        let back = capture();
        assert_eq!(back.side, original.side);
        assert_eq!(back.toggles, original.toggles);
        assert_eq!(back.choices, original.choices);
        assert_eq!(back.bar, original.bar);
        println!("original restored");
    }
}
