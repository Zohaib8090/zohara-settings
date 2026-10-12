//! USB drives, SD cards and external disks, on the Bluetooth & devices page: what is plugged in, a page for each drive,
//! format / erase / rename / check and repair / safely remove, and "make a bootable USB drive". The rules (what may be
//! changed, who the drive is, label rules) and the changes themselves are in `backend/drives.rs`; this is only the screens.
//!
//! Everything that destroys data asks first, names the drive and what is on it, and needs the word ERASE typed. The drive is
//! looked up again right before the change; if it is not the very same one, nothing happens.

use crate::backend::drives::{self as dr, Drive, FormatPlan, Identity, Kind, Progress, Udisks};
use crate::backend::worker::in_background;
use adw::prelude::*;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::sync::Arc;

const CONFIRM_WORD: &str = "ERASE";

fn window_of(w: &impl IsA<gtk4::Widget>) -> Option<gtk4::Window> {
    w.root().and_then(|r| r.downcast::<gtk4::Window>().ok())
}

fn message(from: &impl IsA<gtk4::Widget>, title: &str, body: &str) {
    let d = adw::AlertDialog::new(Some(title), Some(body));
    d.add_response("ok", "OK");
    d.present(Some(from));
}

/// A window that says "working" and cannot be closed until the work is done.
fn busy(from: &impl IsA<gtk4::Widget>, title: &str, body: &str) -> adw::AlertDialog {
    let d = adw::AlertDialog::new(Some(title), Some(body));
    let spin = gtk4::Spinner::new();
    spin.set_spinning(true);
    spin.set_size_request(32, 32);
    d.set_extra_child(Some(&spin));
    d.set_can_close(false);
    d.present(Some(from));
    d
}

fn dim(text: &str) -> gtk4::Label {
    gtk4::Label::builder().label(text).xalign(0.0).wrap(true).css_classes(vec!["dim-label".to_string()]).build()
}

// ── The group on the Bluetooth & devices page ────────────────────────────────

fn signature(drives: &[Drive]) -> String {
    drives
        .iter()
        .map(|d| {
            let fs: Vec<String> = d.filesystems().iter().map(|p| format!("{}:{}:{}:{}", p.path, p.label, p.fstype, p.mounted())).collect();
            format!("{}|{}|{}|{}|{}", d.path, d.size, d.serial, d.system, fs.join(","))
        })
        .collect::<Vec<_>>()
        .join(";")
}

fn drive_row(d: &Drive) -> adw::ActionRow {
    let row = adw::ActionRow::new();
    row.set_title(&glib::markup_escape_text(&d.title()));
    let parts = d.filesystems().len();
    let mut sub = d.summary();
    if d.system {
        sub.push_str(" · In use by Zohara");
    } else if d.mounted_paths().is_empty() {
        sub.push_str(if parts == 0 { " · Empty or not recognised" } else { " · Not open" });
    } else {
        sub.push_str(" · Open");
    }
    row.set_subtitle(&sub);
    row.add_prefix(&gtk4::Image::from_icon_name(d.kind.icon()));
    row.add_suffix(&gtk4::Image::from_icon_name("go-next-symbolic"));
    row.set_activatable(true);
    let path = d.path.clone();
    row.connect_activated(move |r| open(r, &path));
    row
}

/// The "USB & storage devices" group. It follows plugging and unplugging while the page is open.
pub fn group(page: &gtk4::Box) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("USB & storage devices");
    g.set_description(Some("USB drives, SD cards and external disks. Plug one in and it shows up here."));

    let rows: Rc<RefCell<Vec<gtk4::Widget>>> = Default::default();
    let last: Rc<RefCell<String>> = Default::default();
    let fill = {
        let (g, rows, last, page) = (g.clone(), rows.clone(), last.clone(), page.clone());
        Rc::new(move |drives: Vec<Drive>| {
            let outside: Vec<Drive> = drives.into_iter().filter(|d| d.kind != Kind::Internal).collect();
            let sig = signature(&outside);
            if *last.borrow() == sig && !rows.borrow().is_empty() {
                return;
            }
            *last.borrow_mut() = sig;
            for r in rows.borrow_mut().drain(..) {
                g.remove(&r);
            }
            if outside.is_empty() {
                let r = adw::ActionRow::new();
                r.set_title("Nothing plugged in");
                r.set_subtitle("Plug in a USB drive, SD card or external disk");
                r.add_prefix(&gtk4::Image::from_icon_name("drive-removable-media-usb-symbolic"));
                g.add(&r);
                rows.borrow_mut().push(r.upcast());
            }
            for d in &outside {
                let r = drive_row(d);
                g.add(&r);
                rows.borrow_mut().push(r.upcast());
            }
            let boot = adw::ActionRow::new();
            boot.set_title("Make a bootable USB drive");
            boot.set_subtitle("Write an ISO or disk image to a USB drive, to install or try another system");
            boot.add_prefix(&gtk4::Image::from_icon_name("media-optical-symbolic"));
            boot.add_suffix(&gtk4::Image::from_icon_name("go-next-symbolic"));
            boot.set_activatable(true);
            boot.connect_activated(|r| bootable_dialog(r, None));
            g.add(&boot);
            rows.borrow_mut().push(boot.upcast());
            let inner = adw::ActionRow::new();
            inner.set_title("Disks inside this computer");
            inner.set_subtitle("Space used, cleanup and restore points");
            inner.add_prefix(&gtk4::Image::from_icon_name("drive-harddisk-symbolic"));
            inner.add_suffix(&gtk4::Image::from_icon_name("go-next-symbolic"));
            inner.set_activatable(true);
            let page = page.clone();
            inner.connect_activated(move |_| super::goto(&page, "Storage"));
            g.add(&inner);
            rows.borrow_mut().push(inner.upcast());
        })
    };
    {
        let fill = fill.clone();
        super::live::every(&g, 3, move || {
            let fill = fill.clone();
            in_background(dr::list, move |d| fill(d));
        });
    }
    g
}

// ── The page of one drive ────────────────────────────────────────────────────

/// Opens the page of the drive at `path` (for example `/dev/sdb`).
pub fn open(parent: &impl IsA<gtk4::Widget>, path: &str) {
    let dialog = adw::Dialog::new();
    dialog.set_title("Drive");
    dialog.set_content_width(600);
    dialog.set_content_height(700);

    let holder = gtk4::Box::new(gtk4::Orientation::Vertical, 18);
    holder.set_margin_top(12);
    holder.set_margin_bottom(24);
    holder.set_margin_start(18);
    holder.set_margin_end(18);
    let clamp = adw::Clamp::new();
    clamp.set_maximum_size(560);
    clamp.set_child(Some(&holder));
    let scroll = gtk4::ScrolledWindow::builder().hscrollbar_policy(gtk4::PolicyType::Never).vexpand(true).build();
    scroll.set_child(Some(&clamp));
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&scroll));
    dialog.set_child(Some(&view));

    let reload: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Default::default();
    let path = path.to_string();
    let run_reload = {
        let (reload, holder, dialog, path) = (reload.clone(), holder.clone(), dialog.clone(), path.clone());
        Rc::new(move || {
            let (reload, holder, dialog, path2) = (reload.clone(), holder.clone(), dialog.clone(), path.clone());
            in_background(
                {
                    let path = path.clone();
                    move || dr::find(&path)
                },
                move |d| match d {
                    Some(d) => {
                        let again = reload.borrow().clone();
                        fill_detail(&holder, &dialog, &d, again.unwrap_or_else(|| Rc::new(|| {})));
                    }
                    None => {
                        let _ = path2;
                        dialog.force_close();
                    }
                },
            );
        })
    };
    *reload.borrow_mut() = Some(run_reload.clone());
    dialog.present(Some(parent));
    run_reload();
}

fn clear(b: &gtk4::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

fn info_row(title: &str, value: &str) -> adw::ActionRow {
    let r = adw::ActionRow::new();
    r.set_title(title);
    let v = gtk4::Label::new(Some(value));
    v.add_css_class("dim-label");
    v.set_selectable(true);
    v.set_xalign(1.0);
    v.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
    v.set_max_width_chars(34);
    v.set_valign(gtk4::Align::Center);
    r.add_suffix(&v);
    r
}

fn fill_detail(holder: &gtk4::Box, dialog: &adw::Dialog, d: &Drive, reload: Rc<dyn Fn()>) {
    clear(holder);
    dialog.set_title(&d.title());

    let head = gtk4::Box::new(gtk4::Orientation::Horizontal, 14);
    let img = gtk4::Image::from_icon_name(d.kind.icon());
    img.set_pixel_size(64);
    head.append(&img);
    let titles = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    titles.set_valign(gtk4::Align::Center);
    titles.append(&gtk4::Label::builder().label(&d.title()).xalign(0.0).wrap(true).css_classes(vec!["title-2".to_string()]).build());
    titles.append(&dim(&d.summary()));
    head.append(&titles);
    holder.append(&head);

    if let Some(why) = d.locked_reason() {
        let banner = adw::PreferencesGroup::new();
        let r = adw::ActionRow::new();
        r.set_title("Can't be changed here");
        r.set_subtitle(why);
        r.add_prefix(&gtk4::Image::from_icon_name("changes-prevent-symbolic"));
        banner.add(&r);
        holder.append(&banner);
    }

    let can = d.can_modify();
    let identity = d.identity();

    // What is on it.
    let on = adw::PreferencesGroup::new();
    on.set_title("On this drive");
    let fss = d.filesystems();
    if fss.is_empty() {
        let r = adw::ActionRow::new();
        r.set_title("Nothing recognised");
        r.set_subtitle("The drive is empty, or uses a format this computer doesn't know");
        on.add(&r);
    }
    for p in fss {
        let ex = adw::ExpanderRow::new();
        ex.set_title(&glib::markup_escape_text(if p.label.is_empty() { "Unnamed" } else { &p.label }));
        let used = p.used.map(|u| format!("{} used of {}", dr::human(u), dr::human(p.size))).unwrap_or_else(|| dr::human(p.size));
        let fsname = dr::fs_by_id(&p.fstype).map(|f| f.title.to_string()).unwrap_or_else(|| p.fstype.clone());
        ex.set_subtitle(&format!("{fsname} · {used}{}", if p.mounted() { " · Open" } else { "" }));
        ex.add_prefix(&gtk4::Image::from_icon_name("folder-symbolic"));

        // open / mount
        let open_row = adw::ActionRow::new();
        open_row.set_title(if p.mounted() { "Open in Files" } else { "Mount" });
        open_row.set_activatable(true);
        open_row.add_suffix(&gtk4::Image::from_icon_name("go-next-symbolic"));
        {
            let (dev, mount, reload, dialog) = (p.path.clone(), p.mountpoints.first().cloned(), reload.clone(), dialog.clone());
            open_row.connect_activated(move |r| match &mount {
                Some(m) => {
                    let _ = std::process::Command::new("xdg-open").arg(m).spawn();
                }
                None => {
                    let (dev, reload, r2) = (dev.clone(), reload.clone(), r.clone());
                    in_background(
                        move || Udisks::new().mount(&dev),
                        move |res| {
                            if let Err(e) = res {
                                message(&r2, "Couldn't open the drive", &e);
                            }
                            reload();
                        },
                    );
                    let _ = &dialog;
                }
            });
        }
        ex.add_row(&open_row);

        if p.mounted() {
            let un = adw::ActionRow::new();
            un.set_title("Unmount");
            un.set_subtitle("Close the drive without removing it");
            un.set_activatable(true);
            let (dev, reload) = (p.path.clone(), reload.clone());
            un.connect_activated(move |r| {
                let (dev, reload, r2) = (dev.clone(), reload.clone(), r.clone());
                in_background(
                    move || Udisks::new().unmount(&dev),
                    move |res| {
                        if let Err(e) = res {
                            message(&r2, "Couldn't unmount the drive", &e);
                        }
                        reload();
                    },
                );
            });
            ex.add_row(&un);
        }

        if can && !p.fstype.is_empty() {
            let rename = adw::ActionRow::new();
            rename.set_title("Rename…");
            rename.set_activatable(true);
            rename.add_suffix(&gtk4::Image::from_icon_name("go-next-symbolic"));
            {
                let (dev, fs, label, reload) = (p.path.clone(), p.fstype.clone(), p.label.clone(), reload.clone());
                rename.connect_activated(move |r| rename_dialog(r, &dev, &fs, &label, reload.clone()));
            }
            ex.add_row(&rename);

            let check = adw::ActionRow::new();
            check.set_title("Check and repair…");
            check.set_subtitle("Look for file system errors");
            check.set_activatable(true);
            check.add_suffix(&gtk4::Image::from_icon_name("go-next-symbolic"));
            {
                let (dev, reload) = (p.path.clone(), reload.clone());
                check.connect_activated(move |r| check_dialog(r, &dev, reload.clone()));
            }
            ex.add_row(&check);

            let format = adw::ActionRow::new();
            format.set_title("Format…");
            format.set_subtitle("Erase this part and start fresh");
            format.set_activatable(true);
            format.add_css_class("error");
            format.add_suffix(&gtk4::Image::from_icon_name("go-next-symbolic"));
            {
                let (d2, target, id, reload) = (d.clone(), p.path.clone(), identity.clone(), reload.clone());
                format.connect_activated(move |r| erase_dialog(r, &d2, Some(target.clone()), id.clone(), reload.clone()));
            }
            ex.add_row(&format);
        }
        on.add(&ex);
    }
    holder.append(&on);

    if can {
        let acts = adw::PreferencesGroup::new();
        acts.set_title("Drive");
        let remove = adw::ActionRow::new();
        remove.set_title("Safely remove");
        remove.set_subtitle("Close everything on the drive, then it is safe to unplug");
        remove.add_prefix(&gtk4::Image::from_icon_name("media-eject-symbolic"));
        remove.set_activatable(true);
        {
            let (d2, dialog) = (d.clone(), dialog.clone());
            remove.connect_activated(move |r| {
                let b = busy(r, "Closing the drive…", "Don't unplug it yet.");
                let (d3, dialog, r2) = (d2.clone(), dialog.clone(), r.clone());
                in_background(
                    move || Udisks::new().safely_remove(&d3),
                    move |res| {
                        b.force_close();
                        match res {
                            Ok(()) => {
                                message(&r2, "It is safe to unplug the drive", "Nothing is using it any more.");
                                dialog.force_close();
                            }
                            Err(e) => message(&r2, "Couldn't close the drive", &e),
                        }
                    },
                );
            });
        }
        acts.add(&remove);

        let erase = adw::ActionRow::new();
        erase.set_title("Erase the whole drive…");
        erase.set_subtitle("Remove everything, make a new partition table and one empty drive");
        erase.add_prefix(&gtk4::Image::from_icon_name("edit-clear-all-symbolic"));
        erase.add_suffix(&gtk4::Image::from_icon_name("go-next-symbolic"));
        erase.set_activatable(true);
        {
            let (d2, id, reload) = (d.clone(), identity.clone(), reload.clone());
            erase.connect_activated(move |r| erase_dialog(r, &d2, None, id.clone(), reload.clone()));
        }
        acts.add(&erase);

        let boot = adw::ActionRow::new();
        boot.set_title("Make this a bootable USB drive…");
        boot.set_subtitle("Write an ISO or disk image to it");
        boot.add_prefix(&gtk4::Image::from_icon_name("media-optical-symbolic"));
        boot.add_suffix(&gtk4::Image::from_icon_name("go-next-symbolic"));
        boot.set_activatable(true);
        {
            let path = d.path.clone();
            boot.connect_activated(move |r| bootable_dialog(r, Some(path.clone())));
        }
        acts.add(&boot);
        holder.append(&acts);
    }

    let about = adw::PreferencesGroup::new();
    about.set_title("About this drive");
    about.add(&info_row("Size", &dr::human(d.size)));
    if !d.model.trim().is_empty() {
        about.add(&info_row("Model", d.model.trim()));
    }
    if !d.tran.is_empty() {
        about.add(&info_row("Connection", &d.tran.to_uppercase()));
    }
    if !d.serial.is_empty() {
        about.add(&info_row("Serial number", &d.serial));
    }
    let table = if d.pttype.is_empty() { "None".to_string() } else { d.pttype.to_uppercase() };
    about.add(&info_row("Partition table", &table));
    about.add(&info_row("Device", &d.path));
    holder.append(&about);
}

// ── Rename ───────────────────────────────────────────────────────────────────

fn rename_dialog(from: &impl IsA<gtk4::Widget>, dev: &str, fs: &str, current: &str, reload: Rc<dyn Fn()>) {
    let d = adw::AlertDialog::new(Some("Rename the drive"), Some("The name shows up in Files and on other computers."));
    let entry = adw::EntryRow::new();
    entry.set_title("Name");
    entry.set_text(current);
    let group = adw::PreferencesGroup::new();
    group.add(&entry);
    d.set_extra_child(Some(&group));
    d.add_responses(&[("cancel", "Cancel"), ("rename", "Rename")]);
    d.set_response_appearance("rename", adw::ResponseAppearance::Suggested);
    d.set_default_response(Some("rename"));
    let fs = fs.to_string();
    {
        let (d2, fs2) = (d.clone(), fs.clone());
        entry.connect_changed(move |e| {
            let ok = dr::normalize_label(&fs2, &e.text()).is_ok();
            d2.set_response_enabled("rename", ok);
            if ok { e.remove_css_class("error") } else { e.add_css_class("error") }
        });
    }
    let (dev, from2) = (dev.to_string(), from.as_ref().clone());
    d.connect_response(None, move |_, r| {
        if r != "rename" {
            return;
        }
        let Ok(label) = dr::normalize_label(&fs, &entry.text()) else { return };
        let (dev, reload, from3) = (dev.clone(), reload.clone(), from2.clone());
        in_background(
            move || Udisks::new().set_label(&dev, &label),
            move |res| {
                if let Err(e) = res {
                    message(&from3, "Couldn't rename the drive", &e);
                }
                reload();
            },
        );
    });
    d.present(Some(from));
}

// ── Check and repair ─────────────────────────────────────────────────────────

fn check_dialog(from: &impl IsA<gtk4::Widget>, dev: &str, reload: Rc<dyn Fn()>) {
    let b = busy(from, "Checking the drive…", "This can take a few minutes on a big drive. Don't unplug it.");
    let (dev2, from2, reload2) = (dev.to_string(), from.as_ref().clone(), reload.clone());
    in_background(
        {
            let dev = dev.to_string();
            move || {
                let ud = Udisks::new();
                ud.unmount(&dev)?;
                ud.check(&dev, false)
            }
        },
        move |res| {
            b.force_close();
            match res {
                Ok(true) => {
                    message(&from2, "No problems found", "The drive's file system is in good shape.");
                    reload2();
                }
                Ok(false) => {
                    let d = adw::AlertDialog::new(Some("Problems found"), Some("The file system has errors. Repairing can fix them, but files that were already damaged may stay lost."));
                    d.add_responses(&[("cancel", "Not now"), ("repair", "Repair")]);
                    d.set_response_appearance("repair", adw::ResponseAppearance::Suggested);
                    let (dev3, from3, reload3) = (dev2.clone(), from2.clone(), reload2.clone());
                    d.connect_response(None, move |_, r| {
                        if r != "repair" {
                            return;
                        }
                        let b = busy(&from3, "Repairing the drive…", "Don't unplug it.");
                        let (from4, reload4) = (from3.clone(), reload3.clone());
                        let dev4 = dev3.clone();
                        in_background(
                            move || Udisks::new().check(&dev4, true),
                            move |res| {
                                b.force_close();
                                match res {
                                    Ok(true) => message(&from4, "Repaired", "The file system was repaired."),
                                    Ok(false) => message(&from4, "Couldn't repair everything", "Copy what you can to your computer, then format the drive."),
                                    Err(e) => message(&from4, "Couldn't repair the drive", &e),
                                }
                                reload4();
                            },
                        );
                    });
                    d.present(Some(&from2));
                }
                Err(e) => message(&from2, "Couldn't check the drive", &e),
            }
        },
    );
}

// ── Format / erase ───────────────────────────────────────────────────────────

/// `target`: a partition of the drive (`Some`) or the whole drive (`None`).
fn erase_dialog(from: &impl IsA<gtk4::Widget>, d: &Drive, target: Option<String>, identity: Identity, reload: Rc<dyn Fn()>) {
    let whole = target.is_none();
    let what = if whole { "Erase the whole drive?" } else { "Format this part of the drive?" };
    let lost = dr::what_will_be_lost(d);
    let body = format!(
        "{} ({}) will be erased. Everything on it is lost for good:\n• {}",
        d.title(),
        dr::human(d.size),
        lost.join("\n• ")
    );
    let dlg = adw::AlertDialog::new(Some(what), Some(&body));

    let form = gtk4::Box::new(gtk4::Orientation::Vertical, 10);
    let group = adw::PreferencesGroup::new();
    let fss = dr::available_filesystems();
    let titles: Vec<&str> = fss.iter().map(|f| f.title).collect();
    let fs_row = adw::ComboRow::new();
    fs_row.set_title("Format as");
    fs_row.set_model(Some(&gtk4::StringList::new(&titles)));
    let name_row = adw::EntryRow::new();
    name_row.set_title("Name (optional)");
    let mode_row = adw::ComboRow::new();
    mode_row.set_title("How");
    mode_row.set_model(Some(&gtk4::StringList::new(&["Quick", "Overwrite everything with zeros (slow)"])));
    let confirm = adw::EntryRow::new();
    confirm.set_title(&format!("Type {CONFIRM_WORD} to confirm"));
    group.add(&fs_row);
    group.add(&name_row);
    group.add(&mode_row);
    group.add(&confirm);
    form.append(&group);
    let note = dim("");
    form.append(&note);
    dlg.set_extra_child(Some(&form));
    dlg.add_responses(&[("cancel", "Cancel"), ("erase", if whole { "Erase drive" } else { "Format" })]);
    dlg.set_response_appearance("erase", adw::ResponseAppearance::Destructive);
    dlg.set_response_enabled("erase", false);
    dlg.set_close_response("cancel");

    let fss = Rc::new(fss);
    let selected_fs = {
        let (fss, fs_row) = (fss.clone(), fs_row.clone());
        move || fss.get(fs_row.selected() as usize).map(|f| f.id).unwrap_or("exfat")
    };
    let validate = {
        let (dlg, note, name_row, confirm, fss, fs_row, d) =
            (dlg.clone(), note.clone(), name_row.clone(), confirm.clone(), fss.clone(), fs_row.clone(), d.clone());
        Rc::new(move || {
            let f = fss.get(fs_row.selected() as usize).copied();
            let Some(f) = f else { return };
            let table = if whole { format!(" A {} partition table is used.", if dr::table_for(f.id, d.size) == "dos" { "MBR" } else { "GPT" }) } else { String::new() };
            let label = dr::normalize_label(f.id, &name_row.text());
            let typed = confirm.text().trim() == CONFIRM_WORD;
            match &label {
                Ok(_) => {
                    note.set_text(&format!("{}{table}", f.note));
                    name_row.remove_css_class("error");
                }
                Err(e) => {
                    note.set_text(e);
                    name_row.add_css_class("error");
                }
            }
            dlg.set_response_enabled("erase", label.is_ok() && typed);
        })
    };
    for f in [name_row.clone().upcast::<gtk4::Widget>(), confirm.clone().upcast()] {
        let v = validate.clone();
        f.downcast_ref::<adw::EntryRow>().unwrap().connect_changed(move |_| v());
    }
    {
        let v = validate.clone();
        fs_row.connect_selected_notify(move |_| v());
    }
    validate();

    let (from2, d2) = (from.as_ref().clone(), d.clone());
    dlg.connect_response(None, move |_, r| {
        if r != "erase" {
            return;
        }
        let plan = FormatPlan { fs: selected_fs().to_string(), label: name_row.text().to_string(), zero_fill: mode_row.selected() == 1 };
        let (identity, target, reload, from3, d3) = (identity.clone(), target.clone(), reload.clone(), from2.clone(), d2.clone());
        let b = busy(
            &from2,
            if target.is_none() { "Erasing the drive…" } else { "Formatting…" },
            if plan.zero_fill { "Overwriting the drive with zeros takes a while. Don't unplug it." } else { "Don't unplug the drive." },
        );
        in_background(
            move || match &target {
                Some(t) => dr::format_partition(Udisks::new(), &identity, t, &plan),
                None => dr::erase_drive(Udisks::new(), &identity, &plan),
            },
            move |res| {
                b.force_close();
                match res {
                    Ok(()) => message(&from3, "Done", &format!("{} is ready to use.", d3.title())),
                    Err(e) => message(&from3, "Couldn't erase the drive", &e),
                }
                reload();
            },
        );
    });
    dlg.present(Some(from));
}

// ── Make a bootable USB drive ────────────────────────────────────────────────

fn bootable_dialog(from: &impl IsA<gtk4::Widget>, preselect: Option<String>) {
    let dialog = adw::Dialog::new();
    dialog.set_title("Make a bootable USB drive");
    dialog.set_content_width(560);
    dialog.set_content_height(620);

    let body = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
    body.set_margin_top(12);
    body.set_margin_bottom(24);
    body.set_margin_start(18);
    body.set_margin_end(18);
    body.append(&dim("Writes a disk image (an .iso or .img file) to a USB drive, so a computer can start from it. Everything on the drive is erased."));

    let image: Rc<RefCell<Option<PathBuf>>> = Default::default();
    let drives: Rc<RefCell<Vec<Drive>>> = Default::default();

    let group = adw::PreferencesGroup::new();
    let image_row = adw::ActionRow::new();
    image_row.set_title("Disk image");
    image_row.set_subtitle("Nothing chosen yet");
    let choose = gtk4::Button::with_label("Choose…");
    choose.set_valign(gtk4::Align::Center);
    image_row.add_suffix(&choose);
    let drive_row = adw::ComboRow::new();
    drive_row.set_title("Write to");
    drive_row.set_subtitle("Looking for drives…");
    let verify = adw::SwitchRow::new();
    verify.set_title("Check the drive afterwards");
    verify.set_subtitle("Reads everything back and compares it. Takes about as long again, but catches a bad drive");
    verify.set_active(true);
    let confirm = adw::EntryRow::new();
    confirm.set_title(&format!("Type {CONFIRM_WORD} to confirm"));
    group.add(&image_row);
    group.add(&drive_row);
    group.add(&verify);
    group.add(&confirm);
    body.append(&group);

    let note = dim("");
    body.append(&note);
    let bar = gtk4::ProgressBar::new();
    bar.set_visible(false);
    body.append(&bar);
    let status = dim("");
    body.append(&status);
    let go = gtk4::Button::with_label("Write the image");
    go.add_css_class("destructive-action");
    go.add_css_class("pill");
    go.set_halign(gtk4::Align::Center);
    go.set_sensitive(false);
    body.append(&go);
    let cancel = gtk4::Button::with_label("Stop");
    cancel.set_halign(gtk4::Align::Center);
    cancel.set_visible(false);
    body.append(&cancel);

    let clamp = adw::Clamp::new();
    clamp.set_maximum_size(520);
    clamp.set_child(Some(&body));
    let scroll = gtk4::ScrolledWindow::builder().hscrollbar_policy(gtk4::PolicyType::Never).vexpand(true).build();
    scroll.set_child(Some(&clamp));
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&scroll));
    dialog.set_child(Some(&view));

    // Only drives plugged in from outside that the system doesn't use may be written to (`can_modify`).
    let validate = {
        let (go, image, drives, drive_row, confirm, note) = (go.clone(), image.clone(), drives.clone(), drive_row.clone(), confirm.clone(), note.clone());
        Rc::new(move || {
            let drives = drives.borrow();
            let chosen = drives.get(drive_row.selected() as usize);
            let mut problem: Option<String> = None;
            if let (Some(img), Some(d)) = (image.borrow().as_ref(), chosen) {
                problem = dr::check_image_file(img).and_then(|sz| dr::check_image_fits(img, sz, d)).err();
                if problem.is_none() && !dr::looks_like_iso(img) {
                    note.set_text("This file doesn't look like an ISO. It will be written exactly as it is, which is right for a plain disk image (.img).");
                } else if let Some(p) = &problem {
                    note.set_text(p);
                } else {
                    note.set_text("");
                }
            }
            let ready = image.borrow().is_some() && chosen.is_some() && problem.is_none() && confirm.text().trim() == CONFIRM_WORD;
            go.set_sensitive(ready);
        })
    };

    // fill the drive list
    {
        let (drives, drive_row, validate, preselect) = (drives.clone(), drive_row.clone(), validate.clone(), preselect.clone());
        in_background(
            move || dr::list(),
            move |all| {
                let list = all.into_iter().filter(|d| d.can_modify()).collect::<Vec<_>>();
                let titles: Vec<String> = list.iter().map(|d| format!("{} ({})", d.title(), dr::human(d.size))).collect();
                let refs: Vec<&str> = titles.iter().map(|s| s.as_str()).collect();
                drive_row.set_model(Some(&gtk4::StringList::new(&refs)));
                if let Some(p) = &preselect {
                    if let Some(i) = list.iter().position(|d| &d.path == p) {
                        drive_row.set_selected(i as u32);
                    }
                }
                drive_row.set_subtitle(if list.is_empty() { "No USB drive is plugged in" } else { "Everything on it will be erased" });
                *drives.borrow_mut() = list;
                validate();
            },
        );
    }
    {
        let v = validate.clone();
        drive_row.connect_selected_notify(move |_| v());
        let v = validate.clone();
        confirm.connect_changed(move |_| v());
    }
    {
        let (image, image_row, validate, dialog) = (image.clone(), image_row.clone(), validate.clone(), dialog.clone());
        choose.connect_clicked(move |b| {
            let fd = gtk4::FileDialog::new();
            fd.set_title("Choose a disk image");
            let filter = gtk4::FileFilter::new();
            filter.set_name(Some("Disk images (.iso, .img)"));
            filter.add_pattern("*.iso");
            filter.add_pattern("*.img");
            filter.add_pattern("*.ISO");
            filter.add_pattern("*.IMG");
            let all = gtk4::FileFilter::new();
            all.set_name(Some("All files"));
            all.add_pattern("*");
            let store = gtk4::gio::ListStore::new::<gtk4::FileFilter>();
            store.append(&filter);
            store.append(&all);
            fd.set_filters(Some(&store));
            let (image, image_row, validate, _dialog) = (image.clone(), image_row.clone(), validate.clone(), dialog.clone());
            fd.open(window_of(b).as_ref(), gtk4::gio::Cancellable::NONE, move |res| {
                let Ok(file) = res else { return };
                let Some(path) = file.path() else { return };
                let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let size = dr::check_image_file(&path).map(dr::human).unwrap_or_else(|e| e);
                image_row.set_subtitle(&format!("{name} · {size}"));
                *image.borrow_mut() = Some(path);
                validate();
            });
        });
    }

    // go
    let running = Rc::new(RefCell::new(None::<Arc<Progress>>));
    {
        let (cancel, running) = (cancel.clone(), running.clone());
        cancel.connect_clicked(move |b| {
            if let Some(p) = running.borrow().as_ref() {
                p.cancel.store(true, Ordering::SeqCst);
            }
            b.set_sensitive(false);
        });
    }
    {
        let (image, drives, drive_row, verify, confirm, bar, status, cancel, running, dialog, group) = (
            image.clone(),
            drives.clone(),
            drive_row.clone(),
            verify.clone(),
            confirm.clone(),
            bar.clone(),
            status.clone(),
            cancel.clone(),
            running.clone(),
            dialog.clone(),
            group.clone(),
        );
        go.connect_clicked(move |b| {
            let Some(img) = image.borrow().clone() else { return };
            let Some(drive) = drives.borrow().get(drive_row.selected() as usize).cloned() else { return };
            let identity = drive.identity();
            let verify_after = verify.is_active();
            // lock the form, show progress
            b.set_visible(false);
            group.set_sensitive(false);
            confirm.set_sensitive(false);
            bar.set_visible(true);
            bar.set_fraction(0.0);
            cancel.set_visible(true);
            cancel.set_sensitive(true);
            dialog.set_can_close(false);
            status.set_text("Waiting for permission…");
            let progress = Progress::new();
            *running.borrow_mut() = Some(progress.clone());
            // follow the job
            {
                let (progress, bar, status) = (progress.clone(), bar.clone(), status.clone());
                glib::timeout_add_local(std::time::Duration::from_millis(250), move || {
                    let (phase, done, total) = (progress.phase.load(Ordering::SeqCst), progress.done.load(Ordering::SeqCst), progress.total.load(Ordering::SeqCst));
                    match phase {
                        1 | 2 if total > 0 => {
                            bar.set_fraction(done as f64 / total as f64);
                            status.set_text(&format!("{} {} of {}", if phase == 1 { "Writing" } else { "Checking" }, dr::human(done), dr::human(total)));
                        }
                        0 => {}
                        3 => return glib::ControlFlow::Break,
                        _ => {}
                    }
                    glib::ControlFlow::Continue
                });
            }
            let (bar2, status2, cancel2, dialog2, group2, confirm2, running2, b2, drive2) =
                (bar.clone(), status.clone(), cancel.clone(), dialog.clone(), group.clone(), confirm.clone(), running.clone(), b.clone(), drive.clone());
            let progress2 = progress.clone();
            in_background(
                move || dr::write_image(Udisks::new(), &identity, &img, verify_after, &progress),
                move |res| {
                    *running2.borrow_mut() = None;
                    progress2.phase.store(3, Ordering::SeqCst);
                    cancel2.set_visible(false);
                    dialog2.set_can_close(true);
                    match res {
                        Ok(()) => {
                            bar2.set_fraction(1.0);
                            status2.set_text(&format!("Done. A computer can now start from {}.", drive2.title()));
                        }
                        Err(e) => {
                            bar2.set_visible(false);
                            status2.set_text(&e);
                            group2.set_sensitive(true);
                            confirm2.set_sensitive(true);
                            confirm2.set_text("");
                            b2.set_visible(true);
                        }
                    }
                },
            );
        });
    }
    dialog.present(Some(from));
}
