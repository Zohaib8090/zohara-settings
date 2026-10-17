/*
    The Zohara window decoration: a title bar with round buttons (dash, square, cross), like Zohara's own apps.
    Colours come from the colour scheme (title bar and text), so it follows light and dark.
    SPDX-License-Identifier: GPL-2.0-or-later
*/
import QtQuick
import org.kde.kwin.decoration

Decoration {
    id: root
    alpha: true

    DecorationOptions {
        id: options
        deco: decoration
    }

    readonly property color barColor: options.titleBarColor
    readonly property bool isDark: barColor.hslLightness < 0.5
    readonly property real buttonSize: 24
    readonly property real titleHeight: Math.max(36, caption.implicitHeight + 14)
    readonly property real radius: decoration.client.maximized ? 0 : 10
    readonly property color edge: isDark ? Qt.lighter(barColor, 1.5) : Qt.darker(barColor, 1.18)

    function setSizes() {
        borders.setBorders(1)
        borders.setTitle(titleHeight)
        maximizedBorders.setBorders(0)
        maximizedBorders.setTitle(titleHeight)
        // invisible handles around the window, so it is easy to grab an edge
        extendedBorders.setAllBorders(4)
    }

    Component.onCompleted: setSizes()
    onTitleHeightChanged: setSizes()

    // the window's frame: a thin line round the title bar and the sides
    Rectangle {
        anchors.fill: parent
        color: "transparent"
        radius: root.radius
        border.width: decoration.client.maximized ? 0 : 1
        border.color: root.edge
        visible: !decoration.client.maximized
    }

    Rectangle {
        id: bar
        anchors { left: parent.left; right: parent.right; top: parent.top }
        height: root.titleHeight
        color: root.barColor
        topLeftRadius: root.radius
        topRightRadius: root.radius
        Behavior on color { ColorAnimation { duration: 120 } }

        Item {
            id: titleRow
            anchors { fill: parent; leftMargin: 8; rightMargin: 8 }

            ButtonGroup {
                id: leftButtons
                spacing: 8
                explicitSpacer: root.buttonSize
                menuButton: menuButtonComponent
                appMenuButton: appMenuButtonComponent
                minimizeButton: minimizeButtonComponent
                maximizeButton: maximizeButtonComponent
                keepBelowButton: keepBelowButtonComponent
                keepAboveButton: keepAboveButtonComponent
                helpButton: helpButtonComponent
                shadeButton: shadeButtonComponent
                allDesktopsButton: stickyButtonComponent
                closeButton: closeButtonComponent
                buttons: options.titleButtonsLeft
                height: root.buttonSize
                anchors { left: parent.left; verticalCenter: parent.verticalCenter }
            }

            Text {
                id: caption
                anchors {
                    left: leftButtons.right; right: rightButtons.left
                    leftMargin: 10; rightMargin: 10
                    verticalCenter: parent.verticalCenter
                }
                horizontalAlignment: Text.AlignHCenter
                textFormat: Text.PlainText
                text: decoration.client.caption
                color: options.fontColor
                font: options.titleFont
                elide: Text.ElideMiddle
                renderType: Text.NativeRendering
            }

            ButtonGroup {
                id: rightButtons
                spacing: 8
                explicitSpacer: root.buttonSize
                menuButton: menuButtonComponent
                appMenuButton: appMenuButtonComponent
                minimizeButton: minimizeButtonComponent
                maximizeButton: maximizeButtonComponent
                keepBelowButton: keepBelowButtonComponent
                keepAboveButton: keepAboveButtonComponent
                helpButton: helpButtonComponent
                shadeButton: shadeButtonComponent
                allDesktopsButton: stickyButtonComponent
                closeButton: closeButtonComponent
                buttons: options.titleButtonsRight
                height: root.buttonSize
                anchors { right: parent.right; verticalCenter: parent.verticalCenter }
            }

            Component.onCompleted: decoration.installTitleItem(titleRow)
        }
    }

    // the window's own side and bottom frame
    Rectangle {
        visible: !decoration.client.maximized
        anchors { left: parent.left; right: parent.right; top: bar.bottom; bottom: parent.bottom }
        color: root.barColor
        z: -1
    }

    Component {
        id: menuButtonComponent
        MenuButton {
            width: root.buttonSize
            height: root.buttonSize
        }
    }
    Component {
        id: appMenuButtonComponent
        ZoharaButton {
            buttonType: DecorationOptions.DecorationButtonApplicationMenu
            size: root.buttonSize
        }
    }
    Component {
        id: minimizeButtonComponent
        ZoharaButton {
            buttonType: DecorationOptions.DecorationButtonMinimize
            size: root.buttonSize
        }
    }
    Component {
        id: maximizeButtonComponent
        ZoharaButton {
            buttonType: DecorationOptions.DecorationButtonMaximizeRestore
            size: root.buttonSize
        }
    }
    Component {
        id: keepBelowButtonComponent
        ZoharaButton {
            buttonType: DecorationOptions.DecorationButtonKeepBelow
            size: root.buttonSize
        }
    }
    Component {
        id: keepAboveButtonComponent
        ZoharaButton {
            buttonType: DecorationOptions.DecorationButtonKeepAbove
            size: root.buttonSize
        }
    }
    Component {
        id: helpButtonComponent
        ZoharaButton {
            buttonType: DecorationOptions.DecorationButtonQuickHelp
            size: root.buttonSize
        }
    }
    Component {
        id: shadeButtonComponent
        ZoharaButton {
            buttonType: DecorationOptions.DecorationButtonShade
            size: root.buttonSize
        }
    }
    Component {
        id: stickyButtonComponent
        ZoharaButton {
            buttonType: DecorationOptions.DecorationButtonOnAllDesktops
            size: root.buttonSize
        }
    }
    Component {
        id: closeButtonComponent
        ZoharaButton {
            buttonType: DecorationOptions.DecorationButtonClose
            size: root.buttonSize
        }
    }
}
