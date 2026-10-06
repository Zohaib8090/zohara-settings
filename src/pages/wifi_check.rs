//! The "Wi-Fi adapter" card on the Network page. It appears only when something is wrong with Wi-Fi, says what, and
//! offers the one button that can fix that case. The judging is `backend::wifi_diag::diagnose` (unit-tested); this is
//! only the card.

use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use std::process::Command;
use std::rc::Rc;
use std::time::Duration;

use crate::backend::wifi_diag::{self, Facts, Verdict};

/// A box that fills itself with the card (or stays empty when Wi-Fi is fine). Safe to append straight away.
pub fn group() -> gtk4::Widget {
    let holder = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    refresh(&holder);
    holder.upcast()
}

/// Reads the system off the UI thread, then redraws the card.
fn refresh(holder: &gtk4::Box) {
    let (tx, rx) = std::sync::mpsc::channel::<Facts>();
    std::thread::spawn(move || {
        let _ = tx.send(wifi_diag::gather());
    });
    let holder = holder.clone();
    glib::timeout_add_local(Duration::from_millis(120), move || match rx.try_recv() {
        Ok(facts) => {
            render(&holder, facts);
            glib::ControlFlow::Break
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        Err(_) => glib::ControlFlow::Break,
    });
}

fn run_then_refresh(holder: &gtk4::Box, label: &gtk4::Label, mut cmd: Command, working: &str) {
    label.set_text(working);
    let (tx, rx) = std::sync::mpsc::channel::<bool>();
    std::thread::spawn(move || {
        let ok = cmd.status().map(|s| s.success()).unwrap_or(false);
        // Give the driver and NetworkManager a moment to bring the interface back before looking again.
        std::thread::sleep(Duration::from_secs(4));
        let _ = tx.send(ok);
    });
    let (holder, label) = (holder.clone(), label.clone());
    glib::timeout_add_local(Duration::from_millis(200), move || match rx.try_recv() {
        Ok(ok) => {
            if !ok {
                label.set_text("That didn't work (the password prompt may have been cancelled).");
            }
            refresh(&holder);
            glib::ControlFlow::Break
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        Err(_) => glib::ControlFlow::Break,
    });
}

fn render(holder: &gtk4::Box, facts: Facts) {
    while let Some(c) = holder.first_child() {
        holder.remove(&c);
    }
    let verdict = wifi_diag::diagnose(&facts);
    if verdict.is_ready() {
        return; // nothing to say when it works
    }

    let g = adw::PreferencesGroup::new();
    g.set_title("Wi-Fi adapter");
    let row = adw::ActionRow::new();
    row.set_use_markup(false);
    row.set_title(verdict.title());
    row.set_subtitle(&verdict.explanation());
    row.set_subtitle_lines(0);
    row.add_prefix(&gtk4::Image::from_icon_name("network-wireless-disabled-symbolic"));
    g.add(&row);

    let status = gtk4::Label::new(None);
    status.set_xalign(0.0);
    status.add_css_class("dim-label");

    let buttons = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    buttons.set_margin_top(8);
    let add = |label: &str, suggested: bool| {
        let b = gtk4::Button::with_label(label);
        if suggested {
            b.add_css_class("suggested-action");
        }
        buttons.append(&b);
        b
    };

    match &verdict {
        Verdict::SoftBlocked => {
            let b = add("Turn Wi-Fi on", true);
            let (h, s) = (holder.clone(), status.clone());
            b.connect_clicked(move |_| {
                let mut c = Command::new("nmcli");
                c.args(["radio", "wifi", "on"]);
                run_then_refresh(&h, &s, c, "Turning Wi-Fi on…");
            });
        }
        Verdict::NoInterface { .. } => {
            if let Some(module) = wifi_diag::driver_to_reload(&facts) {
                let b = add("Reload the Wi-Fi driver", true);
                let (h, s) = (holder.clone(), status.clone());
                b.connect_clicked(move |_| {
                    let mut c = Command::new("pkexec");
                    c.args(["sh", "-c", "modprobe -r \"$1\"; modprobe \"$1\"; systemctl restart NetworkManager", "sh", &module]);
                    run_then_refresh(&h, &s, c, "Reloading the Wi-Fi driver…");
                });
            }
        }
        Verdict::NotManaged | Verdict::NetworkManagerDown => {
            let b = add("Restart the network service", true);
            let (h, s) = (holder.clone(), status.clone());
            b.connect_clicked(move |_| {
                let mut c = Command::new("pkexec");
                c.args(["systemctl", "restart", "NetworkManager"]);
                run_then_refresh(&h, &s, c, "Restarting the network service…");
            });
        }
        _ => {}
    }

    let again = add("Check again", false);
    {
        let h = holder.clone();
        again.connect_clicked(move |_| refresh(&h));
    }
    let copy = add("Copy report", false);
    {
        let text = wifi_diag::report(&facts, &verdict);
        let s = status.clone();
        copy.connect_clicked(move |b| {
            b.display().clipboard().set_text(&text);
            s.set_text("Copied. Paste it into a message to the Zohara team.");
        });
    }

    let inner = Rc::new(gtk4::Box::new(gtk4::Orientation::Vertical, 0));
    inner.append(&g);
    inner.append(&buttons);
    inner.append(&status);
    inner.set_margin_bottom(8);
    holder.append(&*inner);
}
