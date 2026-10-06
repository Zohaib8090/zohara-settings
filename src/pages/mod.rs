pub mod home;
pub mod system;
pub mod bluetooth;
pub mod network;
pub mod network_extra;
pub mod wifi_check;
pub mod speedtest;
pub mod personalization;
pub mod themes;
pub mod lighting;
pub mod lockscreen;
pub mod text_input;
pub mod start_menu;
pub mod apps;
pub mod offline_maps;
pub mod web_apps;
pub mod accounts;
pub mod time_language;
pub mod troubleshoot;
pub mod gaming;
pub mod game_libraries;
pub mod my_games;
pub mod accessibility;
pub mod printers;
pub mod privacy;
pub mod updates;
pub mod display;
pub mod display_layout;
pub mod sound;
pub mod notifications;
pub mod power;
pub mod storage;
pub mod input_devices;
pub mod mouse;
pub mod touchpad;
pub mod keyboard;
pub mod shortcuts;
pub mod default_apps;
pub mod advanced;
pub mod zohara_link;

/// Switch the Settings window to the page with this sidebar label.
pub fn goto(widget: &impl gtk4::prelude::IsA<gtk4::Widget>, label: &str) {
    use gtk4::prelude::*;
    use gtk4::glib::prelude::ToVariant;
    let _ = widget.as_ref().activate_action("win.goto", Some(&label.to_variant()));
}

/// An `ActionRow` only reacts to clicks and Enter while it sits inside a `ListBox` (the list is what turns a click
/// into "row activated"). Rows laid out in a plain `Box` highlight on hover but do nothing when clicked, so such
/// a row gets a one-row list of its own. Rows added to an `adw::PreferencesGroup` do not need this.
pub fn in_list(row: &impl gtk4::prelude::IsA<gtk4::Widget>) -> gtk4::ListBox {
    use gtk4::prelude::*;
    let list = gtk4::ListBox::new();
    list.set_selection_mode(gtk4::SelectionMode::None);
    list.set_css_classes(&["win11-action-list"]);
    list.append(row);
    list
}

/// Rows outside a list never react to clicks (see `in_list`), and a page that appends its rows to a plain `Box` ends
/// up with rows that highlight on hover and do nothing: Display's Resolution and Scale, Sound's Output and Input
/// pickers, the Wi-Fi expander and others were all like that. This walks a freshly built page and moves every such
/// row into a list, keeping consecutive rows together in one list so they still look like one card. Rows that already
/// sit in a list (every `PreferencesGroup`, `in_list`) are left alone. Returns how many rows it had to move.
pub fn adopt_orphan_rows(root: &gtk4::Widget) -> usize {
    use gtk4::prelude::*;
    let is_orphan = |w: &gtk4::Widget| w.is::<gtk4::ListBoxRow>() && w.ancestor(gtk4::ListBox::static_type()).is_none();

    // Collect every Box first: the tree changes while rows are moved.
    let mut boxes: Vec<gtk4::Box> = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(w) = stack.pop() {
        if let Some(b) = w.downcast_ref::<gtk4::Box>() {
            boxes.push(b.clone());
        }
        let mut c = w.first_child();
        while let Some(child) = c {
            c = child.next_sibling();
            stack.push(child);
        }
    }

    let mut moved = 0;
    for b in boxes {
        // Runs of consecutive orphan rows among this box's direct children.
        let mut runs: Vec<Vec<gtk4::Widget>> = Vec::new();
        let mut current: Vec<gtk4::Widget> = Vec::new();
        let mut c = b.first_child();
        while let Some(child) = c {
            c = child.next_sibling();
            if is_orphan(&child) {
                current.push(child);
            } else if !current.is_empty() {
                runs.push(std::mem::take(&mut current));
            }
        }
        if !current.is_empty() {
            runs.push(current);
        }
        for run in runs {
            let list = gtk4::ListBox::new();
            list.set_selection_mode(gtk4::SelectionMode::None);
            list.set_css_classes(&["win11-action-list"]);
            // Take the place of the run's first row, then move the rows in.
            let prev = run[0].prev_sibling();
            b.remove(&run[0]);
            b.insert_child_after(&list, prev.as_ref());
            list.append(&run[0]);
            for row in &run[1..] {
                b.remove(row);
                list.append(row);
            }
            moved += run.len();
        }
    }
    moved
}
