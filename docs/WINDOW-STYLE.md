# Round window buttons (the Zohara window decoration)

What: title-bar buttons drawn as soft round buttons with a dash, a square and a cross (like Zohara's own apps), a red circle for Close on hover, colours
from the colour scheme (light and dark). It is a KDE QML decoration package: `data/kwin/decorations/kwin4_decoration_qml_zohara/`
(`metadata.json`, `contents/ui/main.qml` the title bar, `ZoharaButton.qml` one button drawn with a Canvas), installed by the PKGBUILD to
`/usr/share/kwin/decorations/`.

Switch: Settings > Personalization > Colors > **Round window buttons** (`pages/personalization.rs`, `backend/window_style.rs`). On: `kwinrc`
`[org.kde.kdecoration2] library=org.kde.kwin.aurorae, theme=kwin4_decoration_qml_zohara`; the style that was there is kept in `kwinrc [Zohara]` and
given back when it is switched off (Breeze if unknown). Off by default; applies to every window.

Verified in the test VM (screenshots, light and dark scheme, hover, maximized/restore icon, switch on and off). Not yet seen on the owner's laptop.

Traps (all found while building it):
* A QML decoration is named by its **plain id** in `kwinrc`. `__aurorae__qml__<id>` gives "Couldn't find QML Decoration".
* KWin keeps a decoration's QML in memory: after editing the files, switching themes does **not** reload it. While developing, install under a new id
  each time (`kwin4_decoration_qml_zt1`, `zt2`, …); users get the change at the next sign-in.
* A `ButtonGroup` has no height of its own: set `height` on it, or `verticalCenter` puts the buttons half a bar too low.
* Not done: shadows round the window (QML decorations cannot draw them), rounded bottom corners, a size option, being the default on a fresh install.
