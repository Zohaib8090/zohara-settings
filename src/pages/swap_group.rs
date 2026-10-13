//! Power & battery > Swap: add compressed memory swap (zram) or a swap file after installing, and see whether Hibernate
//! can work. The system changes are in `backend::swap` (one administrator prompt each); this is only the page.

use crate::backend::hibernate;
use crate::backend::swap::{self, Status};
use crate::backend::worker::in_background;
use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Text for the "In use now" row.
fn in_use_text(s: &Status) -> String {
    if s.devices.is_empty() {
        "No swap. When memory runs out, apps are closed or the computer slows to a crawl.".to_string()
    } else {
        s.devices.iter().map(swap::describe).collect::<Vec<_>>().join("\n")
    }
}

fn hibernate_text(s: &Status) -> (&'static str, String) {
    if hibernate::can_hibernate() {
        ("Ready", "Hibernate saves everything to the disk and turns the computer off completely.".into())
    } else if s.disk_swap_exists() && s.swap_kib() >= s.mem_kib {
        ("Not set up", "There is enough swap, but the start-up setting Hibernate needs is missing. Setting that up from here comes in a later update.".into())
    } else {
        ("Not available", format!("Hibernate needs a swap file or partition on the disk as big as memory ({}). Compressed memory does not count. Make a swap file in the row above.", swap::gib(s.mem_kib)))
    }
}

pub fn group() -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("Swap");
    g.set_description(Some("Spare room the computer uses when memory fills up. It helps small computers and big builds most. Changes ask for your password once."));

    let start = swap::status();
    let status = Rc::new(RefCell::new(start.clone()));
    let quiet = Rc::new(Cell::new(false)); // true while the page itself updates a row

    let info = adw::ActionRow::new();
    info.set_title(&format!("In use now (memory {})", swap::gib(start.mem_kib)));
    info.set_subtitle(&in_use_text(&start));
    info.set_subtitle_lines(0);
    info.set_activatable(false);
    g.add(&info);

    let zram = adw::SwitchRow::new();
    zram.set_title("Compressed memory (zram)");
    zram.set_subtitle("Keeps swap squeezed inside memory. Uses no disk space and is a big help on computers with little memory. The first time it downloads a small package.");
    zram.set_subtitle_lines(0);
    zram.set_active(start.zram_active());

    let file = adw::ComboRow::new();
    file.set_title("Swap file");
    file.set_subtitle_lines(0);

    let hib = adw::ActionRow::new();
    hib.set_title("Hibernate");
    hib.set_activatable(false);
    hib.set_subtitle_lines(0);

    let msg = adw::ActionRow::new();
    msg.set_title("Nothing changed yet");
    msg.set_subtitle("Each change asks for your password once.");
    msg.set_activatable(false);

    // Which sizes the file row offers; index 0 is always "No swap file", and a file that is already there is shown first.
    let options: Rc<RefCell<Vec<u64>>> = Rc::new(RefCell::new(Vec::new()));

    let refresh: Rc<dyn Fn()> = {
        let (info, zram, file, hib, status, options, quiet) = (info.clone(), zram.clone(), file.clone(), hib.clone(), status.clone(), options.clone(), quiet.clone());
        Rc::new(move || {
            let s = swap::status();
            quiet.set(true);
            info.set_title(&format!("In use now (memory {})", swap::gib(s.mem_kib)));
            info.set_subtitle(&in_use_text(&s));
            zram.set_active(s.zram_active());
            let current_gib = s.file_active().map(|d| (d.size_kib / 1024 / 1024).max(1)).unwrap_or(0);
            let offered = swap::sizes(s.mem_kib, s.free_bytes, current_gib);
            let mut labels = vec!["No swap file".to_string()];
            let mut gibs = vec![0u64];
            let mut selected = 0u32;
            if current_gib > 0 && !offered.iter().any(|o| o.1 == current_gib) {
                labels.push(format!("{current_gib} GiB (current)"));
                gibs.push(current_gib);
            }
            for (label, g) in offered {
                labels.push(label);
                gibs.push(g);
            }
            if current_gib > 0 {
                selected = gibs.iter().position(|g| *g == current_gib).unwrap_or(0) as u32;
            }
            file.set_model(Some(&gtk4::StringList::new(&labels.iter().map(String::as_str).collect::<Vec<_>>())));
            file.set_selected(selected);
            file.set_subtitle(&if offered_none(&gibs) {
                format!("Not enough free disk space to offer one ({} GiB must stay free).", swap::KEEP_FREE_GIB)
            } else if current_gib > 0 {
                "Lives in its own folder so system snapshots keep working. Choose another size to replace it.".to_string()
            } else {
                "Uses disk space. Hibernate needs one as big as memory.".to_string()
            });
            *options.borrow_mut() = gibs;
            let (title, why) = hibernate_text(&s);
            hib.set_title(&format!("Hibernate: {title}"));
            hib.set_subtitle(&why);
            *status.borrow_mut() = s;
            quiet.set(false);
        })
    };
    refresh();

    // Runs `script`; shows the result in the message row and re-reads the system either way.
    let run: Rc<dyn Fn(String, &'static str)> = {
        let (msg, refresh, zram, file) = (msg.clone(), refresh.clone(), zram.clone(), file.clone());
        Rc::new(move |script: String, what: &'static str| {
            msg.set_title("Working…");
            msg.set_subtitle(what);
            zram.set_sensitive(false);
            file.set_sensitive(false);
            let (msg, refresh, zram, file) = (msg.clone(), refresh.clone(), zram.clone(), file.clone());
            in_background(move || swap::run(&script), move |err| {
                match err {
                    None => {
                        msg.set_title("Done");
                        msg.set_subtitle("Stays this way after a restart.");
                    }
                    Some(e) => {
                        msg.set_title("Not changed");
                        msg.set_subtitle(&e);
                    }
                }
                refresh();
                zram.set_sensitive(true);
                file.set_sensitive(true);
            });
        })
    };

    {
        let (run, quiet) = (run.clone(), quiet.clone());
        zram.connect_active_notify(move |r| {
            if quiet.get() {
                return;
            }
            if r.is_active() {
                run(swap::zram_enable_script(), "Turning compressed memory on…");
            } else {
                run(swap::zram_disable_script(), "Turning compressed memory off…");
            }
        });
    }

    {
        let (run, quiet, options, status, refresh) = (run.clone(), quiet.clone(), options.clone(), status.clone(), refresh.clone());
        file.connect_selected_notify(move |row| {
            if quiet.get() {
                return;
            }
            let Some(&gib) = options.borrow().get(row.selected() as usize) else { return };
            let had = status.borrow().file_active().map(|d| (d.size_kib / 1024 / 1024).max(1)).unwrap_or(0);
            if gib == had {
                return;
            }
            let (title, body, button) = if gib == 0 {
                ("Remove the swap file?".to_string(), "The swap file is turned off and deleted. This needs enough free memory to hold what is in it now.".to_string(), "Remove")
            } else if had > 0 {
                (format!("Replace it with a {gib} GiB swap file?"), format!("The old swap file is removed and a new one of {gib} GiB is made. It uses {gib} GiB of disk space."), "Replace")
            } else {
                (format!("Make a {gib} GiB swap file?"), format!("It uses {gib} GiB of disk space and is switched on straight away. You can remove it here at any time."), "Make it")
            };
            let d = adw::AlertDialog::new(Some(&title), Some(&body));
            d.add_responses(&[("cancel", "Cancel"), ("go", button)]);
            d.set_response_appearance("go", if gib == 0 { adw::ResponseAppearance::Destructive } else { adw::ResponseAppearance::Suggested });
            d.set_default_response(Some("cancel"));
            d.set_close_response("cancel");
            let (run, refresh) = (run.clone(), refresh.clone());
            d.connect_response(None, move |_, r| {
                if r != "go" {
                    refresh(); // put the row back to what the system has
                    return;
                }
                let mut script = String::new();
                if had > 0 {
                    script += &swap::file_remove_script();
                }
                if gib > 0 {
                    script += &swap::file_create_script(gib);
                }
                run(script, if gib == 0 { "Removing the swap file…" } else { "Making the swap file… this can take a minute." });
            });
            d.present(Some(row));
        });
    }

    g.add(&zram);
    g.add(&file);
    g.add(&hib);
    g.add(&msg);
    g
}

fn offered_none(gibs: &[u64]) -> bool {
    gibs.iter().all(|g| *g == 0)
}
