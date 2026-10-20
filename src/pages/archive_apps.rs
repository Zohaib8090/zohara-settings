//! Settings > Apps > "Apps from archives": install a program that comes as a `.tar.gz` (or `.tar.xz`, `.tar.bz2`, `.tar.zst`),
//! see the ones installed this way, and remove them. The work is in `backend::archive_install`.

use crate::backend::archive_install as ai;
use crate::backend::worker::in_background;
use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use std::path::PathBuf;

fn message(w: &impl IsA<gtk4::Widget>, title: &str, body: &str) {
    let d = adw::AlertDialog::new(Some(title), Some(body));
    d.set_body_use_markup(false);
    d.add_response("ok", "OK");
    d.present(Some(w));
}

/// The group shown on the Apps page. It rebuilds itself after every change.
pub fn build() -> gtk4::Box {
    let holder = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    fill(&holder);
    holder
}

fn fill(holder: &gtk4::Box) {
    while let Some(c) = holder.first_child() {
        holder.remove(&c);
    }
    let g = adw::PreferencesGroup::new();
    g.set_title("Apps from archives");
    g.set_description(Some("For programs that come as a .tar.gz or similar file. Zohara looks inside first, then unpacks the app into its own folder and adds it to the Start menu. You can remove it here later."));
    let pick = gtk4::Button::with_label("Install from an archive…");
    pick.add_css_class("suggested-action");
    pick.set_valign(gtk4::Align::Center);
    {
        let holder = holder.clone();
        pick.connect_clicked(move |_| choose_file(&holder));
    }
    g.set_header_suffix(Some(&pick));

    let installed = ai::list_installed(&ai::Dirs::real());
    if installed.is_empty() {
        let r = adw::ActionRow::new();
        r.set_title("Nothing installed from an archive yet");
        r.set_activatable(false);
        g.add(&r);
    }
    for app in installed {
        let r = adw::ActionRow::new();
        r.set_use_markup(false);
        r.set_title(&app.name);
        r.set_subtitle(&app.dir.to_string_lossy());
        r.set_activatable(false);
        let remove = gtk4::Button::with_label("Remove");
        remove.add_css_class("destructive-action");
        remove.set_valign(gtk4::Align::Center);
        {
            let (holder, slug, name) = (holder.clone(), app.slug.clone(), app.name.clone());
            remove.connect_clicked(move |b| confirm_remove(b, &holder, &slug, &name));
        }
        r.add_suffix(&remove);
        g.add(&r);
    }
    holder.append(&g);
}

fn confirm_remove(from: &gtk4::Button, holder: &gtk4::Box, slug: &str, name: &str) {
    let d = adw::AlertDialog::new(Some(&format!("Remove {name}?")), Some("Its folder and its Start menu entry are deleted. Files you made with it elsewhere are not touched."));
    d.set_body_use_markup(false);
    d.add_responses(&[("cancel", "Cancel"), ("remove", "Remove")]);
    d.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
    d.set_default_response(Some("cancel"));
    d.set_close_response("cancel");
    let (holder, slug) = (holder.clone(), slug.to_string());
    d.connect_response(None, move |_, id| {
        if id != "remove" {
            return;
        }
        let (holder, s) = (holder.clone(), slug.clone());
        in_background(
            move || ai::remove(&s, &ai::Dirs::real()),
            move |r| {
                if let Err(e) = r {
                    message(&holder, "Couldn't remove it", &e);
                }
                fill(&holder);
            },
        );
    });
    d.present(Some(from));
}

fn choose_file(holder: &gtk4::Box) {
    let filter = gtk4::FileFilter::new();
    filter.set_name(Some("Archives (.tar.gz, .tar.xz, .tar.bz2, .tar.zst)"));
    for p in ["*.tar.gz", "*.tgz", "*.tar.xz", "*.txz", "*.tar.bz2", "*.tbz2", "*.tar.zst", "*.tar"] {
        filter.add_pattern(p);
    }
    let filters = gtk4::gio::ListStore::new::<gtk4::FileFilter>();
    filters.append(&filter);
    let dialog = gtk4::FileDialog::builder().title("Choose an archive").filters(&filters).build();
    let window = holder.root().and_then(|r| r.downcast::<gtk4::Window>().ok());
    let holder = holder.clone();
    dialog.open(window.as_ref(), gtk4::gio::Cancellable::NONE, move |res| {
        if let Some(path) = res.ok().and_then(|f| f.path()) {
            inspect_and_offer(&holder, path);
        }
    });
}

fn inspect_and_offer(holder: &gtk4::Box, path: PathBuf) {
    let holder = holder.clone();
    let p = path.clone();
    in_background(move || ai::inspect(&p), move |res| match res {
        Err(e) => message(&holder, "Can't read this archive", &e),
        Ok(info) => {
            if let Some(problem) = &info.problem {
                message(&holder, "This archive was not unpacked", problem);
            } else if info.kind != ai::Kind::PortableApp {
                message(&holder, "Not an app Zohara can install this way", &ai::explain(&info));
            } else {
                offer_install(&holder, path, info);
            }
        }
    });
}

fn offer_install(holder: &gtk4::Box, path: PathBuf, info: ai::Inspection) {
    let d = adw::AlertDialog::new(
        Some(&format!("Install {}?", info.suggested_name)),
        Some(&format!("{} It takes about {} once unpacked.", ai::explain(&info), ai::readable_size(info.total_size))),
    );
    d.set_body_use_markup(false);
    let extra = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    let name = gtk4::Entry::new();
    name.set_text(&info.suggested_name);
    name.set_placeholder_text(Some("Name in the Start menu"));
    extra.append(&name);
    let program = gtk4::DropDown::from_strings(&info.candidates.iter().map(String::as_str).collect::<Vec<_>>());
    if info.candidates.len() > 1 {
        extra.append(&gtk4::Label::builder().label("Starts with:").halign(gtk4::Align::Start).build());
        extra.append(&program);
    }
    d.set_extra_child(Some(&extra));
    d.add_responses(&[("cancel", "Cancel"), ("install", "Install")]);
    d.set_response_appearance("install", adw::ResponseAppearance::Suggested);
    d.set_default_response(Some("install"));
    d.set_close_response("cancel");
    let holder2 = holder.clone();
    d.connect_response(None, move |_, id| {
        if id != "install" {
            return;
        }
        let exec = info.candidates.get(program.selected() as usize).cloned().unwrap_or_default();
        let (name, path, holder) = (name.text().to_string(), path.clone(), holder2.clone());
        let n = name.clone();
        in_background(
            move || ai::install(&path, &n, &exec, &ai::Dirs::real()),
            move |r| {
                match r {
                    Ok(done) => message(&holder, &format!("{} is installed", done.name), "It is in the Start menu. You can remove it from this list later."),
                    Err(e) => message(&holder, "Couldn't install the app", &e),
                }
                fill(&holder);
            },
        );
    });
    d.present(Some(holder));
}
