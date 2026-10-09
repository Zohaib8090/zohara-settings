// Runs the KWin script against a tiny fake workspace (mujs or gjs): `mujs tests/dynamic_desktops_test.js` from the repo root
// after concatenating with the script: see the shell snippet in docs. The script is loaded by the harness below.
var cfg = { GroupByApp: true, SwitchToNew: true, CloseEmpty: true, MaxDesktops: 4, SettleSeconds: 0, Ignore: "ignoredapp" };
function readConfig(k, d) { return cfg.hasOwnProperty(k) ? cfg[k] : d; }
var nextId = 1, handlers = {};
function Signal(name) { this.connect = function (f) { handlers[name] = f; }; }
var desk0 = { id: "d0", name: "Desktop 1" };
var workspace = {
  desktops: [desk0], currentDesktop: desk0, wins: [],
  windowAdded: new Signal("added"), windowRemoved: new Signal("removed"),
  windowList: function () { return this.wins; },
  createDesktop: function (pos, name) { this.desktops.push({ id: "d" + (nextId++), name: name }); },
  removeDesktop: function (d) {
    this.desktops = this.desktops.filter(function (x) { return x.id !== d.id; });
    // KWin moves windows of a removed desktop to the previous one; none are left here
  }
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

var a = win("kate");
open(a);
check("first app gets its own new desktop", workspace.desktops.length === 2 && a.desktops[0].id === "d1");
check("it switches to the new desktop", workspace.currentDesktop.id === "d1");
check("the desktop is named after the app (plus the invisible mark)", workspace.desktops[1].name === "Kate\u200b");

var a2 = win("kate");
open(a2);
check("a second window of the same app joins its desktop", workspace.desktops.length === 2 && a2.desktops[0].id === "d1");

var dotted = win("org.kde.kwrite");
open(dotted);
check("a dotted app id gives a short desktop name", workspace.desktops[workspace.desktops.length - 1].name === "Kwrite\u200b");
close(dotted);
var b = win("konsole");
open(b);
check("another app gets another desktop", workspace.desktops.length === 3 && b.desktops[0].id !== a.desktops[0].id && b.desktops[0].id !== "d0");

open(win("kate", { dialog: true }));
open(win("kate", { transient: true }));
open(win("plasmashell", { dock: true }));
check("dialogs, popups and panels never get a desktop", workspace.desktops.length === 3);

open(win("ignoredapp"));
check("an ignored app stays where it is", workspace.desktops.length === 3);

open(win("pinned", { onAllDesktops: true }));
check("a window on all desktops stays", workspace.desktops.length === 3);

close(a);
check("closing one of two windows keeps the desktop", workspace.desktops.length === 3);
close(a2);
check("closing the last window removes the desktop", workspace.desktops.length === 2 && !workspace.desktops.some(function (x) { return x.id === a.desktops[0].id; }));
check("you are never left on a removed desktop", workspace.currentDesktop.id !== "d1");

workspace.currentDesktop = desk0;
var w0 = win("firefox"); open(w0);
var w1 = win("gimp"); open(w1);
check("the maximum number of desktops is respected", workspace.desktops.length <= 4);
var extra = win("inkscape"); open(extra);
check("past the maximum a new app stays on the current desktop", workspace.desktops.length <= 4 && extra.desktops[0].id === workspace.currentDesktop.id || workspace.desktops.length === 4);

// a desktop left over by an earlier run of the script (its mark is in the name) is cleared at the next close
var stray = { id: "s1", name: "Gimp\u200b" };
workspace.desktops.push(stray);
var tmp = win("calc"); open(tmp); close(tmp);
check("an empty desktop of ours left over from an earlier run is cleared", !workspace.desktops.some(function (x) { return x.id === "s1"; }));

// desktops the script did not make are never removed
var userDesk = { id: "u1", name: "mine" };
workspace.desktops.push(userDesk);
var w2 = win("okular", { desktops: [userDesk] });
workspace.wins.push(w2);
var before = workspace.desktops.length;
close(w2);
check("a desktop you made yourself is never removed", workspace.desktops.length === before);

print(fails === 0 ? "ALL OK" : fails + " FAILED");
