// Manual mode of the dynamic desktops script: apps never create desktops by themselves; you send a window to a new desktop
// from the title bar menu or with the shortcut. Run like the other test (see tests/run-dynamic-desktops-tests.sh).
var cfg = { Mode: "manual", GroupByApp: true, SwitchToNew: true, CloseEmpty: true, MaxDesktops: 4, SettleSeconds: 0, Ignore: "" };
function readConfig(k, d) { return cfg.hasOwnProperty(k) ? cfg[k] : d; }
var nextId = 1, handlers = {}, shortcuts = [], menus = [];
function Signal(name) { this.connect = function (f) { handlers[name] = f; }; }
function registerShortcut(title, text, keys, cb) { shortcuts.push({ title: title, keys: keys, cb: cb }); }
function registerUserActionsMenu(cb) { menus.push(cb); }
var desk0 = { id: "d0", name: "Desktop 1" };
var workspace = {
  desktops: [desk0], currentDesktop: desk0, wins: [], activeWindow: null,
  windowAdded: new Signal("added"), windowRemoved: new Signal("removed"),
  windowList: function () { return this.wins; },
  createDesktop: function (pos, name) { this.desktops.push({ id: "d" + (nextId++), name: name }); },
  removeDesktop: function (d) { this.desktops = this.desktops.filter(function (x) { return x.id !== d.id; }); }
};
function win(cls, extra) {
  var w = { normalWindow: true, transient: false, dialog: false, desktopWindow: false, dock: false, splash: false, skipTaskbar: false,
            resourceClass: cls, desktopFileName: cls, onAllDesktops: false, desktops: [workspace.currentDesktop] };
  for (var k in extra || {}) w[k] = extra[k];
  return w;
}
function open(w) { workspace.wins.push(w); handlers.added(w); }
function close(w) { handlers.removed(w); workspace.wins = workspace.wins.filter(function (x) { return x !== w; }); }
var fails = 0;
function check(name, ok) { print((ok ? "ok   " : "FAIL ") + name); if (!ok) fails++; }

//SCRIPT//

check("the shortcut and the menu entry are registered", shortcuts.length === 1 && shortcuts[0].keys === "Meta+Shift+N" && menus.length === 1);

var a = win("kate"); open(a);
check("manual mode: opening an app does not create a desktop", workspace.desktops.length === 1 && a.desktops[0].id === "d0");

var entry = menus[0](a);
check("the title bar menu offers 'Move to a new desktop'", entry && entry.text === "Move to a new desktop");
entry.triggered();
check("choosing it puts the window on a new desktop named after the app", workspace.desktops.length === 2 && a.desktops[0].id === "d1" && workspace.desktops[1].name === "Kate\u200b");
check("and takes you there", workspace.currentDesktop.id === "d1");

var b = win("konsole"); open(b); workspace.activeWindow = b;
shortcuts[0].cb();
check("the shortcut moves the active window to a new desktop", workspace.desktops.length === 3 && b.desktops[0].id !== "d0");

check("system helpers are not offered the menu entry", menus[0](win("polkit-kde-authentication-agent-1")) === undefined);
check("dialogs are not offered it either", menus[0](win("kate", { dialog: true })) === undefined);

close(a);
check("closing the window still removes its (now empty) desktop of ours", !workspace.desktops.some(function (x) { return x.id === "d1"; }));

// moving a window away from a desktop of ours that is then empty clears that desktop
var c = win("gimp"); open(c); workspace.activeWindow = c; shortcuts[0].cb();
var cd = c.desktops[0].id;
var before = workspace.desktops.length;
workspace.activeWindow = c; shortcuts[0].cb();
check("sending a window on from one of our desktops to another leaves no empty desktop behind", !workspace.desktops.some(function (x) { return x.id === cd; }));

print(fails === 0 ? "ALL OK" : fails + " FAILED");
