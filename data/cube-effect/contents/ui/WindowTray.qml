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
    readonly property bool trayShown: effect.overview && (effect.selected !== null || effect.appFilter !== "")

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
        visible: windowLayer.trayShown
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
                width: Math.max(110, label.implicitWidth + 28)
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
                TapHandler { onTapped: effect.selectDesktop(chip.desktop) }
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
            TapHandler { onTapped: effect.newDesktop() }
        }
    }

    // ── windows along the bottom ──
    Rectangle {
        id: tray
        visible: windowLayer.trayShown
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
            text: effect.appFilter !== "" ? "Click the app icon again to go back to this desktop" : "Drag a window to a desktop above. Click a window to go to it. Click the desktop again to open it"
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
                            border.width: dragHandler.active ? 2 : 0
                            border.color: "#3b82f6"
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
                                if (!appButton.contains(appButton.mapFromItem(card, eventPoint.position.x, eventPoint.position.y))) {
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
