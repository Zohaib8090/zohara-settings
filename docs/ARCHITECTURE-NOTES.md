# Settings: how the pieces fit (notes from 2026-10-06)

Read with `docs/UPDATES.md` (OS updates) and the `zohara` repo's `docs/HANDOFF-2026-10-06.md`.

* **Pages** are `src/pages/*.rs`, registered in `PAGES` (`src/main.rs`). Every page is wrapped by `build_page`, which also runs
  `pages::adopt_orphan_rows`: a row appended to a plain `Box` never reacts to clicks, so such rows are moved into a `ListBox`.
  Add rows with `adw::PreferencesGroup` or `pages::in_list` when you can; the safety net is there for the rest.
* **Search** (`src/search.rs`): `ENTRIES` is a hand-written index of pages and single options (title as shown on the page, the
  page label, extra keywords). Picking a result opens the page, scrolls to the row and flashes it. When you add or rename a
  setting, add or rename its `ENTRIES` row; the test `every_entry_is_on_its_page` fails if a title is not in the page sources.
* **Display** (`pages/display.rs`): `kscreen-doctor -j` to read, `kscreen-doctor output.<n>.mode.<id>` / `.scale.<x>` /
  `.rotation.<r>` to apply. Every picker goes through `guard_change`: apply, ask "Keep these display settings?" for 15 seconds,
  revert by itself otherwise. Failures from the tool are shown under the row.
* **Mouse / Touchpad** (`pages/input_devices.rs`): KWin's `org.kde.KWin.InputDevice` D-Bus API per device. Devices with no
  controls or on a virtual bus are hidden (listed in an expander). `classify_bus` reads where `/sys/class/input/eventN` points.
  **Identify** runs `pkexec libinput debug-events` for 8 seconds and marks the devices that moved (`parse_libinput_events`);
  needs `libinput-tools`.
* **Network / Wi-Fi** (`backend/wifi_diag.rs`, `pages/wifi_check.rs`): `diagnose(&Facts)` is pure and tested; `gather()` reads
  `/sys` (wireless interfaces, PCI class 0x0280 hardware and its driver, rfkill) and `nmcli`. The card is shown only when
  Wi-Fi is not fine. `data/zohara-wifi-resume` is a systemd-sleep hook that reloads the Wi-Fi driver if the interface is gone
  after resume (test: `tests/test-wifi-resume.sh`, also run in CI).
* **Default apps** (`pages/default_apps.rs`): GIO `AppInfo` for the MIME defaults plus `kdeglobals` `BrowserApplication`. The
  dropdown matches entries by their own label, never by list position.
* **Updates** (`src/sysupdate/`, `pages/updates.rs`): see `docs/UPDATES.md`.
* **Tests**: `cargo test` (in the `zs-img` container; the laptop lacks libadwaita headers) plus `sh tests/test-wifi-resume.sh`.
  UI changes are checked by running the app under Xvfb in the container or in the VM; see the handoff.
