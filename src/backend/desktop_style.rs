//! Desktop style: which side the window buttons sit on and how the taskbar behaves (hiding, floating, see-through,
//! thickness). Plasma keeps the window buttons in kwinrc (`ButtonsOnLeft`/`ButtonsOnRight`, one letter per button) and the
//! taskbar's look in plasmashellrc `[PlasmaViews][Panel N]`; the running shell is changed through its scripting
//! interface, which also writes those keys.

use super::kconfig;
use std::process::Command;

/// Window button letters in KWin's own notation: M menu, S keep-above, I minimize, A maximize, X close.
const LEFT_DEFAULT: &str = "MS";
const RIGHT_DEFAULT: &str = "IAX";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonSide {
    Right,
    Left,
}

/// The side the close button is on. Both lists empty or missing means Plasma's defaults (close on the right).
pub fn side_of(left: &str, right: &str) -> ButtonSide {
    if left.contains('X') && !right.contains('X') {
        ButtonSide::Left
    } else {
        ButtonSide::Right
    }
}

/// The `(left, right)` lists with the minimize/maximize/close group moved to `side`. The menu and keep-above buttons
/// stay where they are, so a person's own choice of those is kept.
pub fn lists_for(side: ButtonSide, left: &str, right: &str) -> (String, String) {
    let group = |c: &char| "IAX".contains(*c);
    let mut all: Vec<char> = left.chars().chain(right.chars()).filter(group).collect();
    // The order is what the person sees: close, minimize, maximize on a mac-like left side, minimize, maximize, close on the right.
    all.sort_by_key(|c| match (side, c) {
        (ButtonSide::Left, 'X') => 0,
        (ButtonSide::Left, 'I') => 1,
        (ButtonSide::Left, _) => 2,
        (ButtonSide::Right, 'I') => 0,
        (ButtonSide::Right, 'A') => 1,
        (ButtonSide::Right, _) => 2,
    });
    if all.is_empty() {
        all = RIGHT_DEFAULT.chars().collect();
    }
    let keep = |s: &str| s.chars().filter(|c| !group(c)).collect::<String>();
    let (l, r) = (keep(left), keep(right));
    let group: String = all.into_iter().collect();
    match side {
        ButtonSide::Left => (format!("{group}{l}{r}"), String::new()),
        ButtonSide::Right => (l, format!("{r}{group}")),
    }
}

/// A window button list as stored. `None` is "not set" (Plasma's default applies); an empty list is a real value
/// (no buttons on that side), which `kconfig::read` would report as `None` too.
fn button_list(key: &str) -> Option<String> {
    let o = Command::new("kreadconfig6")
        .args(["--file", "kwinrc", "--group", "org.kde.kdecoration2", "--key", key, "--default", "\u{1}unset"])
        .output()
        .ok()?;
    let v = String::from_utf8_lossy(&o.stdout).trim_end_matches('\n').to_string();
    (v != "\u{1}unset").then_some(v)
}

pub fn button_side() -> ButtonSide {
    let l = button_list("ButtonsOnLeft").unwrap_or_else(|| LEFT_DEFAULT.into());
    let r = button_list("ButtonsOnRight").unwrap_or_else(|| RIGHT_DEFAULT.into());
    side_of(&l, &r)
}

pub fn set_button_side(side: ButtonSide) -> bool {
    let l = button_list("ButtonsOnLeft").unwrap_or_else(|| LEFT_DEFAULT.into());
    let r = button_list("ButtonsOnRight").unwrap_or_else(|| RIGHT_DEFAULT.into());
    let (nl, nr) = lists_for(side, &l, &r);
    kconfig::write("kwinrc", &["org.kde.kdecoration2"], "ButtonsOnLeft", &nl);
    // An empty list must be written as an empty value, not deleted (a missing key means "use the default").
    kconfig::write("kwinrc", &["org.kde.kdecoration2"], "ButtonsOnRight", &nr);
    reconfigure_kwin()
}

fn reconfigure_kwin() -> bool {
    Command::new("dbus-send")
        .args(["--session", "--type=method_call", "--dest=org.kde.KWin", "/KWin", "org.kde.KWin.reconfigure"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hiding {
    Always,    // the bar always stays
    Auto,      // slides away, shows when the pointer reaches the edge
    Dodge,     // stays unless a window covers it
}

pub const HIDING: [(&str, Hiding, &str); 3] = [
    ("Always shown", Hiding::Always, "none"),
    ("Hide automatically", Hiding::Auto, "autohide"),
    ("Hide when a window covers it", Hiding::Dodge, "dodgewindows"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opacity {
    Adaptive,
    Solid,
    Clear,
}

pub const OPACITY: [(&str, Opacity, &str); 3] =
    [("Automatic", Opacity::Adaptive, "adaptive"), ("Solid", Opacity::Solid, "opaque"), ("See-through", Opacity::Clear, "translucent")];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bar {
    pub hiding: Hiding,
    pub floating: bool,
    pub opacity: Opacity,
    pub thickness: u32,
}

impl Default for Bar {
    fn default() -> Self {
        Bar { hiding: Hiding::Always, floating: true, opacity: Opacity::Clear, thickness: 46 }
    }
}

pub const MIN_THICKNESS: u32 = 28;
pub const MAX_THICKNESS: u32 = 96;

/// Reads the main taskbar's look from plasmashellrc (the panel that holds the task manager is `Panel N`, N is its
/// containment id; the first `[PlasmaViews][Panel N]` group is taken, like `parse_taskbar_state` does).
pub fn parse_bar(plasmashellrc: &str) -> Bar {
    let mut bar = Bar::default();
    let (mut in_view, mut in_defaults) = (false, false);
    let mut seen = false;
    for l in plasmashellrc.lines().map(str::trim) {
        if l.starts_with('[') {
            let main = l.starts_with("[PlasmaViews][Panel ") && l.matches('[').count() == 2;
            let defaults = l.starts_with("[PlasmaViews][Panel ") && l.ends_with("[Defaults]");
            in_view = main && !seen;
            if main {
                seen = true;
            }
            in_defaults = defaults && !in_view && seen_defaults_ok(l);
            continue;
        }
        if in_view {
            if let Some(v) = l.strip_prefix("floating=") {
                bar.floating = v != "0";
            } else if let Some(v) = l.strip_prefix("panelOpacity=") {
                bar.opacity = match v {
                    "0" => Opacity::Adaptive,
                    "1" => Opacity::Solid,
                    _ => Opacity::Clear,
                };
            } else if let Some(v) = l.strip_prefix("panelVisibility=") {
                bar.hiding = match v {
                    "1" => Hiding::Auto,
                    "2" => Hiding::Dodge,
                    _ => Hiding::Always,
                };
            }
        } else if in_defaults {
            if let Some(v) = l.strip_prefix("thickness=").and_then(|v| v.parse().ok()) {
                bar.thickness = v;
            }
        }
    }
    bar
}

fn seen_defaults_ok(header: &str) -> bool {
    header.matches('[').count() == 3
}

/// The taskbar's look as the running shell has it. plasmashellrc is written some seconds after a change, so it is
/// only the fallback (for when the shell cannot be asked).
pub fn bar() -> Bar {
    ask_shell().unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_default();
        std::fs::read_to_string(format!("{home}/.config/plasmashellrc")).map(|t| parse_bar(&t)).unwrap_or_default()
    })
}

const READ_SCRIPT: &str = r#"var o = "";
panels().forEach(function (p) {
  var mine = false;
  p.widgets().forEach(function (w) { var t = w.type; if (t == "org.kde.plasma.icontasks" || t == "org.kde.plasma.taskmanager") mine = true; });
  if (mine && o == "") o = "hiding=" + p.hiding + ";floating=" + p.floating + ";opacity=" + p.opacity + ";height=" + p.height;
});
print(o);"#;

fn ask_shell() -> Option<Bar> {
    let out = Command::new("gdbus")
        .args(["call", "--session", "--dest", "org.kde.plasmashell", "--object-path", "/PlasmaShell", "--method", "org.kde.PlasmaShell.evaluateScript", READ_SCRIPT])
        .output()
        .ok()?;
    out.status.success().then(|| parse_script_answer(&String::from_utf8_lossy(&out.stdout))).flatten()
}

/// `('hiding=none;floating=true;opacity=translucent;height=46',)` -> Bar.
pub fn parse_script_answer(answer: &str) -> Option<Bar> {
    let mut bar = Bar::default();
    let mut found = false;
    for part in answer.trim_matches(|c: char| "(),' \n".contains(c)).split(';') {
        let Some((k, v)) = part.split_once('=') else { continue };
        found = true;
        match k {
            "hiding" => bar.hiding = HIDING.iter().find(|h| h.2 == v).map(|h| h.1).unwrap_or(Hiding::Always),
            "floating" => bar.floating = v == "true",
            "opacity" => bar.opacity = OPACITY.iter().find(|o| o.2 == v).map(|o| o.1).unwrap_or(Opacity::Adaptive),
            "height" => bar.thickness = v.parse().unwrap_or(bar.thickness),
            _ => found = false,
        }
    }
    found.then_some(bar)
}

/// Plasma script that sets the look of the panel holding the task manager (the same panel the alignment script finds).
pub fn bar_script(bar: Bar) -> String {
    let hiding = HIDING.iter().find(|h| h.1 == bar.hiding).map(|h| h.2).unwrap_or("none");
    let opacity = OPACITY.iter().find(|o| o.1 == bar.opacity).map(|o| o.2).unwrap_or("translucent");
    let thick = bar.thickness.clamp(MIN_THICKNESS, MAX_THICKNESS);
    format!(
        r#"panels().forEach(function (p) {{
  var mine = false;
  p.widgets().forEach(function (w) {{ var t = w.type; if (t == "org.kde.plasma.icontasks" || t == "org.kde.plasma.taskmanager") mine = true; }});
  if (!mine) return;
  p.hiding = "{hiding}"; p.floating = {floating}; p.opacity = "{opacity}"; p.height = {thick};
}});"#,
        floating = bar.floating
    )
}

pub fn apply_bar(bar: Bar) -> bool {
    Command::new("dbus-send")
        .args(["--session", "--type=method_call", "--dest=org.kde.plasmashell", "/PlasmaShell", "org.kde.PlasmaShell.evaluateScript"])
        .arg(format!("string:{}", bar_script(bar)))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plasma_default_has_close_on_the_right() {
        assert_eq!(side_of("MS", "IAX"), ButtonSide::Right);
        assert_eq!(side_of("", ""), ButtonSide::Right);
    }

    #[test]
    fn moving_the_buttons_left_puts_close_first_and_keeps_the_menu_buttons() {
        let (l, r) = lists_for(ButtonSide::Left, "MS", "IAX");
        assert_eq!(l, "XIAMS");
        assert_eq!(r, "");
        assert_eq!(side_of(&l, &r), ButtonSide::Left);
    }

    #[test]
    fn moving_back_right_restores_the_usual_order_and_is_stable() {
        let (l, r) = lists_for(ButtonSide::Left, "MS", "IAX");
        let (l2, r2) = lists_for(ButtonSide::Right, &l, &r);
        assert_eq!((l2.as_str(), r2.as_str()), ("MS", "IAX"));
        assert_eq!(lists_for(ButtonSide::Right, &l2, &r2), (l2.clone(), r2.clone()));
        assert_eq!(lists_for(ButtonSide::Left, &l, &r), (l, r));
    }

    #[test]
    fn a_person_with_no_buttons_gets_the_three_back() {
        assert_eq!(lists_for(ButtonSide::Right, "", "").1, "IAX");
    }

    const RC: &str = "[PlasmaViews][Panel 2]\nfloating=0\npanelOpacity=1\npanelVisibility=1\n\n[PlasmaViews][Panel 2][Defaults]\nthickness=60\n\n[Updates]\nperformed=x\n";

    #[test]
    fn bar_is_read_from_plasmashellrc() {
        let b = parse_bar(RC);
        assert_eq!(b, Bar { hiding: Hiding::Auto, floating: false, opacity: Opacity::Solid, thickness: 60 });
    }

    #[test]
    fn the_shells_answer_is_read() {
        let b = parse_script_answer("('hiding=autohide;floating=false;opacity=opaque;height=58',)\n").unwrap();
        assert_eq!(b, Bar { hiding: Hiding::Auto, floating: false, opacity: Opacity::Solid, thickness: 58 });
        assert!(parse_script_answer("('',)").is_none());
    }

    #[test]
    fn missing_file_content_gives_the_zohara_default() {
        assert_eq!(parse_bar(""), Bar::default());
    }

    #[test]
    fn script_only_touches_the_task_manager_panel_and_clamps_size() {
        let s = bar_script(Bar { hiding: Hiding::Dodge, floating: false, opacity: Opacity::Clear, thickness: 500 });
        assert!(s.contains("dodgewindows") && s.contains("p.floating = false") && s.contains("translucent"));
        assert!(s.contains(&format!("p.height = {MAX_THICKNESS}")));
        assert!(s.contains("if (!mine) return;"));
    }
}

#[cfg(test)]
mod live {
    use super::*;

    /// Changes the real taskbar and window buttons for a few seconds and puts them back. Not while someone is working.
    #[test]
    #[ignore]
    fn live_desktop_style_changes_the_real_taskbar_and_window_buttons_and_restores_them() {
        let (before_bar, before_side) = (bar(), button_side());
        let (l0, r0) = (button_list("ButtonsOnLeft"), button_list("ButtonsOnRight"));
        println!("before: {before_bar:?} {before_side:?} left={l0:?} right={r0:?}");

        let wanted = Bar { hiding: Hiding::Dodge, floating: !before_bar.floating, opacity: Opacity::Solid, thickness: 58 };
        assert!(apply_bar(wanted), "plasmashell refused the script");
        std::thread::sleep(std::time::Duration::from_secs(3));
        let got = bar();
        println!("after:  {got:?}");
        assert_eq!(got.hiding, wanted.hiding);
        assert_eq!(got.floating, wanted.floating);
        assert_eq!(got.opacity, wanted.opacity);

        assert!(set_button_side(ButtonSide::Left));
        assert_eq!(button_side(), ButtonSide::Left);
        println!("buttons: left={:?} right={:?}", button_list("ButtonsOnLeft"), button_list("ButtonsOnRight"));
        std::thread::sleep(std::time::Duration::from_secs(2));

        // put everything back exactly
        assert!(apply_bar(before_bar));
        match (l0, r0) {
            (None, None) => {
                kconfig::delete_notify("kwinrc", &["org.kde.kdecoration2"], "ButtonsOnLeft");
                kconfig::delete_notify("kwinrc", &["org.kde.kdecoration2"], "ButtonsOnRight");
            }
            (l, r) => {
                kconfig::write("kwinrc", &["org.kde.kdecoration2"], "ButtonsOnLeft", &l.unwrap_or_default());
                kconfig::write("kwinrc", &["org.kde.kdecoration2"], "ButtonsOnRight", &r.unwrap_or_default());
            }
        }
        reconfigure_kwin();
        std::thread::sleep(std::time::Duration::from_secs(3));
        assert_eq!(bar().hiding, before_bar.hiding);
        assert_eq!(bar().floating, before_bar.floating);
        assert_eq!(button_side(), before_side);
        println!("restored: {:?}", bar());
    }
}
