# Sidebar structure and theming (2026-09)

How the Settings sidebar is organised and why, plus the theming and status
bugs fixed alongside it. Code: `src/main.rs`, `src/theme.rs`,
`src/pages/system.rs`, `src/pages/home.rs`, `data/win11.css`.

## Sidebar

The sidebar follows the Windows 11 grouping, one row per top-level area:

Home, System, Bluetooth & devices, Network & internet, Personalization, Apps,
Accounts, Time & language, Gaming, Accessibility, Privacy & security,
Zohara Update, About. **Zohara Link** and **Troubleshoot** are kept as their own
rows below a divider (`win11-nav-separator`, set with `ListBox::set_header_func`
before "Zohara Link").

Implementation notes:

- `PAGES` is a static array of `PageDef { label, icon, hidden }`. Routing is
  **index-based** (`build_page_inner` matches on position), so the 25 entries keep
  their original order. Sub-pages that used to clutter the sidebar (Display, Sound,
  Notifications, Power & battery, Storage, Mouse, Touchpad, Keyboard, Printers,
  Default apps) are `hidden: true`: the nav loop skips them but they still exist.
- Hidden pages stay reachable. `system_links_group` (in `pages/system.rs`) adds
  Display, Sound, Notifications, Power & battery and Storage rows to the System page,
  which call the `win.goto` action. `goto` selects the sidebar row whose widget name
  is the page index, or, for a hidden page, builds and shows the stack page directly
  and clears the selection.
- Icon accent colour is chosen by label: Home and Personalization orange, Accounts
  and Zohara Link green, Gaming purple, Zohara Update cyan, Troubleshoot orange,
  everything else blue.

## Dark mode

White rows and a wrong titlebar appeared in dark mode because `@define-color`
only affects our own `.win11-*` classes. Native surfaces (ExpanderRow content,
headerbar, popovers) follow libadwaita's colour scheme. `theme::apply_native_color_scheme`
now sets `adw::StyleManager::set_color_scheme` (PreferLight for "light", otherwise
PreferDark), and is called from both `apply()` and `set_and_apply()`.

## Home page network badge

The Wi-Fi tile said "Online" when nothing was connected. The else-branch subtitle in
`pages/home.rs` is now "Not connected".

## Not yet verified

Seen and fixed from photos on a Dell Latitude live session, not re-checked on an
installed system in the VM (only its login screen has been seen). See
`zohara/docs/HANDOFF-2026-09-29.md` for the VM rig used to check.
