/*
    SPDX-License-Identifier: GPL-2.0-only OR GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL
*/

import QtQuick
import org.kde.kirigami as Kirigami
import org.kde.kwin as KWinComponents

// The overview's window layer: a bar of desktops along the top (drop a window on one to move it there, or on "New desktop"),
// and a tray along the bottom with the windows of the picked desktop, or every window of one app when you pressed its icon.
Item {
    id: windowLayer

    required property QtObject targetScreen
    // The layer follows the cube's own zoom-out (effect.amount, 0 closed .. 1 open, which also follows your fingers on a 4 finger
    // swipe): the desktop bar glides down from the top and the tray up from the bottom, fading in a little after the cube starts.
    readonly property real p: Math.max(0, Math.min(1, (effect.amount - 0.08) / 0.92))
    readonly property bool trayShown: effect.amount > 0.02

    // an application window: not a panel, the wallpaper, a popup or a dialog
    function isApp(w) {
        return w && w.normalWindow && !w.dock && !w.desktopWindow && !w.dialog && !w.transient && !w.skipTaskbar;
    }

    // is this window one of the tray's windows right now?
    function shown(w) {
        if (!isApp(w)) return false;
        if (effect.appFilter !== "") return String(w.resourceClass || "") === effect.appFilter;
        if (effect.selected === null || w.onAllDesktops) return false;
        for (let i = 0; i < w.desktops.length; i++) {
            if (w.desktops[i].id === effect.selected.id) return true;
        }
        return false;
    }

    // ── desktops along the top: drop targets ──
    Row {
        id: bar
        objectName: "desktopBar"
        visible: windowLayer.trayShown
        opacity: windowLayer.p
        transform: Translate { y: -(1 - windowLayer.p) * 70 }
        anchors.top: parent.top
        anchors.topMargin: Kirigami.Units.largeSpacing
        anchors.horizontalCenter: parent.horizontalCenter
        spacing: Kirigami.Units.smallSpacing

        Repeater {
            model: KWinComponents.VirtualDesktopModel {}
            delegate: Rectangle {
                id: chip
                objectName: "chip_" + desktop.id
                required property QtObject desktop
                required property int index
                width: Math.max(110, label.implicitWidth + 28) + (effect.selected !== null && effect.selected.id === desktop.id ? 30 : 0)
                height: 40
                radius: 8
                color: drop.containsDrag ? "#cc3b82f6" : (effect.selected !== null && effect.selected.id === desktop.id ? "#cc2a2d38" : "#99181a22")
                border.width: effect.selected !== null && effect.selected.id === desktop.id ? 2 : 0
                border.color: "#ffffff"
                Text {
                    id: label
                    anchors.centerIn: parent
                    text: chip.desktop.name.replace(/​/g, "")
                    color: "white"
                    elide: Text.ElideRight
                }
                DropArea {
                    id: drop
                    anchors.fill: parent
                    keys: ["zwin"]
                    onDropped: d => effect.moveWindowTo(d.source.win, chip.desktop)
                }
                TapHandler {
                    id: chipTap
                    property real lastTap: 0
                    onTapped: {
                        // two taps close together open the desktop's menu; a single tap picks the desktop
                        const now = Date.now();
                        if (now - chipTap.lastTap < 450) {
                            chipTap.lastTap = 0;
                            effect.openDesktopMenu(chip.desktop);
                        } else {
                            chipTap.lastTap = now;
                            effect.chipTapped(chip.desktop);
                        }
                    }
                }
                // the same menu from a button, for the picked desktop
                Rectangle {
                    visible: effect.selected !== null && effect.selected.id === chip.desktop.id
                    objectName: "chipMore_" + chip.desktop.id
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.rightMargin: 6
                    width: 26
                    height: 26
                    radius: 13
                    color: "#55ffffff"
                    Text { anchors.centerIn: parent; text: "\u22ef"; color: "white" }
                    TapHandler { onTapped: effect.openDesktopMenu(chip.desktop) }
                }
            }
        }

        Rectangle {
            objectName: "newChip"
            width: 150
            height: 40
            radius: 8
            color: newDrop.containsDrag ? "#cc22a55a" : "#99181a22"
            border.width: 1
            border.color: "#88ffffff"
            Text {
                anchors.centerIn: parent
                text: "+ New desktop"
                color: "white"
            }
            DropArea {
                id: newDrop
                anchors.fill: parent
                keys: ["zwin"]
                onDropped: d => effect.moveWindowToNew(d.source.win)
            }
            // a click makes an empty desktop; dropping a window on it makes one with that window
            TapHandler { onTapped: effect.newChipTapped() }
        }
    }

    // ── the menu of one desktop: move it, or delete it ──
    Rectangle {
        id: menu
        objectName: "desktopMenu"
        visible: windowLayer.trayShown && effect.menuDesktop !== null
        readonly property int idx: effect.menuDesktop !== null ? effect.indexOf(effect.menuDesktop) : -1
        readonly property int count: effect.desktopCount
        anchors.top: bar.bottom
        anchors.topMargin: 10
        anchors.horizontalCenter: parent.horizontalCenter
        width: 330
        height: col.implicitHeight + 24
        radius: 12
        color: "#f0181a22"
        border.width: 1
        border.color: "#88ffffff"
        z: 2000

        Column {
            id: col
            anchors.fill: parent
            anchors.margins: 12
            spacing: 8

            Text {
                color: "white"
                font.bold: true
                text: effect.menuDesktop !== null ? effect.menuDesktop.name.replace(/\u200b/g, "") + "  (desktop " + (menu.idx + 1) + " of " + menu.count + ")" : ""
            }

            // one action: a labelled button that does nothing when it is not available
            component MenuAction: Rectangle {
                id: action
                property alias label: title.text
                // a smaller second line under the label; it wraps inside the button
                property string hint: ""
                property bool available: true
                property color tint: "#33ffffff"
                signal activated()
                width: parent.width
                height: Math.max(38, content.implicitHeight + 18)
                radius: 8
                color: available ? tint : "#1affffff"
                Column {
                    id: content
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.leftMargin: 12
                    anchors.rightMargin: 12
                    spacing: 2
                    Text {
                        id: title
                        width: parent.width
                        elide: Text.ElideRight
                        color: action.available ? "white" : "#77ffffff"
                    }
                    Text {
                        visible: action.hint !== ""
                        width: parent.width
                        wrapMode: Text.WordWrap
                        font.pixelSize: 12
                        color: action.available ? "#ddffffff" : "#55ffffff"
                        text: action.hint
                    }
                }
                // always taking the tap, even when the action is not available, so it never falls through to the cube underneath
                TapHandler { onTapped: { if (action.available) action.activated(); } }
            }

            MenuAction {
                objectName: "menuLeft"
                label: "\u25c0  Move left"
                available: menu.idx > 0
                onActivated: effect.moveDesktop(effect.menuDesktop, menu.idx - 1)
            }
            MenuAction {
                objectName: "menuRight"
                label: "Move right  \u25b6"
                available: menu.idx >= 0 && menu.idx < menu.count - 1
                onActivated: effect.moveDesktop(effect.menuDesktop, menu.idx + 1)
            }
            MenuAction {
                objectName: "menuFirst"
                label: "\u23ee  Move to the first place"
                available: menu.idx > 0
                onActivated: effect.moveDesktop(effect.menuDesktop, 0)
            }
            MenuAction {
                objectName: "menuDelete"
                label: "Delete this desktop"
                hint: "Its windows move to another desktop. Nothing is closed."
                available: menu.count > 1
                tint: "#99b91c1c"
                onActivated: effect.deleteDesktop(effect.menuDesktop)
            }
            MenuAction {
                objectName: "menuClose"
                label: "Close"
                onActivated: effect.closeDesktopMenu()
            }
        }
    }

    // ── windows along the bottom ──
    Rectangle {
        id: tray
        objectName: "windowTray"
        visible: windowLayer.trayShown
        opacity: windowLayer.p
        transform: Translate { y: (1 - windowLayer.p) * tray.height }
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        height: Math.min(windowLayer.height * 0.38, 420)
        color: "#d9101218"

        Text {
            id: heading
            anchors.left: parent.left
            anchors.top: parent.top
            anchors.margins: Kirigami.Units.largeSpacing
            color: "white"
            font.bold: true
            text: effect.appFilter !== ""
                  ? "All windows of " + effect.appFilter
                  : (effect.selected !== null ? effect.selected.name.replace(/​/g, "") : "")
        }
        Text {
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.margins: Kirigami.Units.largeSpacing
            color: "#bbbbbb"
            text: effect.armed !== null
                  ? "Tap a desktop above to move \u201c" + effect.armed.caption + "\u201d there (or New desktop)"
                  : (effect.appFilter !== "" ? "Tap the app icon again to go back to this desktop" : "Tap a window to go to it. Tap Move, then a desktop above, to move it. Or drag it")
        }

        Flow {
            id: flow
            anchors.fill: parent
            anchors.topMargin: 48
            anchors.leftMargin: Kirigami.Units.largeSpacing
            anchors.rightMargin: Kirigami.Units.largeSpacing
            spacing: Kirigami.Units.largeSpacing

            Repeater {
                model: KWinComponents.WindowModel {}
                delegate: Item {
                    id: slot
                    readonly property QtObject win: model.window
                    readonly property bool on: windowLayer.shown(win)
                    visible: on
                    width: on ? 220 : 0
                    height: on ? 170 : 0

                    Item {
                        id: card
                    objectName: "card_" + String(slot.win.resourceClass)
                        width: 220
                        height: 170
                        property QtObject win: slot.win
                        z: dragHandler.active ? 1000 : 0
                        Drag.active: dragHandler.active
                        Drag.keys: ["zwin"]
                        Drag.hotSpot.x: width / 2
                        Drag.hotSpot.y: height / 2

                        Rectangle {
                            anchors.fill: parent
                            radius: 10
                            color: "#99202330"
                            border.width: (dragHandler.active || effect.armed === slot.win) ? 2 : 0
                            border.color: effect.armed === slot.win ? "#22c55e" : "#3b82f6"
                        }
                        KWinComponents.WindowThumbnail {
                            id: thumb
                            wId: slot.win.internalId
                            readonly property real scaleToFit: Math.min(200 / Math.max(1, slot.win.width), 112 / Math.max(1, slot.win.height))
                            width: slot.win.width * scaleToFit
                            height: slot.win.height * scaleToFit
                            x: (card.width - width) / 2
                            y: 10 + (112 - height) / 2
                        }
                        // the app's icon: press it to see every window of that app
                        Rectangle {
                            id: appButton
                            x: 8
                            y: card.height - 44
                            width: 36
                            height: 36
                            radius: 8
                            color: effect.appFilter !== "" && effect.appFilter === String(slot.win.resourceClass || "") ? "#cc3b82f6" : "#66ffffff"
                            Kirigami.Icon {
                                anchors.fill: parent
                                anchors.margins: 4
                                source: slot.win.icon
                            }
                            TapHandler {
                                onTapped: effect.toggleAppFilter(String(slot.win.resourceClass || ""))
                            }
                        }
                        // Move: pick this window up, then tap a desktop at the top
                        Rectangle {
                            id: moveButton
                            objectName: "move_" + String(slot.win.resourceClass)
                            x: card.width - 70
                            y: 8
                            width: 62
                            height: 28
                            radius: 8
                            color: effect.armed === slot.win ? "#cc22a55a" : "#99202330"
                            border.width: 1
                            border.color: "#88ffffff"
                            Text {
                                anchors.centerIn: parent
                                color: "white"
                                text: effect.armed === slot.win ? "Cancel" : "Move"
                            }
                            TapHandler { onTapped: effect.toggleArm(slot.win) }
                        }
                        Text {
                            x: 52
                            y: card.height - 40
                            width: card.width - 60
                            color: "white"
                            elide: Text.ElideRight
                            text: slot.win.caption
                        }
                        TapHandler {
                            // a press on the icon is handled by the icon; anywhere else on the card goes to the window
                            gesturePolicy: TapHandler.DragThreshold
                            onTapped: eventPoint => {
                                if (!appButton.contains(appButton.mapFromItem(card, eventPoint.position.x, eventPoint.position.y))
                                        && !moveButton.contains(moveButton.mapFromItem(card, eventPoint.position.x, eventPoint.position.y))) {
                                    effect.goToWindow(slot.win);
                                }
                            }
                        }
                        DragHandler {
                            id: dragHandler
                            target: card
                            onActiveChanged: {
                                if (!active) {
                                    card.Drag.drop();
                                    card.x = 0;
                                    card.y = 0;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
