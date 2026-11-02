/*
    SPDX-License-Identifier: GPL-2.0-only OR GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL
*/

import QtQuick
import org.kde.kirigami as Kirigami
import org.kde.kwin as KWinComponents

// One desktop of the side panel with its windows; the whole group is a drop target for a dragged window.
// `desktop` null is the group of windows that are on every desktop.
Item {
    id: group

    // the side panel: its look, its window test and the dragged copy
    property Item owner: null
    property QtObject desktop: null

    readonly property bool current: desktop !== null && effect.selected !== null && effect.selected.id === desktop.id
    property int count: 0

    function recount() {
        let n = 0;
        for (let i = 0; i < rows.count; i++) {
            const it = rows.itemAt(i);
            if (it && it.on) n++;
        }
        group.count = n;
    }

    width: parent ? parent.width : 0
    height: inner.implicitHeight + 8
    visible: desktop !== null || count > 0

    Rectangle {
        anchors.fill: parent
        radius: 10
        color: drop.containsDrag ? "#553b82f6" : (group.current ? group.owner.card : "transparent")
        border.width: drop.containsDrag ? 2 : 0
        border.color: group.owner.accent
    }
    DropArea {
        id: drop
        anchors.fill: parent
        keys: ["zwin"]
        enabled: group.desktop !== null
        onDropped: d => effect.moveWindowTo(d.source.win, group.desktop)
    }

    Column {
        id: inner
        x: 4
        y: 4
        width: parent.width - 8
        spacing: 6

        // the group's title: tap to turn the cube to this desktop
        Item {
            width: parent.width
            height: 28
            Text {
                anchors.left: parent.left
                anchors.leftMargin: 6
                anchors.right: num.left
                anchors.verticalCenter: parent.verticalCenter
                elide: Text.ElideRight
                color: group.current ? group.owner.accent : group.owner.fg
                font.bold: true
                text: group.desktop !== null ? group.desktop.name.replace(/​/g, "") : "On every desktop"
            }
            Text {
                id: num
                anchors.right: parent.right
                anchors.rightMargin: 6
                anchors.verticalCenter: parent.verticalCenter
                color: group.owner.muted
                text: group.count
            }
            TapHandler {
                enabled: group.desktop !== null
                onTapped: effect.chipTapped(group.desktop)
            }
        }

        Text {
            visible: group.count === 0 && group.desktop !== null
            leftPadding: 6
            color: group.owner.muted
            font.pixelSize: 12
            text: "No windows. Drop one here."
        }

        Repeater {
            id: rows
            model: KWinComponents.WindowModel {}
            onItemAdded: Qt.callLater(group.recount)
            onItemRemoved: Qt.callLater(group.recount)
            delegate: Item {
                id: row
                readonly property QtObject win: model.window
                readonly property bool on: group.owner.inGroup(win, group.desktop)
                readonly property bool isActive: KWinComponents.Workspace.activeWindow === win
                readonly property bool previews: group.owner.previews
                onOnChanged: Qt.callLater(group.recount)
                visible: on
                width: inner.width
                height: on ? (previews ? 128 : 40) : 0
                scale: group.owner.hoverFx && hover.hovered ? 1.03 : 1.0
                Behavior on scale { NumberAnimation { duration: 120; easing.type: Easing.OutCubic } }

                Rectangle {
                    anchors.fill: parent
                    radius: 8
                    color: hover.hovered ? group.owner.cardHover : group.owner.card
                    border.width: row.isActive ? 2 : 0
                    border.color: group.owner.accent
                }

                // the window's picture (off in the settings: just its icon and name)
                Loader {
                    x: 8
                    y: 8
                    active: row.on && row.previews
                    sourceComponent: Item {
                        width: row.width - 16
                        height: 80
                        KWinComponents.WindowThumbnail {
                            wId: row.win.internalId
                            readonly property real fit: Math.min((row.width - 16) / Math.max(1, row.win.width), 80 / Math.max(1, row.win.height))
                            width: row.win.width * fit
                            height: row.win.height * fit
                            anchors.centerIn: parent
                        }
                    }
                }

                Kirigami.Icon {
                    x: 10
                    y: row.previews ? 94 : 9
                    width: 22
                    height: 22
                    source: row.win ? row.win.icon : ""
                }
                Text {
                    x: 40
                    y: row.previews ? 94 : 0
                    width: row.width - 40 - 30
                    height: row.previews ? 22 : row.height
                    verticalAlignment: Text.AlignVCenter
                    elide: Text.ElideRight
                    color: group.owner.fg
                    text: row.win ? row.win.caption : ""
                }
                // a hint that the whole row can be dragged
                Text {
                    anchors.right: parent.right
                    anchors.rightMargin: 10
                    y: row.previews ? 94 : 0
                    height: row.previews ? 22 : row.height
                    verticalAlignment: Text.AlignVCenter
                    color: group.owner.muted
                    text: "⋮⋮"
                }

                HoverHandler { id: hover }
                TapHandler {
                    gesturePolicy: TapHandler.DragThreshold
                    onTapped: effect.goToWindow(row.win)
                }
                DragHandler {
                    target: null
                    grabPermissions: PointerHandler.CanTakeOverFromAnything
                    onActiveChanged: {
                        if (active) {
                            group.owner.dragCopy.begin(row.win, centroid.scenePosition);
                        } else {
                            group.owner.dragCopy.finish();
                        }
                    }
                    onCentroidChanged: {
                        if (active) group.owner.dragCopy.follow(centroid.scenePosition);
                    }
                }
            }
        }
    }
}
