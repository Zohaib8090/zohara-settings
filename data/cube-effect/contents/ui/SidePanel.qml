/*
    SPDX-License-Identifier: GPL-2.0-only OR GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL
*/

import QtQuick
import org.kde.kirigami as Kirigami
import org.kde.kwin as KWinComponents

// The overview's side panel: every open window of every desktop, grouped by desktop, down the left edge. Tap a window to go to
// it; drag it onto another desktop's group (or onto a chip in the desktop bar) to move it there. It scrolls when there are many.
// Settings (Personalization > Desktop cube > Side panel): SidePanel, SidePanelPreview, SidePanelWidth, SidePanelOpacity,
// SidePanelStyle (0 dark, 1 light), SidePanelAnimation (it slides in, taking TrayTime), SidePanelHover.
Item {
    id: panel

    // the window layer: the dragged copy of a window lives there, so it can leave the panel and reach the desktop bar
    required property Item host

    readonly property bool wanted: effect.configuration.SidePanel !== false
    readonly property bool previews: effect.configuration.SidePanelPreview !== false
    readonly property bool hoverFx: effect.configuration.SidePanelHover !== false
    readonly property bool animate: effect.configuration.SidePanelAnimation !== false
    readonly property bool light: effect.configuration.SidePanelStyle === 1
    readonly property real alpha: Math.max(0.3, Math.min(1, effect.configuration.SidePanelOpacity !== undefined ? effect.configuration.SidePanelOpacity : 0.85))

    readonly property color fg: light ? "#1b1d26" : "#ffffff"
    readonly property color muted: light ? "#5a5f70" : "#b8bccb"
    readonly property color card: light ? "#12000000" : "#1affffff"
    readonly property color cardHover: light ? "#26000000" : "#33ffffff"
    readonly property color accent: "#3b82f6"

    width: Math.max(200, Math.min(420, effect.configuration.SidePanelWidth || 280))
    visible: wanted && effect.amount > 0.02

    // 0 hidden .. 1 shown; it glides in from the left with the rest of the layer
    property real q: effect.overview ? 1 : 0
    Behavior on q {
        enabled: panel.animate
        NumberAnimation { duration: Math.max(100, effect.configuration.TrayTime || 350); easing.type: Easing.OutCubic }
    }
    opacity: q
    transform: Translate { x: -(1 - panel.q) * (panel.width + 20) }

    // an application window: not a panel, the wallpaper, a popup or a dialog
    function isApp(w) {
        return w && w.normalWindow && !w.dock && !w.desktopWindow && !w.dialog && !w.transient && !w.skipTaskbar;
    }

    // is this window listed under desktop `d` (null: the group of windows that are on every desktop)?
    function inGroup(w, d) {
        if (!isApp(w)) return false;
        if (d === null) return w.onAllDesktops;
        if (w.onAllDesktops) return false;
        for (let i = 0; i < w.desktops.length; i++) {
            if (w.desktops[i].id === d.id) return true;
        }
        return false;
    }

    // how many windows are open, for the heading
    property int total: 0
    function recountTotal() {
        let n = 0;
        for (let i = 0; i < allWindows.count; i++) {
            const it = allWindows.itemAt(i);
            if (it && it.app) n++;
        }
        panel.total = n;
    }
    Repeater {
        id: allWindows
        model: KWinComponents.WindowModel {}
        delegate: Item {
            readonly property bool app: panel.isApp(model.window)
            onAppChanged: Qt.callLater(panel.recountTotal)
        }
        onItemAdded: Qt.callLater(panel.recountTotal)
        onItemRemoved: Qt.callLater(panel.recountTotal)
    }

    Rectangle {
        id: back
        anchors.fill: parent
        radius: 12
        color: panel.light ? Qt.rgba(0.96, 0.96, 0.98, panel.alpha) : Qt.rgba(0.063, 0.07, 0.094, panel.alpha)
        border.width: 1
        border.color: panel.light ? "#22000000" : "#33ffffff"
    }

    Column {
        id: head
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.margins: 14
        spacing: 2
        Text {
            width: parent.width
            elide: Text.ElideRight
            color: panel.fg
            font.bold: true
            font.pixelSize: 15
            text: panel.total === 1 ? "1 open window" : panel.total + " open windows"
        }
        Text {
            width: parent.width
            wrapMode: Text.WordWrap
            color: panel.muted
            font.pixelSize: 12
            text: "Tap to go there. Drag onto a desktop to move it."
        }
    }

    Flickable {
        id: scroller
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: head.bottom
        anchors.bottom: parent.bottom
        anchors.topMargin: 8
        anchors.bottomMargin: 8
        anchors.leftMargin: 8
        anchors.rightMargin: 8
        clip: true
        contentWidth: width
        contentHeight: groups.implicitHeight
        boundsBehavior: Flickable.StopAtBounds

        Column {
            id: groups
            width: scroller.width
            spacing: 6

            Repeater {
                model: KWinComponents.VirtualDesktopModel {}
                delegate: Item {
                    id: slot
                    required property QtObject desktop
                    width: groups.width
                    height: oneGroup.height
                    DesktopGroup { id: oneGroup; owner: panel; desktop: slot.desktop }
                }
            }
            DesktopGroup { owner: panel; desktop: null }
        }
    }

    // the window copy, for the groups to hand to
    readonly property Item dragCopy: ghost

    // the copy of a window that follows the pointer while it is dragged; it sits in the window layer so it is not cut off by the
    // panel, and the desktop chips and groups (drop areas with the key "zwin") see it as the dragged item
    Rectangle {
        id: ghost
        parent: panel.host
        property QtObject win: null
        visible: win !== null
        z: 5000
        width: 220
        height: 44
        radius: 10
        color: panel.light ? "#f2f4f8" : "#e61b1d26"
        border.width: 2
        border.color: panel.accent
        Drag.active: ghost.win !== null
        Drag.keys: ["zwin"]
        Drag.hotSpot.x: width / 2
        Drag.hotSpot.y: height / 2

        Kirigami.Icon {
            x: 10
            anchors.verticalCenter: parent.verticalCenter
            width: 24
            height: 24
            source: ghost.win ? ghost.win.icon : ""
        }
        Text {
            x: 42
            width: parent.width - 52
            anchors.verticalCenter: parent.verticalCenter
            elide: Text.ElideRight
            color: panel.light ? "#1b1d26" : "white"
            text: ghost.win ? ghost.win.caption : ""
        }

        function follow(pt) {
            const p = panel.host.mapFromItem(null, pt.x, pt.y);
            ghost.x = p.x - ghost.width / 2;
            ghost.y = p.y - ghost.height / 2;
        }
        function begin(w, pt) {
            follow(pt);
            ghost.win = w;
        }
        // let go: the area under it (a desktop chip or a group) takes the window, then the copy goes away
        function finish() {
            if (ghost.win === null) return;
            ghost.Drag.drop();
            ghost.win = null;
        }
    }
}
