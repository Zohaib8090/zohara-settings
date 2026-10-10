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
    // overview scrolling: 0 up/down and left/right, 1 up/down only, 2 left/right only; reverse flips the direction
    readonly property int scrollMode: effect.configuration.ScrollMode || 0
    readonly property bool scrollReverse: effect.configuration.ScrollReverse === true
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
    property QtObject selected: null
    property string appFilter: ""
    // the desktop whose menu (move, delete) is open, opened by a double tap on its chip
    property QtObject menuDesktop: null

    // a window "picked up" with its Move button: tap a desktop above to put it there (for touchpads, where dragging is awkward)
    property QtObject armed: null

    // the desktop whose face is number `i`
    function desktopAt(i) {
        const list = KWinComponents.Workspace.desktops;
        const n = list.length;
        return list[((i % n) + n) % n];
    }

    KWinComponents.DBusCall {
        id: renameCall
        service: "org.kde.KWin"
        path: "/VirtualDesktopManager"
        dbusInterface: "org.kde.KWin.VirtualDesktopManager"
        method: "setDesktopName"
    }

    function setDesktopName(id, name) {
        renameCall.arguments = [String(id), String(name)];
        renameCall.call();
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
            clearSelection();
            visible = false;
            lastIndex = currentIndex();
        }
    }

    NumberAnimation {
        id: zoom
        target: effect
        property: "amount"
        duration: 480
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
        // works with a single desktop too: the overview is also where you make the next one ("+ New desktop")
        glide.stop();
        dragging = false;
        pos = currentIndex();
        effect.visible = true;
        selected = KWinComponents.Workspace.currentDesktop;
        appFilter = "";
        armed = null;
        overview = true;
        zoom.stop();
        zoom.to = 1;
        zoom.restart();
    }

    function closeOverview() {
        if (!overview && amount === 0) return;
        // what is picked stays until the layer has glided away (maybeHide clears it), so the tray does not empty as it leaves
        menuDesktop = null;
        armed = null;
        appFilter = "";
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

    // ── the overview's window layer ──
    function nearestEquivalent(index) {
        // the overview cube is a whole prism: the same face comes round every lap, take the nearest one
        const n = desktopCount;
        return index + n * Math.round((pos - index) / n);
    }

    function selectDesktop(d) {
        selected = d;
        appFilter = "";
        armed = null;
        glide.stop();
        glide.from = pos;
        glide.to = nearestEquivalent(indexOf(d));
        glide.duration = Math.max(150, Math.min(effect.duration, effect.duration * Math.max(0.35, Math.abs(glide.to - glide.from))));
        glide.restart();
    }

    // leaves the overview's layers: nothing picked, no app filter, nothing armed
    function clearSelection() {
        selected = null;
        appFilter = "";
        armed = null;
        menuDesktop = null;
    }

    function toggleArm(w) {
        armed = armed === w ? null : w;
    }

    // ── the desktop menu (double tap on a chip) ──
    function openDesktopMenu(d) {
        armed = null;
        menuDesktop = d;
    }

    function closeDesktopMenu() {
        menuDesktop = null;
    }

    // Moves the desktop `d` to position `to` (0 is the first), like dragging it in a list: the desktops in between shift by one.
    // KWin cannot reorder desktops, so the same thing is done by hand: every window follows its desktop to the new place and
    // the names are handed along, so what you see is exactly a moved desktop. You stay on the same windows.
    function moveDesktop(d, to) {
        const ws = KWinComponents.Workspace;
        const list = [];
        for (let i = 0; i < ws.desktops.length; i++) list.push(ws.desktops[i]);
        const n = list.length;
        const from = list.findIndex(x => x.id === d.id);
        if (from < 0 || to < 0 || to >= n || from === to) return;

        // order[newPosition] = oldPosition
        const order = [];
        for (let p = 0; p < n; p++) order.push(p);
        order.splice(from, 1);
        order.splice(to, 0, from);
        const newPosOf = {};
        for (let p = 0; p < n; p++) newPosOf[order[p]] = p;

        const names = list.map(x => String(x.name));
        const currentOld = list.findIndex(x => ws.currentDesktop && x.id === ws.currentDesktop.id);
        const selectedOld = selected ? list.findIndex(x => x.id === selected.id) : -1;

        // windows follow their desktop
        const wins = ws.windows;
        for (let k = 0; k < wins.length; k++) {
            const w = wins[k];
            if (!w || w.onAllDesktops || w.desktops.length === 0) continue;
            const next = [];
            for (let j = 0; j < w.desktops.length; j++) {
                const at = list.findIndex(x => x.id === w.desktops[j].id);
                if (at >= 0) next.push(list[newPosOf[at]]);
            }
            if (next.length > 0) w.desktops = next;
        }
        // names travel with their desktop
        for (let p = 0; p < n; p++) setDesktopName(list[p].id, names[order[p]]);

        if (currentOld >= 0) ws.currentDesktop = list[newPosOf[currentOld]];
        if (selectedOld >= 0) selected = list[newPosOf[selectedOld]];
        menuDesktop = list[to];
        pos = to;
    }

    // Removes a desktop. KWin moves its windows to another desktop; nothing is closed.
    function deleteDesktop(d) {
        const ws = KWinComponents.Workspace;
        if (ws.desktops.length <= 1) return;
        const wasSelected = selected !== null && selected.id === d.id;
        menuDesktop = null;
        ws.removeDesktop(d);
        if (wasSelected || selected === null) selected = ws.currentDesktop;
        appFilter = "";
        armed = null;
    }

    // a tap on a desktop chip: with a window armed it moves there, otherwise it picks that desktop
    function chipTapped(d) {
        if (armed !== null) {
            moveWindowTo(armed, d);
            armed = null;
        } else {
            selectDesktop(d);
        }
    }

    // a tap on "+ New desktop": with a window armed it goes to a new desktop of its own, otherwise an empty one is made
    function newChipTapped() {
        if (armed !== null) {
            moveWindowToNew(armed);
            armed = null;
        } else {
            newDesktop();
        }
    }

    function toggleAppFilter(app) {
        appFilter = appFilter === app ? "" : app;
    }

    // a click on a face: first click picks the desktop (its windows show), the same face again goes there
    function faceClicked(d) {
        if (selected !== null && selected.id === d.id) {
            pick(d);
        } else {
            selectDesktop(d);
        }
    }

    // a click on nothing: close what is open, one layer at a time
    function emptyClicked() {
        // the tray is always there now: a click on nothing steps back one layer, then closes the overview
        if (menuDesktop !== null) menuDesktop = null;
        else if (armed !== null) armed = null;
        else if (appFilter !== "") appFilter = "";
        else closeOverview();
    }

    function handleEscape() {
        emptyClicked();
    }

    function handleEnter() {
        pick(selected !== null ? selected : desktopAt(Math.round(pos)));
    }

    // an application window the tray may move: never a panel, the wallpaper, a popup or a dialog
    function movable(w) {
        return w && w.normalWindow && !w.dock && !w.desktopWindow && !w.dialog && !w.transient && !w.skipTaskbar;
    }

    function moveWindowTo(w, d) {
        if (!movable(w) || !d) return;
        w.desktops = [d];
        // the tray follows by itself: it lists what is on the picked desktop
    }

    // an empty new desktop (the "+ New desktop" button), picked at once so you can drop windows on it
    function newDesktop() {
        const ws = KWinComponents.Workspace;
        if (ws.desktops.length >= 20) return;
        ws.createDesktop(ws.desktops.length, "Desktop " + (ws.desktops.length + 1));
        const d = ws.desktops[ws.desktops.length - 1];
        if (d) selectDesktop(d);
    }

    function moveWindowToNew(w) {
        if (!movable(w)) return;
        const ws = KWinComponents.Workspace;
        if (ws.desktops.length >= 20) return;
        ws.createDesktop(ws.desktops.length, "Desktop " + (ws.desktops.length + 1));
        const d = ws.desktops[ws.desktops.length - 1];
        if (d) w.desktops = [d];
    }

    // go straight to a window: its desktop, and focus it
    function goToWindow(w) {
        if (!movable(w)) return;
        if (w.desktops.length > 0) KWinComponents.Workspace.currentDesktop = w.desktops[0];
        KWinComponents.Workspace.activeWindow = w;
        clearSelection();
        closeOverview();
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
                if (progress <= 0) return;
                if (modelData.kind === "switch") {
                    if (effect.desktopCount < 2) return;
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
                        effect.selected = KWinComponents.Workspace.currentDesktop;
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
