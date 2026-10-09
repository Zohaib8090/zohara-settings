/*
    SPDX-License-Identifier: GPL-2.0-only OR GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL
*/

import QtQuick
import QtQml
import org.kde.kwin as KWinComponents

// Zohara Cube: a cube that turns when you switch desktops, and follows 3 and 4 finger swipes on the touchpad while
// your fingers are still moving. KWin itself decides which desktop a swipe ends on; this effect only draws.
KWinComponents.SceneEffect {
    id: effect

    delegate: ScreenView {}

    readonly property int duration: Math.max(100, effect.configuration.Duration || 450)
    readonly property bool openCube: effect.configuration.OpenCube !== false
    readonly property real pullback: effect.configuration.Pullback !== undefined ? effect.configuration.Pullback : 0.6
    // 4 fingers up and down belong to the cube's overview, or are left to KWin's own Overview
    readonly property bool overviewGesture: effect.configuration.OverviewGesture !== false
    readonly property real tilt: effect.configuration.Tilt || 0
    readonly property int desktopCount: KWinComponents.Workspace.desktops.length

    // where the cube is turned to, in desktops (0 = first desktop faces you)
    property real pos: 0
    // fingers are on the touchpad
    property bool dragging: false
    // which kind of swipe is moving the cube right now: "" (none), "switch" (left or right) or "overview" (up)
    property string gesture: ""
    property int lastIndex: indexOf(KWinComponents.Workspace.currentDesktop)

    // Overview: the cube zoomed out and tipped so you can turn it with the mouse and pick a desktop.
    property bool overview: false
    // 0 = normal view, 1 = fully zoomed out (follows 4 finger swipe up while the fingers move)
    property real amount: 0
    // extra tilt in degrees from dragging up and down with the mouse
    property real userTilt: 0
    readonly property real maxTilt: 50

    // the desktop whose face is number `i`
    function desktopAt(i) {
        const list = KWinComponents.Workspace.desktops;
        const n = list.length;
        return list[((i % n) + n) % n];
    }

    function indexOf(desktop) {
        return desktop ? desktop.x11DesktopNumber - 1 : 0;
    }

    function currentIndex() {
        return indexOf(KWinComponents.Workspace.currentDesktop);
    }

    function show() {
        if (!effect.visible && desktopCount > 1) {
            effect.visible = true;
        }
    }

    // glide to the desktop that is current now, then get out of the way
    function settle() {
        if (gesture !== "") return;
        dragging = false;
        glide.stop();
        glide.from = effect.pos;
        glide.to = currentIndex();
        // a short turn takes less time than a long one
        glide.duration = Math.max(120, Math.min(effect.duration, effect.duration * Math.max(0.35, Math.abs(glide.to - glide.from))));
        glide.restart();
    }

    NumberAnimation {
        id: glide
        target: effect
        property: "pos"
        easing.type: Easing.OutCubic
        onFinished: effect.maybeHide()
    }

    function maybeHide() {
        if (!dragging && !overview && amount === 0 && !glide.running && !zoom.running) {
            visible = false;
            lastIndex = currentIndex();
        }
    }

    NumberAnimation {
        id: zoom
        target: effect
        property: "amount"
        duration: 380
        easing.type: Easing.OutCubic
        onFinished: effect.maybeHide()
    }

    NumberAnimation {
        id: tiltBack
        target: effect
        property: "userTilt"
        to: 0
        duration: 380
        easing.type: Easing.OutCubic
    }

    function openOverview() {
        if (desktopCount < 2) return;
        glide.stop();
        dragging = false;
        pos = currentIndex();
        show();
        overview = true;
        zoom.stop();
        zoom.to = 1;
        zoom.restart();
    }

    function closeOverview() {
        if (!overview && amount === 0) return;
        overview = false;
        // the whole cube was turned round: bring the turn back into one lap so it ends on a real desktop
        const n = desktopCount;
        pos = pos - Math.floor(pos / n) * n;
        zoom.stop();
        zoom.to = 0;
        zoom.restart();
        tiltBack.restart();
        settle();
    }

    function toggleOverview() {
        if (overview) closeOverview(); else openOverview();
    }

    // choose a desktop in the overview and leave
    function pick(desktop) {
        KWinComponents.Workspace.currentDesktop = desktop;
        closeOverview();
    }

    // after the mouse lets go in the overview: turn to the nearest face, without switching yet
    function snapNearest(velocity) {
        glide.stop();
        // the overview cube is a whole prism: it may be turned any number of laps
        const target = Math.round(pos + velocity);
        glide.from = pos;
        glide.to = target;
        glide.duration = Math.max(150, Math.min(effect.duration, effect.duration * Math.max(0.35, Math.abs(target - pos))));
        glide.restart();
    }

    // A switch that did not come from the touchpad (keyboard, pager, window dragged to the edge).
    Connections {
        target: KWinComponents.Workspace
        function onCurrentDesktopChanged(previous, current, screen) {
            if (effect.desktopCount < 2 || effect.overview) return;
            if (!effect.dragging && !effect.visible) {
                effect.pos = effect.lastIndex;
                effect.show();
            }
            settle();
        }
    }

    // One handler per finger count and direction. KWin reports how far the fingers moved as a number from 0 to 1.
    //   kind "switch": left or right turns the cube to the next or previous desktop (left = next, like KWin's Slide)
    //   kind "up":     up zooms the cube out into the overview while the fingers move
    //   kind "down":   down closes the overview
    // KWin also tells every handler that did not take a swipe that it was cancelled (a swipe up cancels the left and
    // right ones): only a handler that really moved the cube may end it.
    Instantiator {
        model: [
            { dir: KWinComponents.SwipeGestureHandler.Direction.Left, fingers: 3, kind: "switch", sign: 1 },
            { dir: KWinComponents.SwipeGestureHandler.Direction.Right, fingers: 3, kind: "switch", sign: -1 },
            { dir: KWinComponents.SwipeGestureHandler.Direction.Left, fingers: 4, kind: "switch", sign: 1 },
            { dir: KWinComponents.SwipeGestureHandler.Direction.Right, fingers: 4, kind: "switch", sign: -1 },
            { dir: KWinComponents.SwipeGestureHandler.Direction.Up, fingers: 4, kind: "up", sign: 0 },
            { dir: KWinComponents.SwipeGestureHandler.Direction.Down, fingers: 4, kind: "down", sign: 0 }
        ]
        delegate: KWinComponents.SwipeGestureHandler {
            direction: modelData.dir
            fingerCount: modelData.fingers
            deviceType: KWinComponents.SwipeGestureHandler.Touchpad
            property bool engaged: false

            onProgressChanged: {
                if (effect.desktopCount < 2 || progress <= 0) return;
                if (modelData.kind === "switch") {
                    if (effect.overview || (effect.gesture !== "" && effect.gesture !== "switch")) return;
                    if (!engaged) {
                        engaged = true;
                        effect.gesture = "switch";
                        glide.stop();
                        effect.dragging = true;
                        effect.pos = effect.currentIndex();
                        effect.show();
                    }
                    const base = effect.currentIndex();
                    const target = base + modelData.sign * Math.min(1, progress);
                    // the first and last desktop pull back a little instead of turning into nothing
                    const last = effect.desktopCount - 1;
                    effect.pos = target < 0 ? target * 0.25 : (target > last ? last + (target - last) * 0.25 : target);
                } else if (modelData.kind === "up") {
                    if (!effect.overviewGesture) return;
                    if (effect.overview || (effect.gesture !== "" && effect.gesture !== "overview")) return;
                    if (!engaged) {
                        engaged = true;
                        effect.gesture = "overview";
                        glide.stop();
                        zoom.stop();
                        effect.dragging = true;
                        effect.pos = effect.currentIndex();
                        effect.show();
                    }
                    effect.amount = Math.min(1, progress);
                }
            }

            // fingers lifted far enough
            onActivated: {
                if (modelData.kind === "down") {
                    if (effect.overviewGesture) effect.closeOverview();
                    return;
                }
                if (!engaged) return;
                engaged = false;
                effect.gesture = "";
                if (modelData.kind === "up") {
                    effect.dragging = false;
                    effect.openOverview();
                } else {
                    settleTimer.restart();
                }
            }

            // not far enough, or KWin gave the swipe to another handler
            onCancelled: {
                if (!engaged) return;
                engaged = false;
                effect.gesture = "";
                if (modelData.kind === "up") {
                    effect.dragging = false;
                    zoom.to = 0;
                    zoom.restart();
                } else {
                    settleTimer.restart();
                }
            }
        }
    }

    KWinComponents.ShortcutHandler {
        name: "Zohara Cube"
        text: "Toggle the desktop cube"
        sequence: "Meta+C"
        onActivated: effect.toggleOverview()
    }

    // KWin switches the desktop just after the fingers lift; give it a moment, then settle on whatever is current
    Timer {
        id: settleTimer
        interval: 60
        onTriggered: effect.settle()
    }

    Component.onCompleted: {
        effect.pos = currentIndex();
    }
}
