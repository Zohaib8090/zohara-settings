/*
    SPDX-License-Identifier: GPL-3.0-or-later
    Zohara OS: dynamic desktops. A new app gets a desktop of its own; when its last window closes that desktop goes away.
    Options (kwinrc [Script-zoharadynamicdesktops]): GroupByApp, SwitchToNew, CloseEmpty, MaxDesktops, Ignore, SettleSeconds.
*/

var groupByApp = readConfig("GroupByApp", true);
var switchToNew = readConfig("SwitchToNew", true);
var closeEmpty = readConfig("CloseEmpty", true);
var maxDesktops = Math.max(2, Number(readConfig("MaxDesktops", 12)));
// Windows that appear while you log in come back from the last session: leave them where they were.
var settleMs = Math.max(0, Number(readConfig("SettleSeconds", 15))) * 1000;
var ignore = String(readConfig("Ignore", "")).toLowerCase().split(",").map(function (s) { return s.trim(); }).filter(function (s) { return s.length > 0; });

var startedAt = Date.now();
// the desktops this script made: only these are ever removed again
var made = {};

function appName(w) {
    return String(w.desktopFileName || w.resourceClass || w.resourceName || "").toLowerCase();
}

// a real application window: not a dialog, popup, panel, splash or the desktop itself
function isAppWindow(w) {
    return w && w.normalWindow && !w.transient && !w.dialog && !w.desktopWindow && !w.dock && !w.splash && !w.skipTaskbar;
}

function ignored(w) {
    var a = appName(w), c = String(w.resourceClass || "").toLowerCase();
    return ignore.indexOf(a) >= 0 || ignore.indexOf(c) >= 0;
}

function otherWindowOfSameApp(w) {
    var a = appName(w);
    if (a === "") return null;
    var all = workspace.windowList();
    for (var i = 0; i < all.length; i++) {
        var o = all[i];
        if (o !== w && isAppWindow(o) && appName(o) === a && !o.onAllDesktops && o.desktops.length > 0) return o;
    }
    return null;
}

// "org.kde.kwrite" -> "Kwrite", "firefox" -> "Firefox"
function title(w) {
    var n = String(w.resourceClass || w.resourceName || "App");
    n = n.substring(n.lastIndexOf(".") + 1) || n;
    return n.charAt(0).toUpperCase() + n.slice(1);
}

function onAdded(w) {
    if (!isAppWindow(w) || ignored(w) || w.onAllDesktops) return;
    if (Date.now() - startedAt < settleMs) return;

    if (groupByApp) {
        var other = otherWindowOfSameApp(w);
        if (other) {
            w.desktops = other.desktops;
            if (switchToNew) workspace.currentDesktop = other.desktops[0];
            return;
        }
    }
    if (workspace.desktops.length >= maxDesktops) return;

    workspace.createDesktop(workspace.desktops.length, title(w));
    var d = workspace.desktops[workspace.desktops.length - 1];
    if (!d) return;
    made[d.id] = true;
    w.desktops = [d];
    if (switchToNew) workspace.currentDesktop = d;
}

// `leaving` is the window that is closing right now: it may still be in the list, and must not keep its own desktop alive
function hasAppWindows(d, leaving) {
    var all = workspace.windowList();
    for (var i = 0; i < all.length; i++) {
        var o = all[i];
        if (o === leaving || !isAppWindow(o)) continue;
        if (o.onAllDesktops) continue;
        for (var j = 0; j < o.desktops.length; j++) if (o.desktops[j].id === d.id) return true;
    }
    return false;
}

function onRemoved(w) {
    if (!closeEmpty || !w) return;
    var ds = w.desktops ? w.desktops.slice() : [];
    for (var i = 0; i < ds.length; i++) {
        var d = ds[i];
        if (!d || !made[d.id]) continue;
        if (hasAppWindows(d, w)) continue;
        // never leave you looking at a desktop that is about to disappear
        if (workspace.currentDesktop && workspace.currentDesktop.id === d.id) {
            var all = workspace.desktops;
            for (var k = 0; k < all.length; k++) {
                if (all[k].id === d.id) { workspace.currentDesktop = all[Math.max(0, k - 1)]; break; }
            }
        }
        delete made[d.id];
        workspace.removeDesktop(d);
    }
}

workspace.windowAdded.connect(onAdded);
workspace.windowRemoved.connect(onRemoved);
