//! The OS-level theming engine.
//!
//! The Personalization page is the single source of truth for accent color,
//! background and transparency. It persists a `ThemeConfig` to
//! `~/.config/zohara/theme.json` and calls `set_and_apply()`, which re-renders
//! a small GTK CSS snippet (`@define-color` overrides) into a provider that's
//! already registered on the display — so every open window re-themes live,
//! with no restart.
//!
//! Other Zohara apps (welcome, and eventually the Zohara Link panel) read the
//! same `theme.json` at startup so the whole OS shares one accent instead of
//! each app hardcoding its own.

use libadwaita as adw;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeConfig {
    pub accent: String,
    pub background: String,
    pub transparency: bool,
    pub mode: String, // "system" (follow the OS, the default) | "dark" | "light" | "custom"
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            accent: "#4c8dff".to_string(),
            background: "#1a1a1e".to_string(),
            transparency: false,
            mode: "system".to_string(),
        }
    }
}

pub fn config_path() -> PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
            PathBuf::from(home).join(".config")
        });
    base.join("zohara").join("theme.json")
}

pub fn load() -> ThemeConfig {
    std::fs::read_to_string(config_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save(cfg: &ThemeConfig) {
    let path = config_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(s) = serde_json::to_string_pretty(cfg) {
        let _ = std::fs::write(&path, s);
    }
}

/// Parses `#rgb` or `#rrggbb` into 0–255 components. Anything else (a bad
/// value in a hand-edited `theme.json`) falls back to opaque black, which
/// `foreground_for` then reads as "needs a light foreground" — the safe
/// direction, since Zohara defaults to dark surfaces.
fn parse_hex(s: &str) -> (u8, u8, u8) {
    let h = s.trim_start_matches('#');
    let expand = |c: char| u8::from_str_radix(&c.to_string().repeat(2), 16).unwrap_or(0);
    let byte = |i: usize| u8::from_str_radix(h.get(i..i + 2).unwrap_or("00"), 16).unwrap_or(0);
    match h.len() {
        3 => (expand(h.as_bytes()[0] as char), expand(h.as_bytes()[1] as char), expand(h.as_bytes()[2] as char)),
        _ => (byte(0), byte(2), byte(4)),
    }
}

/// The text/overlay color that reads clearly on `background`. Every surface
/// color in win11.css (card fills, borders, dividers, secondary text) is an
/// `alpha(@window_fg_color, …)` overlay of this, so switching to a light
/// background — the "Light" mode, or a light theme preset such as "Windows
/// 11 Light" — doesn't leave white text on a white card.
fn foreground_for(background: &str) -> &'static str {
    let (r, g, b) = parse_hex(background);
    let luminance = 0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32;
    if luminance > 140.0 {
        "#16161a" // dark text/overlay for a light background
    } else {
        "#ffffff" // light text/overlay for a dark background
    }
}

const DEFAULT_DARK_BG: &str = "#1a1a1e";
const DEFAULT_LIGHT_BG: &str = "#f3f3f6";

/// The colours the whole window is painted with.
struct Palette {
    bg: String,
    fg: &'static str,
    /// True when `bg` is dark. Decides whether libadwaita's own palette is told to be dark or light.
    dark: bool,
}

/// Works out the palette. `system_dark` is what the OS (KDE, through the desktop portal) currently prefers; it
/// only matters in "system" mode. A background picked on Personalization (anything but the default dark one)
/// wins over the light/dark default so presets keep working.
fn resolve(cfg: &ThemeConfig, system_dark: bool) -> Palette {
    let picked = cfg.background != DEFAULT_DARK_BG;
    let bg = match cfg.mode.as_str() {
        "custom" => cfg.background.clone(),
        "light" if !picked => DEFAULT_LIGHT_BG.to_string(),
        "dark" => cfg.background.clone(),
        "light" => cfg.background.clone(),
        _ if picked => cfg.background.clone(),
        _ if system_dark => DEFAULT_DARK_BG.to_string(),
        _ => DEFAULT_LIGHT_BG.to_string(),
    };
    let fg = foreground_for(&bg);
    Palette { dark: fg == "#ffffff", bg, fg }
}

/// `a` moved `t` (0..1) of the way towards `b`, as `#rrggbb`.
fn mix_hex(a: &str, b: &str, t: f32) -> String {
    let (ar, ag, ab) = parse_hex(a);
    let (br, bg, bb) = parse_hex(b);
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round().clamp(0.0, 255.0) as u8;
    format!("#{:02x}{:02x}{:02x}", m(ar, br), m(ag, bg), m(ab, bb))
}

/// Renders the theme as `@define-color` overrides plus the transparency
/// toggle. Named colors defined here win over the fallback values declared
/// in win11.css because this provider is registered at a higher priority.
///
/// libadwaita draws most of its own widgets (row text and icons, the window buttons in the header bar, cards,
/// popovers, expander content) from its own CSS variables (`--window-fg-color` and friends), which
/// `@define-color` does not reach. They are set here too, from the same palette, so those widgets can never end
/// up black on a dark window or white on a light one, whatever palette libadwaita picked on its own.
///
/// Note: without transparency, `window_bg_color` is fully opaque. With it
/// on, the alpha channel is lowered — that gives a translucent window, not
/// real compositor blur-behind (Acrylic/Mica). True blur-behind needs a
/// KWin `_KDE_NET_WM_BLUR_BEHIND_REGION` window-property integration, which
/// is out of scope here; this is why the toggle defaults off, since a
/// translucent-but-unblurred window looks worse than a flat one.
fn to_css(cfg: &ThemeConfig, system_dark: bool) -> String {
    let alpha: f32 = if cfg.transparency { 0.82 } else { 1.0 };
    let p = resolve(cfg, system_dark);
    let raised = mix_hex(&p.bg, p.fg, if p.dark { 0.07 } else { 0.0 }); // popovers and dialogs sit a little above the window
    let card = mix_hex(&p.bg, p.fg, if p.dark { 0.05 } else { 0.0 });
    format!(
        "@define-color accent_color {accent};\n\
         @define-color window_bg_color {bg};\n\
         @define-color window_fg_color {fg};\n\
         window, .win11-window {{\n\
         \x20   background-color: alpha(@window_bg_color, {alpha});\n\
         \x20   --window-bg-color: {bg}; --window-fg-color: {fg};\n\
         \x20   --view-bg-color: {bg}; --view-fg-color: {fg};\n\
         \x20   --headerbar-bg-color: {bg}; --headerbar-fg-color: {fg}; --headerbar-backdrop-color: {bg};\n\
         \x20   --card-bg-color: {card}; --card-fg-color: {fg};\n\
         \x20   --popover-bg-color: {raised}; --popover-fg-color: {fg};\n\
         \x20   --dialog-bg-color: {raised}; --dialog-fg-color: {fg};\n\
         \x20   --sidebar-bg-color: {bg}; --sidebar-fg-color: {fg};\n\
         }}\n",
        accent = cfg.accent,
        bg = p.bg,
        fg = p.fg,
        card = card,
        raised = raised,
        alpha = alpha,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_background_gets_dark_foreground() {
        assert_eq!(foreground_for("#e0e7ff"), "#16161a"); // the "Windows 11 Light" preset
        assert_eq!(foreground_for("#ffffff"), "#16161a");
    }

    #[test]
    fn dark_background_gets_light_foreground() {
        assert_eq!(foreground_for("#1a1a1e"), "#ffffff"); // the default
        assert_eq!(foreground_for("#000000"), "#ffffff");
        assert_eq!(foreground_for("#0a0e17"), "#ffffff"); // "Deep Nebula" preset
    }

    #[test]
    fn system_mode_follows_the_os() {
        let cfg = ThemeConfig::default();
        assert_eq!(cfg.mode, "system");
        let dark = resolve(&cfg, true);
        assert!(dark.dark && dark.fg == "#ffffff");
        let light = resolve(&cfg, false);
        assert!(!light.dark && light.fg == "#16161a" && light.bg == DEFAULT_LIGHT_BG);
    }

    #[test]
    fn picked_background_wins_over_the_system() {
        let cfg = ThemeConfig { background: "#0a0e17".into(), ..ThemeConfig::default() };
        assert!(resolve(&cfg, false).dark);
    }

    #[test]
    fn explicit_modes_ignore_the_os() {
        let dark = ThemeConfig { mode: "dark".into(), ..ThemeConfig::default() };
        assert!(resolve(&dark, false).dark);
        let light = ThemeConfig { mode: "light".into(), ..ThemeConfig::default() };
        assert!(!resolve(&light, true).dark);
    }

    #[test]
    fn css_sets_libadwaita_variables_in_both_modes() {
        for sys_dark in [true, false] {
            let css = to_css(&ThemeConfig::default(), sys_dark);
            let fg = if sys_dark { "#ffffff" } else { "#16161a" };
            assert!(css.contains(&format!("--window-fg-color: {fg}")));
            assert!(css.contains(&format!("--headerbar-fg-color: {fg}")));
            assert!(css.contains(&format!("--card-fg-color: {fg}")));
        }
    }

    #[test]
    fn mix_moves_between_colours() {
        assert_eq!(mix_hex("#000000", "#ffffff", 0.0), "#000000");
        assert_eq!(mix_hex("#000000", "#ffffff", 1.0), "#ffffff");
    }

    #[test]
    fn short_hex_and_garbage_dont_panic() {
        assert_eq!(foreground_for("#fff"), "#16161a");
        assert_eq!(foreground_for("not-a-color"), "#ffffff");
    }
}

thread_local! {
    static PROVIDER: gtk4::CssProvider = gtk4::CssProvider::new();
}

/// What the OS prefers right now (needs `apply_native_color_scheme` to have run with `Default`).
fn system_is_dark() -> bool {
    adw::StyleManager::default().is_dark()
}

/// libadwaita draws its own widgets from its own palette. In "system" mode it is left to follow the OS; in the
/// other modes it is forced to match the colours we paint, so the two never disagree (that was the black text
/// and the white expander box seen on real hardware).
fn apply_native_color_scheme(cfg: &ThemeConfig) {
    let scheme = if cfg.mode == "system" && cfg.background == DEFAULT_DARK_BG {
        adw::ColorScheme::Default
    } else if resolve(cfg, true).dark {
        adw::ColorScheme::ForceDark
    } else {
        adw::ColorScheme::ForceLight
    };
    adw::StyleManager::default().set_color_scheme(scheme);
}

fn render_now(cfg: &ThemeConfig) {
    PROVIDER.with(|p| p.load_from_string(&to_css(cfg, system_is_dark())));
}

/// Call once at startup: registers the dynamic-theme provider on the given
/// display and loads the persisted config into it. Returns the config so
/// callers (Personalization) can seed their controls from it.
pub fn apply(display: &gtk4::gdk::Display) -> ThemeConfig {
    let cfg = load();
    apply_native_color_scheme(&cfg);
    render_now(&cfg);
    PROVIDER.with(|p| {
        gtk4::style_context_add_provider_for_display(
            display,
            p,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
        );
    });
    // The OS switching between light and dark while the app is open: repaint to match.
    adw::StyleManager::default().connect_dark_notify(|_| render_now(&load()));
    cfg
}

/// Persists `cfg` and re-renders the already-registered provider so every
/// open window picks up the change immediately.
pub fn set_and_apply(cfg: &ThemeConfig) {
    save(cfg);
    apply_native_color_scheme(cfg);
    render_now(cfg);
}
