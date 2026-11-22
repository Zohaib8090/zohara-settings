//! Zohara Update > Download routes. Extra servers for the approved day, so a network that cannot reach
//! archive.archlinux.org can still update. pacman tries the mirrorlist's servers in order and skips one that keeps failing.
//! The root part is `zohara-settings --apply-routes` (see `manifest::Routes`).

use super::manifest::{self, Routes, MAX_ROUTES};
use crate::backend::worker::in_background;
use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use std::cell::RefCell;
use std::process::Command;
use std::rc::Rc;

/// The address that proves a route works: the first package database of the pinned day (or of the live mirror).
pub fn probe_url(base: &str, dated: bool, date: &str) -> String {
    let base = base.trim_end_matches('/');
    if dated { format!("{base}/{date}/core/os/x86_64/core.db") } else { format!("{base}/core/os/x86_64/core.db") }
}

/// What a person typed: a clean https address, or the reason it is not one.
pub fn check_typed(url: &str) -> Result<String, String> {
    let u = url.trim().trim_end_matches('/').to_string();
    if !u.starts_with("https://") {
        return Err("The address must start with https://".into());
    }
    let r = manifest::parse_routes(&format!("dated {u}\n"));
    r.dated.into_iter().next().ok_or_else(|| "That address has characters a package server address cannot have.".to_string())
}

fn reachable(url: &str) -> bool {
    Command::new("curl").args(["-sI", "-m", "20", "-o", "/dev/null", "-f", url]).status().map(|s| s.success()).unwrap_or(false)
}

fn save(routes: &Routes, done: impl FnOnce(Result<(), String>) + 'static) {
    let text = manifest::routes_text(routes);
    let script = format!("mkdir -p /etc/zohara && cat > {} && /usr/bin/zohara-settings --apply-routes", manifest::ROUTES_FILE);
    in_background(move || crate::pages::accounts::pkexec_sh(&script, &[], Some(text)), done);
}

pub fn open(parent: &gtk4::Widget) {
    let win = adw::Window::builder().default_width(560).default_height(620).title("Download routes").build();
    if let Some(p) = parent.root().and_downcast::<gtk4::Window>() {
        win.set_transient_for(Some(&p));
        win.set_modal(true);
    }
    let page = adw::PreferencesPage::new();
    let list = adw::PreferencesGroup::new();
    list.set_title("Routes");
    list.set_description(Some("System updates are downloaded from these servers, in this order. If one cannot be reached, the next is tried"));
    let add = adw::PreferencesGroup::new();
    add.set_title("Add a route");
    let kind = adw::ComboRow::new();
    kind.set_title("Kind");
    kind.set_model(Some(&gtk4::StringList::new(&["Snapshot server (same approved day)", "Live Arch mirror (newest packages, not tested by Zohara)"])));
    let entry = adw::EntryRow::new();
    entry.set_title("Address, for example https://mirror.example.org/repos");
    entry.set_show_apply_button(true);
    add.add(&kind);
    add.add(&entry);
    let status = gtk4::Label::new(None);
    status.set_wrap(true);
    status.set_halign(gtk4::Align::Start);
    status.add_css_class("dim-label");
    let note = adw::PreferencesGroup::new();
    note.add(&status);
    page.add(&list);
    page.add(&add);
    page.add(&note);
    let header = adw::HeaderBar::new();
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&page));
    win.set_content(Some(&view));

    let routes = Rc::new(RefCell::new(manifest::read_routes()));
    let rows: Rc<RefCell<Vec<adw::ActionRow>>> = Default::default();
    let rebuild: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Default::default();

    let build = {
        let (list, routes, rows, status, rebuild, win) = (list.clone(), routes.clone(), rows.clone(), status.clone(), rebuild.clone(), win.clone());
        Rc::new(move || {
            for r in rows.borrow_mut().drain(..) {
                list.remove(&r);
            }
            let date = manifest::current_pinned_date().unwrap_or_else(|| "2026/01/01".into());
            let mut new_rows = Vec::new();
            let first = adw::ActionRow::new();
            first.set_title("Arch Linux Archive");
            first.set_subtitle("archive.archlinux.org: the approved day. Always first, built in");
            list.add(&first);
            new_rows.push(first);
            let snapshot = routes.borrow().clone();
            for (dated, urls) in [(true, snapshot.dated.clone()), (false, snapshot.live.clone())] {
                for u in urls {
                    let row = adw::ActionRow::new();
                    row.set_title(&u);
                    row.set_subtitle(if dated { "Snapshot server" } else { "Live Arch mirror: only tried when every other route failed" });
                    let test = gtk4::Button::with_label("Test");
                    test.set_valign(gtk4::Align::Center);
                    {
                        let (u, date, status) = (u.clone(), date.clone(), status.clone());
                        test.connect_clicked(move |b| {
                            b.set_sensitive(false);
                            status.set_text("Testing…");
                            let (url, status, b2) = (probe_url(&u, dated, &date), status.clone(), b.clone());
                            in_background(move || reachable(&url), move |ok| {
                                b2.set_sensitive(true);
                                status.set_text(if ok { "Reachable from this connection." } else { "Could not be reached from this connection, or it does not carry the approved day." });
                            });
                        });
                    }
                    let remove = gtk4::Button::with_label("Remove");
                    remove.set_valign(gtk4::Align::Center);
                    {
                        let (routes, status, rebuild, win, u) = (routes.clone(), status.clone(), rebuild.clone(), win.clone(), u.clone());
                        remove.connect_clicked(move |b| {
                            b.set_sensitive(false);
                            let mut next = routes.borrow().clone();
                            next.dated.retain(|x| *x != u);
                            next.live.retain(|x| *x != u);
                            let (routes, status, rebuild, win, next2) = (routes.clone(), status.clone(), rebuild.clone(), win.clone(), next.clone());
                            save(&next, move |res| {
                                match res {
                                    Ok(()) => {
                                        *routes.borrow_mut() = next2;
                                        status.set_text("Removed.");
                                    }
                                    Err(e) => status.set_text(&format!("Not changed: {e}")),
                                }
                                let _ = &win;
                                if let Some(r) = rebuild.borrow().as_ref() {
                                    r();
                                }
                            });
                        });
                    }
                    row.add_suffix(&test);
                    row.add_suffix(&remove);
                    list.add(&row);
                    new_rows.push(row);
                }
            }
            *rows.borrow_mut() = new_rows;
        })
    };
    *rebuild.borrow_mut() = Some(build.clone() as Rc<dyn Fn()>);
    build();

    {
        let (routes, status, kind, rebuild, entry2) = (routes.clone(), status.clone(), kind.clone(), rebuild.clone(), entry.clone());
        entry.connect_apply(move |e| {
            let url = match check_typed(&e.text()) {
                Ok(u) => u,
                Err(msg) => {
                    status.set_text(&msg);
                    return;
                }
            };
            let live = kind.selected() == 1;
            let mut next = routes.borrow().clone();
            let list = if live { &mut next.live } else { &mut next.dated };
            if list.contains(&url) {
                status.set_text("That route is already in the list.");
                return;
            }
            if list.len() >= MAX_ROUTES {
                status.set_text(&format!("At most {MAX_ROUTES} routes of each kind."));
                return;
            }
            list.push(url);
            status.set_text("Saving (asks for your password)…");
            let (routes, status, rebuild, entry, next2) = (routes.clone(), status.clone(), rebuild.clone(), entry2.clone(), next.clone());
            save(&next, move |res| {
                match res {
                    Ok(()) => {
                        *routes.borrow_mut() = next2;
                        entry.set_text("");
                        status.set_text("Added. Use Test to check it works from this connection.");
                    }
                    Err(e) => status.set_text(&format!("Not changed: {e}")),
                }
                if let Some(r) = rebuild.borrow().as_ref() {
                    r();
                }
            });
        });
    }
    win.present();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_test_address_is_the_first_database_of_the_day() {
        assert_eq!(probe_url("https://m.example/repos/", true, "2026/10/13"), "https://m.example/repos/2026/10/13/core/os/x86_64/core.db");
        assert_eq!(probe_url("https://geo.example/archlinux", false, "2026/10/13"), "https://geo.example/archlinux/core/os/x86_64/core.db");
    }

    #[test]
    fn typed_addresses_are_checked() {
        assert_eq!(check_typed(" https://m.example/repos/ ").unwrap(), "https://m.example/repos");
        assert!(check_typed("http://m.example/repos").unwrap_err().contains("https"));
        assert!(check_typed("https://m.example/re pos").is_err());
        assert!(check_typed("https://m.example/../x").is_err());
        assert!(check_typed("").is_err());
    }
}
