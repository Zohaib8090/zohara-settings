//! Settings > Apps > "Mac and iPhone apps": pick a `.app` folder, `.ipa`, `.dmg` or Mac program file and be told, in plain words,
//! whether Zohara has any chance of running it. Nothing is installed or run; the work is in `backend::mac_check`.

use crate::backend::mac_check as mc;
use crate::backend::worker::in_background;
use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use std::path::PathBuf;

fn show(from: &gtk4::Widget, path: PathBuf) {
    let from = from.clone();
    in_background(move || mc::check(&path), move |r| {
        let d = adw::AlertDialog::new(Some(&mc::headline(&r)), Some(&mc::explain(&r)));
        d.set_heading_use_markup(false);
        d.set_body_use_markup(false);
        d.add_response("ok", "OK");
        d.present(Some(&from));
    });
}

fn choose(from: &gtk4::Widget, folder: bool) {
    let window = from.root().and_then(|r| r.downcast::<gtk4::Window>().ok());
    let from = from.clone();
    let dialog = gtk4::FileDialog::builder().title(if folder { "Choose a Mac app (the .app folder)" } else { "Choose a .ipa, .dmg or Mac program" }).build();
    let done = move |res: Result<gtk4::gio::File, glib::Error>| {
        if let Some(path) = res.ok().and_then(|f| f.path()) {
            show(&from, path);
        }
    };
    if folder {
        dialog.select_folder(window.as_ref(), gtk4::gio::Cancellable::NONE, done);
    } else {
        dialog.open(window.as_ref(), gtk4::gio::Cancellable::NONE, done);
    }
}

pub fn build() -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("Mac and iPhone apps");
    g.set_description(Some("Zohara cannot run Mac apps yet. This tells you whether an app has a chance, by reading what it says it needs. Nothing is installed or started."));
    let row = adw::ActionRow::new();
    row.set_title("Check an app");
    row.set_subtitle("A Mac app (.app folder), an iPhone app (.ipa), a disk image (.dmg) or a Mac program file");
    row.set_activatable(false);
    let folder = gtk4::Button::with_label("Mac app…");
    folder.set_valign(gtk4::Align::Center);
    let file = gtk4::Button::with_label("File…");
    file.set_valign(gtk4::Align::Center);
    folder.connect_clicked(|b| choose(b.upcast_ref(), true));
    file.connect_clicked(|b| choose(b.upcast_ref(), false));
    row.add_suffix(&folder);
    row.add_suffix(&file);
    g.add(&row);
    g
}
