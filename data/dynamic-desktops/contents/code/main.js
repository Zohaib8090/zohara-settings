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

// The desktops this script makes carry an invisible mark at the end of their name, so a script that was restarted (Settings
// restarts it when an option changes) still knows which desktops are its own. Only marked desktops are ever removed again;
// rename a desktop and it is yours.
var MARK = "\u200b";
function isMine(d) {
    var n = d ? String(d.name) : "";
    return n.length > 0 && n.charAt(n.length - 1) === MARK;
}

function appName(w) {
    return String(w.desktopFileName || w.resourceClass || w.resourceName || "").toLowerCase();
}

// a real application window: not a dialog, popup, panel, splash or the desktop itself
function isAppWindow(w) {
    return w && w.normalWindow && !w.transient && !w.dialog && !w.desktopWindow && !w.dock && !w.splash && !w.skipTaskbar;
}

// System helpers are never "apps you opened": the password prompt (polkit), wallet and key prompts, the lock and login
// screens, the shell itself, portals and background services. Their windows must appear where you are, or you would not
// see them (a password prompt on a desktop of its own is a prompt you never answer).
var SYSTEM = ["polkit", "pkexec", "kwallet", "ksecret", "gcr-prompter", "pinentry", "ssh-askpass", "plasmashell", "krunner",
              "kscreenlocker", "ksmserver", "ksplash", "kded", "xdg-desktop-portal", "org.freedesktop.impl.portal", "kdeconnect",
              "org.kde.kwin", "kwin_", "plasma-", "org.kde.plasma", "systemsettings-kcm", "zohara-polkit", "sddm"];

function isSystemHelper(w) {
    var names = [appName(w), String(w.resourceClass || "").toLowerCase(), String(w.resourceName || "").toLowerCase()];
    for (var i = 0; i < names.length; i++) {
        if (names[i] === "") continue;
        for (var j = 0; j < SYSTEM.length; j++) if (names[i].indexOf(SYSTEM[j]) >= 0) return true;
    }
    return false;
}

function ignored(w) {
    var a = appName(w), c = String(w.resourceClass || "").toLowerCase();
    return isSystemHelper(w) || ignore.indexOf(a) >= 0 || ignore.indexOf(c) >= 0;
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

    workspace.createDesktop(workspace.desktops.length, title(w) + MARK);
    var d = workspace.desktops[workspace.desktops.length - 1];
    if (!d) return;
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

// Removes `d` if it is one of ours and nothing is on it. Never leaves you looking at a desktop that is about to disappear.
function removeIfEmpty(d, leaving) {
    if (!d || !isMine(d) || hasAppWindows(d, leaving)) return;
    if (workspace.currentDesktop && workspace.currentDesktop.id === d.id) {
        var all = workspace.desktops;
        for (var k = 0; k < all.length; k++) {
            if (all[k].id === d.id) { workspace.currentDesktop = all[Math.max(0, k - 1)]; break; }
        }
    }
    workspace.removeDesktop(d);
}

function onRemoved(w) {
    if (!closeEmpty || !w) return;
    var ds = w.desktops ? w.desktops.slice() : [];
    for (var i = 0; i < ds.length; i++) removeIfEmpty(ds[i], w);
    // Also clear empty desktops of ours that an earlier run left behind. Not while you are logging in: the windows of the
    // last session are still on their way and their desktops only look empty.
    if (Date.now() - startedAt < settleMs) return;
    var all = workspace.desktops.slice();
    for (var j = 0; j < all.length; j++) removeIfEmpty(all[j], w);
}

workspace.windowAdded.connect(onAdded);
workspace.windowRemoved.connect(onRemoved);
