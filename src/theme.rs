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

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeConfig {
    pub accent: String,
    pub background: String,
    pub transparency: bool,
    pub mode: String, // "dark" | "light"
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            accent: "#4c8dff".to_string(),
            background: "#1a1a1e".to_string(),
            transparency: false,
            mode: "dark".to_string(),
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

/// Renders the theme as `@define-color` overrides plus the transparency
/// toggle. Named colors defined here win over the fallback values declared
/// in win11.css because this provider is registered at a higher priority.
///
/// Note: without transparency, `window_bg_color` is fully opaque. With it
/// on, the alpha channel is lowered — that gives a translucent window, not
/// real compositor blur-behind (Acrylic/Mica). True blur-behind needs a
/// KWin `_KDE_NET_WM_BLUR_BEHIND_REGION` window-property integration, which
/// is out of scope here; this is why the toggle defaults off, since a
/// translucent-but-unblurred window looks worse than a flat one.
fn to_css(cfg: &ThemeConfig) -> String {
    let alpha: f32 = if cfg.transparency { 0.82 } else { 1.0 };
    format!(
        "@define-color accent_color {accent};\n\
         @define-color window_bg_color {bg};\n\
         window, .win11-window {{\n\
         \x20   background-color: alpha(@window_bg_color, {alpha});\n\
         }}\n",
        accent = cfg.accent,
        bg = cfg.background,
        alpha = alpha,
    )
}

thread_local! {
    static PROVIDER: gtk4::CssProvider = gtk4::CssProvider::new();
}

/// Call once at startup: registers the dynamic-theme provider on the given
/// display and loads the persisted config into it. Returns the config so
/// callers (Personalization) can seed their controls from it.
pub fn apply(display: &gtk4::gdk::Display) -> ThemeConfig {
    let cfg = load();
    PROVIDER.with(|p| {
        p.load_from_string(&to_css(&cfg));
        gtk4::style_context_add_provider_for_display(
            display,
            p,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
        );
    });
    cfg
}

/// Persists `cfg` and re-renders the already-registered provider so every
/// open window picks up the change immediately.
pub fn set_and_apply(cfg: &ThemeConfig) {
    save(cfg);
    PROVIDER.with(|p| p.load_from_string(&to_css(cfg)));
}
