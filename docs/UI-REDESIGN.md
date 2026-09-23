# Zohara OS Premium UI Redesign — status & plan

This is the durable record of an in-progress initiative spanning three
repos: **zohara-settings** (this repo), **zohara-link**, and **zohara-apps**
(the `welcome` binary). It exists so a fresh session — one with no memory of
the conversation that produced it — can pick this up without re-deriving the
decisions below. Update it whenever the plan or status changes; treat stale
sections here as more trustworthy than nothing, but always cross-check
against the actual code before assuming something is still true.

## Why this exists

The goal is for Zohara OS's own apps (Settings, the first-run Welcome
wizard, and the Zohara Link phone-pairing UI) to feel like a premium,
considered desktop OS — closer to macOS System Settings and Windows 11
Settings than to a typical Linux GTK app — without looking AI-generated.
"AI-generated" was called out specifically during design review as: emoji
icons, purple/blue gradient glows, glassmorphism/blur used by default, pill
buttons everywhere. The fix, and the standing design rule for all three
apps, is below.

## Design language (binding for all three apps)

- **Flat fills only. No gradients, anywhere**, in production UI.
- **No backdrop blur / "glass" by default.** A translucent window without
  real compositor blur-behind (which needs a KWin
  `_KDE_NET_WM_BLUR_BEHIND_REGION` window-property integration — not
  implemented anywhere yet) just looks washed out. The "Transparency
  effects" toggle in Personalization exists but **defaults off**.
- **One accent color**, sourced from the OS-level theme config (below) —
  never hardcoded per app, never a gradient of two colors.
- **Icons are flat monoline SVG / GTK symbolic icon names** (e.g.
  `preferences-desktop-color-symbolic`), matching what this app already
  uses. **Never emoji.**
- Structural layout takes cues from macOS (sidebar + grouped card lists);
  visual treatment (mica-style flat surfaces, accent placement) takes cues
  from Windows 11, since the rest of Zohara OS is already Windows-11-styled.

The reviewed visual reference is a private Claude Artifact
(`https://claude.ai/artifact/7Qzvw7XF5XLeLQeP5P6g7W`, owned by the project's
maintainer — not guaranteed accessible to anyone else, hence everything
load-bearing from it is restated here in text) covering four screens:
Settings → Displays, Settings → Power, the Welcome app's Migrate step
(showing the fixed error-banner behavior), and a Zohara Link pairing panel.

## The theming engine (implemented, this repo)

`src/theme.rs` is the OS-level source of truth for accent color, background,
and the transparency toggle:

- Persisted at `~/.config/zohara/theme.json` (`ThemeConfig`: `accent`,
  `background`, `transparency: bool`, `mode: "dark"|"light"|"custom"`).
- Rendered as GTK CSS `@define-color accent_color …; @define-color
  window_bg_color …;` in a provider registered at
  `STYLE_PROVIDER_PRIORITY_APPLICATION + 1` — one priority level above the
  static `data/win11.css`, which declares fallback values for the same two
  named colors so the app still paints correctly before the dynamic
  provider loads.
- `theme::apply(&display)` — call once at startup, registers the provider
  and loads the persisted config.
- `theme::set_and_apply(&cfg)` — call whenever the user changes something in
  Personalization; persists to disk and hot-reloads the *same* provider
  (not a new one), so every open window re-themes live with no restart.
- Personalization's theme preset buttons, the dark/light/custom mode
  `ComboRow`, and the transparency `SwitchRow` are all wired to this.

**Other apps should read `~/.config/zohara/theme.json` at startup** and
apply the same `@define-color` pattern, rather than hardcoding their own
palette — that's what "OS-level theming" means in practice. Neither
`zohara-link` nor `welcome` do this yet (see below).

## Sequencing (explicit, user-specified order — don't reorder without asking)

1. **Settings app** — theming engine + flat redesign + bundle the
   functional bugs found in Display/Power. ✅ **Done**, see Status below.
2. **Zohara Link page, embedded inside Settings** (not a standalone popover
   app) — see "Next up" below.
3. **Welcome app** — last.

## Status

### ✅ Settings app — done, verified on real CI

Commit `29aa289` on `zohara-settings` (pushed to `main`, confirmed green on
the project's GitHub Actions build — this machine has no local GTK4
toolchain, not even `pkg-config`, so CI is the only real verification
available; see the note at the bottom of this file).

- Theming engine (above).
- `data/win11.css` flattened: every gradient removed (avatar circles,
  primary/update buttons, device thumbnails, theme preview, nav accent bar)
  in favor of flat `@accent_color` fills.
- **Display page bug fix** (`src/pages/display.rs`): `parse_outputs()` used
  to return one hardcoded fake `eDP-1` display regardless of what was
  actually connected. Now does real `wlr-randr --json` parsing (exact
  schema) with a best-effort `xrandr --verbose` text parser as the X11
  fallback. Rotation had no click handler at all — now wired to
  `wlr-randr --transform` / `xrandr --rotate`. Scale was hardcoded to
  output name `"eDP-1"` — now uses whatever output was actually detected.
- **Power page bug fix** (`src/pages/power.rs`): screen-off timeout and
  both lid-close-action dropdowns had zero `connect_selected_notify`
  handlers — pure dead UI. Lid actions now write a `logind` drop-in
  (`/etc/systemd/logind.conf.d/99-zohara.conf`, via `pkexec` for
  elevation) and reload `systemd-logind`. Screen timeout uses `xset dpms`
  on X11; **on Wayland it's left disabled with an explicit "not wired up
  yet" subtitle** rather than silently doing nothing — real Wayland idle
  handling needs a compositor daemon (swayidle/hypridle) integration that
  hasn't been built.

### ⏭ Next up: Zohara Link page inside Settings

Not started yet. Plan, per the user's explicit direction (pairing UI lives
*inside* Settings, not as a separate tray/popover app):

- New page module `src/pages/zohara_link.rs` (mirror the shape of
  `display.rs`/`power.rs`), registered in `main.rs`'s `PAGES`/`build_page`
  under the existing "Ecosystem › Zohara Link" sidebar entry.
- UI per the reviewed mockup: pairing-request card (PIN confirm/deny),
  paired-devices list with connection/battery status, quick actions (send
  clipboard, lock screen) — flat design language, same accent system.
- **Real IPC client**, not a mockup: connect to `zohara-linkd`'s Unix
  socket at `/run/user/$UID/zohara.sock` (see
  `zohara-link/linux-daemon/README.md`'s "Unix Socket IPC Integration"
  section — newline-delimited JSON, `GET_STATUS` / `SEND_CLIPBOARD` /
  `LOCK_SCREEN` commands, streaming events). `zohara-link-status` in that
  repo is a working reference client to copy the protocol handling from.
- **"A little bit of verification system"** (the user's phrasing) — the
  page needs to visibly prove it's actually talking to the real daemon,
  not just rendering the mockup: a live connection-status indicator that
  attempts the socket connection at page-build time (and on a refresh
  interval or manual retry), showing a clear "Zohara Link daemon not
  running" state when the socket doesn't exist — same honest-fallback
  pattern as the Display/Power fixes above, not a hardcoded "Connected".
- This page is real GTK+Tokio code that will need the same CI-only
  verification as everything else in this repo.

### ⏭ Last: Welcome app (zohara-apps/welcome)

Not started. Known work:

- Apply the same flat design language (currently has its own minimal
  inline CSS, not the shared theme).
- **Read `~/.config/zohara/theme.json`** at startup for the accent color,
  matching "OS-level theming."
- **Fix the silent-failure bug**: `launch()` in `welcome/src/main.rs`
  treats a successful `Command::spawn()` as success and closes the welcome
  window, even though `zohara-migrate`/`zohara-usermgr` are stubs that spawn
  fine and then immediately `exit(1)`. Needs a real exit-status/stderr check
  and the inline error banner shown in the reviewed mockup ("Migration tool
  isn't ready yet…") instead of silently closing.

## Key decisions worth remembering

- Zohara Link's GUI lives inside Settings, not as a standalone popover/tray
  app — reverse this only if explicitly told to.
- The main `zohara` repo has a second, unrelated `zohara-connectd` Rust
  crate that's a dead skeleton (every subcommand returns "not yet
  implemented") — a duplicate of the now-working `zohara-linkd` in
  `zohara-link`. Flagged for eventual deletion, not yet acted on.
- `zohara` repo's ISO Docker build clones `zohara-settings` live at build
  time (`git clone --depth 1 … main`); this used to risk serving a stale
  cached clone from Docker's layer cache regardless of new pushes — fixed
  in `zohara@6ef9142` via a `ZOHARA_SETTINGS_SHA` build-arg cache-bust. If
  you add a similarly "clone at build time" step for `zohara-link` or
  `zohara-apps` later, it needs the same treatment.

## Known related debt (from an earlier full-ecosystem audit — not part of this initiative, not yet addressed)

- Android app (`zohara-link/app`) trusts any TLS certificate presented by
  the daemon (`TlsSocketEngine.kt`) — needs certificate pinning before use
  over an untrusted network.
- `zohara-hub` doesn't actually watch the `zohara-store` repo despite its
  README claiming it does.
- `zohara-packages`' alpha-channel manual publish workflow is broken (its
  `workflow_dispatch` form has no `run_id` input).
- `welcome`/`migrate`/`usermgr` in `zohara-apps` are honestly-disclosed
  stubs (README says so) — the actual Python-to-Rust port hasn't happened.

## Compile-verification note

This work has been done from a Windows machine with **no local GTK4/
Libadwaita toolchain at all** — confirmed directly: `pkg-config` isn't
installed, so not even `cargo check` runs for this crate locally. Every
change to `zohara-settings` is reviewed by hand, then verified by pushing
and watching the project's real GitHub Actions build (which runs on actual
Arch Linux with the real dependencies). Don't trust a change here as
"working" until that CI run is green — hand-review alone has already caught
real logic bugs, but it cannot catch anything the Rust/GTK type system
would.
