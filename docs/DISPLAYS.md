# Several displays (Settings > Display)

Page code: `src/pages/display.rs` (cards, switches, mirror) and `src/pages/display_layout.rs` (drag to arrange). Backend: `kscreen-doctor`
(libkscreen), the same tool Plasma uses. The extra controls below exist only on that backend.

## What the page shows

* **One card per connected screen**, including one that is switched off (so it can be switched back on). A screen that is not plugged in is left out.
* **Name**: the laptop's own panel (`eDP-*`, `LVDS-*`, `DSI-*`) is "Built-in display"; others show the monitor's own name read from its EDID
  (`/sys/class/drm/card*-<connector>/edid`, descriptor tag `0xFC`), with the connector (`DP-2`) in small print; no readable name falls back to the connector.
* **Use this display** (only with two or more screens): switches a screen off or on. The last screen that is on cannot be switched off.
* **Main display**: the screen that gets the panel and the first windows (`priority 1` in kscreen). You make another screen main by turning its switch on; the current one stays on.
* **Multiple displays**: Extend (one desktop) or Mirror (every other screen copies the main display). Shown when two or more screens are on.

Every one of these asks "Keep these display settings?" with the 15 second countdown used for resolution; no answer puts the old state back. After the answer the page redraws.

## Commands (checked on a real two-screen KWin session, 2026-10-18)

| Action | Command |
|---|---|
| main display | `kscreen-doctor output.NAME.primary` |
| off / on | `output.NAME.disable` / `output.NAME.enable` |
| mirror | `output.OTHER.mirror.SOURCE` (all in one call) |
| extend | `output.OTHER.mirror.none` |

State comes from `kscreen-doctor -j`: `enabled`, `connected`, `priority` (1 = main), `replicationSource` (id of the screen being copied, 0 = none).

## Testing without touching the real screen

A hidden two-screen desktop: `dbus-run-session -- kwin_wayland --virtual --output-count 2 --socket zt-disp --xwayland --no-lockscreen --exit-with-session ./script.sh`
and `export WAYLAND_DISPLAY=zt-disp` in the script (without `--socket` KWin fails on the owner's `wayland-0` lock and a script would talk to the real session).
The outputs are named `Virtual-0` and `Virtual-1`. Maximise the window for a screenshot with a KWin script (`workspace.windowList()` + `setMaximize`) loaded over D-Bus.

## Not done / not tested

* Clicking the switches (no safe way to send input to the hidden session; the commands themselves were run by hand there).
* A real second monitor, real EDID names (the parser is tested with a built EDID block), 3+ screens.
* Per-screen night light, HDR and variable refresh rate.
