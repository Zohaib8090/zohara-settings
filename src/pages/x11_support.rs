//! Display > "X11 support": say plainly whether this Plasma has an X11 session, and add one (or another desktop that has one) on
//! request. Plasma 6.8 and newer have no X11 session, so there the answer is Cinnamon or Xfce. The work is in `backend::x11_support`.

use crate::backend::worker::in_background;
use crate::backend::x11_support::{self as x, Desktop, Status};
use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use std::rc::Rc;

fn describe(s: &Status) -> (String, String) {
    let ready = s.ready();
    let have = if ready.is_empty() { String::new() } else { format!(" Ready on the sign-in screen: {}.", ready.join(", ")) };
    match s.plasma {
        Some(p) if x::plasma_has_x11(p) => (
            "X11 support".into(),
            format!("This Plasma ({}.{}) still has its own X11 session; Plasma 6.8 and newer will not.{have}", p.0, p.1),
        ),
        Some(p) => (
            "X11 support".into(),
            format!("Plasma {}.{} has no X11 session. Programs that need X11 still run. For a full X11 desktop, add Cinnamon or Xfce.{have}", p.0, p.1),
        ),
        None => ("X11 support".into(), format!("Could not read the Plasma version.{have}")),
    }
}

pub fn build() -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("X11 support");
    g.set_description(Some("Some older programs and remote-desktop tools need a full X11 desktop, not just X11 programs."));

    let row = adw::ActionRow::new();
    row.set_use_markup(false);
    row.set_subtitle_lines(0);
    row.set_activatable(false);
    let plasma_btn = gtk4::Button::with_label("Add Plasma's X11 session");
    let other_btn = gtk4::Button::with_label("Add an X11 desktop…");
    for b in [&plasma_btn, &other_btn] {
        b.set_valign(gtk4::Align::Center);
    }
    other_btn.add_css_class("suggested-action");
    row.add_suffix(&plasma_btn);
    row.add_suffix(&other_btn);
    g.add(&row);

    let msg = adw::ActionRow::new();
    msg.set_use_markup(false);
    msg.set_subtitle_lines(0);
    msg.set_activatable(false);
    msg.set_visible(false);
    g.add(&msg);

    let refresh: Rc<dyn Fn()> = {
        let (row, plasma_btn, other_btn) = (row.clone(), plasma_btn.clone(), other_btn.clone());
        Rc::new(move || {
            let (row, plasma_btn, other_btn) = (row.clone(), plasma_btn.clone(), other_btn.clone());
            in_background(x::status, move |s| {
                let (title, body) = describe(&s);
                row.set_title(&title);
                row.set_subtitle(&body);
                // Plasma's own session is the first choice while it exists and is not there yet
                plasma_btn.set_visible(s.plasma_x11_possible() && !s.has(Desktop::PlasmaX11));
                other_btn.set_visible(!(s.has(Desktop::Cinnamon) && s.has(Desktop::Xfce)));
            });
        })
    };
    row.set_title("X11 support");
    row.set_subtitle("Checking…");
    refresh();

    // Runs the install and shows the result.
    let run: Rc<dyn Fn(Desktop)> = {
        let (msg, plasma_btn, other_btn, refresh) = (msg.clone(), plasma_btn.clone(), other_btn.clone(), refresh.clone());
        Rc::new(move |d: Desktop| {
            msg.set_visible(true);
            msg.set_title("Working…");
            msg.set_subtitle(&format!("Adding {}. This downloads a few hundred MB and can take several minutes.", d.name()));
            plasma_btn.set_sensitive(false);
            other_btn.set_sensitive(false);
            let (msg, plasma_btn, other_btn, refresh) = (msg.clone(), plasma_btn.clone(), other_btn.clone(), refresh.clone());
            in_background(move || x::install(d), move |err| {
                match err {
                    None => {
                        msg.set_title(&format!("{} is added", d.name()));
                        msg.set_subtitle("Sign out, then choose it on the sign-in screen (the Desktop Session menu, bottom left). Plasma stays as it is.");
                    }
                    Some(e) => {
                        msg.set_title("Nothing was added");
                        msg.set_subtitle(&e);
                    }
                }
                plasma_btn.set_sensitive(true);
                other_btn.set_sensitive(true);
                refresh();
            });
        })
    };

    {
        let run = run.clone();
        plasma_btn.connect_clicked(move |b| {
            let d = adw::AlertDialog::new(
                Some("Add Plasma's X11 session?"),
                Some("This adds a Plasma (X11) entry to the sign-in screen. Use it when a program needs a full X11 desktop. Plasma 6.8 and newer will not have this session, so it only helps for now."),
            );
            d.set_body_use_markup(false);
            d.add_responses(&[("cancel", "Cancel"), ("go", "Add it")]);
            d.set_response_appearance("go", adw::ResponseAppearance::Suggested);
            d.set_default_response(Some("cancel"));
            d.set_close_response("cancel");
            let run = run.clone();
            d.connect_response(None, move |_, r| {
                if r == "go" {
                    run(Desktop::PlasmaX11);
                }
            });
            d.present(Some(b));
        });
    }

    {
        let run = run.clone();
        other_btn.connect_clicked(move |b| {
            let plasma_native = x::status().plasma_x11_possible();
            let intro = if plasma_native {
                "Plasma's own X11 session only exists up to Plasma 6.7, so for the long run another desktop is the answer."
            } else {
                "Plasma 6.8 and newer do not support X11 natively any more: KDE removed the X11 session. Programs that need X11 still run on Plasma, but a full X11 desktop needs a different desktop."
            };
            let d = adw::AlertDialog::new(
                Some("Add an X11 desktop?"),
                Some(&format!(
                    "{intro}\n\nZohara can add one that still has an X11 session:\n• Cinnamon looks like Windows.\n• Xfce is lighter and plainer.\n\nIt downloads a few hundred MB and asks for your password. Plasma stays your normal desktop; you pick the other one on the sign-in screen. Zohara's own settings pages and desktop effects only work in Plasma."
                )),
            );
            d.set_body_use_markup(false);
            d.add_responses(&[("cancel", "Cancel"), ("xfce", "Add Xfce"), ("cinnamon", "Add Cinnamon")]);
            d.set_response_appearance("cinnamon", adw::ResponseAppearance::Suggested);
            d.set_default_response(Some("cancel"));
            d.set_close_response("cancel");
            let run = run.clone();
            d.connect_response(None, move |_, r| match r {
                "cinnamon" => run(Desktop::Cinnamon),
                "xfce" => run(Desktop::Xfce),
                _ => {}
            });
            d.present(Some(b));
        });
    }
    g
}
