/*
    SPDX-License-Identifier: GPL-2.0-only OR GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL
    Based on the KDE Cube effect (kdeplasma-addons), SPDX-FileCopyrightText: 2022 Vlad Zahorodnii
*/

import QtQuick
import QtQuick3D
import org.kde.kwin as KWinComponents

// The cube for one screen. `effect.pos` is the one number that drives everything: 0 is the first desktop facing you,
// 1 the second, 0.5 halfway between them. Fingers set it while they move; after you let go it glides to a whole number.
Item {
    id: root

    focus: true
    readonly property QtObject targetScreen: KWinComponents.SceneView.screen
    readonly property int count: KWinComponents.Workspace.desktops.length
    readonly property real faceW: width
    readonly property real faceH: height
    readonly property real fov: 60
    // Zorin's shape while you switch: with the gap the faces cover only half a circle (no wrap-around). In the overview
    // the cube closes into a whole prism so every desktop is a side of it and it can be turned all the way round.
    readonly property real openAngle: count < 2 ? 90 : (count === 2 ? 90 : 180 / (count - 1))
    readonly property real closedAngle: count < 2 ? 90 : 360 / count
    readonly property real faceAngle: effect.openCube ? openAngle + (closedAngle - openAngle) * effect.amount : closedAngle
    readonly property real centerDepth: faceAngle < 180 ? 0.5 * faceW / Math.tan(faceAngle * Math.PI / 360) : 0.5 * faceW
    readonly property real cornerDepth: Math.sqrt(centerDepth * centerDepth + 0.25 * faceW * faceW)
    readonly property real halfFov: 0.5 * fov * Math.PI / 180
    // camera distance from the middle of the cube so that the face in front fills the screen exactly
    readonly property real closeRadius: centerDepth + 0.5 * faceH / Math.tan(halfFov)
    // the whole cube in view: far enough back that its height and its widest point both fit with some room around
    readonly property real farRadius: {
        const closedDepth = 0.5 * faceW / Math.tan(closedAngle * Math.PI / 360);
        const closedCorner = Math.sqrt(closedDepth * closedDepth + 0.25 * faceW * faceW);
        const fitH = 0.5 * faceH * 1.55 / Math.tan(halfFov);
        const fitW = closedCorner * 1.35 / (Math.tan(halfFov) * (faceW / faceH));
        return closedCorner + Math.max(fitH, fitW);
    }
    // 0 when a face looks straight at you, 1 halfway between two faces
    readonly property real between: (1 - 2 * Math.abs(effect.pos - Math.floor(effect.pos) - 0.5)) * (1 - effect.amount)

    // Behind the cube: your own wallpaper, a little darker in the overview so the cube stands out.
    KWinComponents.DesktopBackground {
        anchors.fill: parent
        activity: KWinComponents.Workspace.currentActivity
        desktop: KWinComponents.Workspace.currentDesktop
        outputName: root.targetScreen.name
    }
    Rectangle {
        anchors.fill: parent
        color: "black"
        opacity: 0.35 * effect.amount
    }

    View3D {
        id: view
        anchors.fill: parent
        renderMode: View3D.Offscreen

        environment: SceneEnvironment {
            backgroundMode: SceneEnvironment.Transparent
        }

        // The camera swings around the middle of the cube.
        Node {
            id: pivot
            eulerRotation.y: effect.pos * root.faceAngle
            eulerRotation.x: -(effect.tilt * root.between + effect.amount * 25 + effect.userTilt)

            PerspectiveCamera {
                id: camera
                fieldOfView: root.fov
                clipNear: 10.0
                clipFar: 100000.0
                z: root.closeRadius + (root.farRadius - root.closeRadius) * effect.amount
                   + effect.pullback * root.between * (root.cornerDepth - root.centerDepth) * 2.0
            }
        }

        Node {
            id: cube
            Repeater3D {
                model: KWinComponents.VirtualDesktopModel {}
                delegate: Model {
                    id: face
                    required property QtObject desktop
                    required property int index
                    pickable: true

                    source: "#Rectangle"
                    scale: Qt.vector3d(root.faceW / 100, root.faceH / 100, 1)
                    eulerRotation.y: root.faceAngle * index
                    position: {
                        const t = Qt.matrix4x4();
                        t.rotate(root.faceAngle * index, Qt.vector3d(0, 1, 0));
                        return t.times(Qt.vector3d(0, 0, root.centerDepth));
                    }
                    materials: [
                        DefaultMaterial {
                            cullMode: Material.NoCulling
                            lighting: DefaultMaterial.NoLighting
                            diffuseMap: Texture {
                                sourceItem: DesktopView {
                                    desktop: face.desktop
                                    targetScreen: root.targetScreen
                                    width: root.faceW
                                    height: root.faceH
                                }
                            }
                        }
                    ]
                }
            }
        }
    }

    // Overview: drag to turn the cube (left and right) and tip it (up and down), click a face to go there.
    MouseArea {
        id: mouse
        anchors.fill: parent
        enabled: effect.overview
        property real lastX: 0
        property real lastY: 0
        property real startX: 0
        property real startY: 0
        property real velocity: 0
        property bool moved: false

        onPressed: m => {
            lastX = startX = m.x; lastY = startY = m.y; velocity = 0; moved = false;
            glide.stop();
        }
        onPositionChanged: m => {
            const dx = m.x - lastX, dy = m.y - lastY;
            lastX = m.x; lastY = m.y;
            if (Math.abs(m.x - startX) + Math.abs(m.y - startY) > 6) moved = true;
            if (!moved) return;
            const step = -dx / (root.faceW * 0.55);
            velocity = 0.6 * velocity + 0.4 * step;
            // free all the way round
            effect.pos = effect.pos + step;
            effect.userTilt = Math.max(-effect.maxTilt, Math.min(effect.maxTilt, effect.userTilt + dy * 0.15));
        }
        onReleased: m => {
            if (moved) {
                effect.snapNearest(velocity * 6);
            } else {
                const hit = view.pick(m.x, m.y);
                if (hit.objectHit && hit.objectHit.desktop) {
                    effect.faceClicked(hit.objectHit.desktop);
                } else {
                    effect.emptyClicked();
                }
            }
        }
    }

    // Overview: the mouse wheel and two-finger scrolling turn the cube. A wheel notch moves one desktop; a touchpad turns it
    // as your fingers move and settles on the nearest desktop when they stop.
    WheelHandler {
        id: wheel
        enabled: effect.overview
        acceptedDevices: PointerDevice.Mouse | PointerDevice.TouchPad
        onWheel: ev => {
            // which axis counts: both (the stronger one), only up/down or only left/right (Settings: Scroll in the overview)
            const pick = (x, y) => effect.scrollMode === 1 ? y : (effect.scrollMode === 2 ? x : (Math.abs(x) > Math.abs(y) ? x : y));
            const sign = effect.scrollReverse ? -1 : 1;
            // a wheel notch is 120 units; a touchpad sends many small steps (and some drivers fill pixelDelta instead)
            const px = pick(ev.pixelDelta.x, ev.pixelDelta.y) * sign;
            const an = pick(ev.angleDelta.x, ev.angleDelta.y) * sign;
            if (px === 0 && Math.abs(an) >= 120) {
                effect.snapNearest(an > 0 ? -1 : 1);
                return;
            }
            // smooth: your fingers turn the cube (about 240 units for one desktop), it settles when they stop
            const step = px !== 0 ? -px / (root.faceW * 0.55) : -an / 240;
            if (step === 0) return;
            glide.stop();
            effect.pos = effect.pos + step;
            settleTimer.restart();
        }
    }
    Timer {
        id: settleTimer
        interval: 160
        onTriggered: effect.snapNearest(0)
    }

    WindowTray {
        anchors.fill: parent
        targetScreen: root.targetScreen
    }

    Keys.onEscapePressed: effect.handleEscape()
    Keys.onLeftPressed: effect.snapNearest(-1)
    Keys.onRightPressed: effect.snapNearest(1)
    Keys.onReturnPressed: effect.handleEnter()
    Keys.onEnterPressed: effect.handleEnter()
}
