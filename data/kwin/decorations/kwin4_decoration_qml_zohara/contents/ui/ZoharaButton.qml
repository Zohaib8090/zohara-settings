/*
    One round window button: a soft circle with a thin white-or-dark symbol, a red circle for Close on hover.
    SPDX-License-Identifier: GPL-2.0-or-later
*/
import QtQuick
import org.kde.kwin.decoration

DecorationButton {
    id: button
    property real size: 24
    width: size
    height: size

    readonly property bool isClose: buttonType === DecorationOptions.DecorationButtonClose
    // the circle behind the symbol: a veil of the text colour, stronger when hovered or pressed
    property color circle: {
        if (isClose && (hovered || pressed)) return pressed ? "#c23b3f" : "#e5484d"
        const base = root.isDark ? "#ffffff" : "#000000"
        const alpha = pressed ? 0.28 : hovered ? 0.20 : 0.10
        return Qt.rgba(Qt.color(base).r, Qt.color(base).g, Qt.color(base).b, alpha)
    }
    property color glyph: (isClose && (hovered || pressed)) ? "#ffffff" : options.fontColor
    opacity: enabled ? 1.0 : 0.4

    Rectangle {
        anchors.fill: parent
        radius: width / 2
        color: button.circle
        Behavior on color { ColorAnimation { duration: 90 } }
    }

    Canvas {
        id: canvas
        anchors.fill: parent
        antialiasing: true
        renderTarget: Canvas.Image
        onPaint: {
            const ctx = getContext("2d")
            ctx.reset()
            const s = width
            ctx.strokeStyle = button.glyph
            ctx.fillStyle = button.glyph
            ctx.lineWidth = 1.7
            ctx.lineCap = "round"
            ctx.lineJoin = "round"
            const c = s / 2
            const h = s * 0.21     // half the symbol's size
            ctx.beginPath()
            switch (button.buttonType) {
            case DecorationOptions.DecorationButtonMinimize:
                ctx.moveTo(c - h, c + h * 0.7); ctx.lineTo(c + h, c + h * 0.7)
                break
            case DecorationOptions.DecorationButtonMaximizeRestore:
                if (decoration.client.maximized) {
                    ctx.rect(c - h, c - h * 0.5, h * 1.5, h * 1.5)
                    ctx.moveTo(c - h * 0.5, c - h * 0.5); ctx.lineTo(c - h * 0.5, c - h); ctx.lineTo(c + h, c - h); ctx.lineTo(c + h, c + h * 0.5); ctx.lineTo(c + h * 0.5, c + h * 0.5)
                } else {
                    ctx.rect(c - h, c - h, h * 2, h * 2)
                }
                break
            case DecorationOptions.DecorationButtonClose:
                ctx.moveTo(c - h, c - h); ctx.lineTo(c + h, c + h)
                ctx.moveTo(c + h, c - h); ctx.lineTo(c - h, c + h)
                break
            case DecorationOptions.DecorationButtonOnAllDesktops:
                ctx.arc(c, c, h * 0.8, 0, Math.PI * 2)
                if (button.toggled) ctx.fill()
                break
            case DecorationOptions.DecorationButtonKeepAbove:
                ctx.moveTo(c - h, c + h * 0.4); ctx.lineTo(c, c - h * 0.5); ctx.lineTo(c + h, c + h * 0.4)
                if (button.toggled) { ctx.moveTo(c - h, c + h * 1.1); ctx.lineTo(c + h, c + h * 1.1) }
                break
            case DecorationOptions.DecorationButtonKeepBelow:
                ctx.moveTo(c - h, c - h * 0.4); ctx.lineTo(c, c + h * 0.5); ctx.lineTo(c + h, c - h * 0.4)
                if (button.toggled) { ctx.moveTo(c - h, c - h * 1.1); ctx.lineTo(c + h, c - h * 1.1) }
                break
            case DecorationOptions.DecorationButtonShade:
                ctx.moveTo(c - h, c - h * 0.6); ctx.lineTo(c + h, c - h * 0.6)
                ctx.moveTo(c - h, c + h * 0.4); ctx.lineTo(c, c + h * 1.1); ctx.lineTo(c + h, c + h * 0.4)
                break
            case DecorationOptions.DecorationButtonApplicationMenu:
                ctx.moveTo(c - h, c - h * 0.7); ctx.lineTo(c + h, c - h * 0.7)
                ctx.moveTo(c - h, c); ctx.lineTo(c + h, c)
                ctx.moveTo(c - h, c + h * 0.7); ctx.lineTo(c + h, c + h * 0.7)
                break
            case DecorationOptions.DecorationButtonQuickHelp:
                ctx.font = "bold " + Math.round(s * 0.55) + "px sans-serif"
                ctx.textAlign = "center"
                ctx.fillText("?", c, c + s * 0.2)
                break
            }
            ctx.stroke()
        }
        Connections {
            target: button
            function onGlyphChanged() { canvas.requestPaint() }
            function onToggledChanged() { canvas.requestPaint() }
        }
        Connections {
            target: decoration.client
            function onMaximizedChanged() { canvas.requestPaint() }
        }
        Component.onCompleted: requestPaint()
        onWidthChanged: requestPaint()
    }
}
