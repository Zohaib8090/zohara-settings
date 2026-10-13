//! Privacy & security > App permissions: one permission at a time, and which apps can use it.
//!
//! Flatpak apps are fenced in, so each of them shows a switch for the chosen thing (the same switches as in the app's own
//! details screen). Apps installed some other way are not fenced in: that is said in the description, and they can be
//! restricted from their own details screen.

use crate::backend::app_details as ad;
use crate::backend::gpu_pref as pref;
use crate::backend::worker::in_background;
use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::process::Command;

#[derive(Clone, Copy, PartialEq)]
enum What {
    Internet,
    Files,
    Camera,
    Microphone,
    Sound,
    Devices,
    Bluetooth,
    Graphics,
}

/// (what, the choice in the list, what the switch means)
const CHOICES: [(What, &str, &str); 8] = [
    (What::Internet, "Internet", "Can use the internet and your local network"),
    (What::Files, "All your files", "Can see your whole home folder"),
    (What::Camera, "Camera", "Allowed to use the camera"),
    (What::Microphone, "Microphone", "Allowed to use the microphone"),
    (What::Sound, "Sound", "Can play sound (and record when it has the microphone)"),
    (What::Devices, "USB devices and controllers", "Can use every device: USB, game controllers, cameras"),
    (What::Bluetooth, "Bluetooth", "Can connect to Bluetooth devices"),
    (What::Graphics, "Dedicated graphics card", "Always opens on the dedicated graphics card"),
];

#[derive(Clone)]
struct AppState {
    id: String,
    name: String,
    ctx: ad::Context,
    camera: Option<bool>,
    microphone: Option<bool>,
    gpu: bool,
}

fn load() -> Vec<AppState> {
    let Ok(o) = Command::new("flatpak").args(["list", "--app", "--columns=application,name"]).output() else { return Vec::new() };
    let mut out: Vec<AppState> = String::from_utf8_lossy(&o.stdout)
        .lines()
        .filter_map(|l| {
            let mut p = l.split('\t');
            let id = p.next()?.trim().to_string();
            let name = p.next().map(str::trim).filter(|n| !n.is_empty()).unwrap_or(&id).to_string();
            let ctx = ad::effective_context(&id)?;
            Some(AppState {
                camera: ad::portal_state(&id, ad::Portal::Camera),
                microphone: ad::portal_state(&id, ad::Portal::Microphone),
                gpu: pref::flatpak_is_set(&id),
                id,
                name,
                ctx,
            })
        })
        .collect();
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

fn is_on(a: &AppState, what: What) -> bool {
    match what {
        What::Internet => ad::Switch::Network.allowed_in(&a.ctx),
        What::Files => ad::files_level(&a.ctx) != ad::Files::Chosen,
        What::Camera => a.camera == Some(true),
        What::Microphone => a.microphone == Some(true),
        What::Sound => ad::Switch::Sound.allowed_in(&a.ctx),
        What::Devices => ad::Switch::AllDevices.allowed_in(&a.ctx),
        What::Bluetooth => ad::Switch::Bluetooth.allowed_in(&a.ctx),
        What::Graphics => a.gpu,
    }
}

fn apply(id: &str, what: What, on: bool) -> Result<(), String> {
    match what {
        What::Internet => ad::set_switch(id, ad::Switch::Network, on),
        What::Files => ad::set_files(id, if on { ad::Files::Home } else { ad::Files::Chosen }),
        What::Camera => ad::set_portal(id, ad::Portal::Camera, on),
        What::Microphone => ad::set_portal(id, ad::Portal::Microphone, on),
        What::Sound => ad::set_switch(id, ad::Switch::Sound, on),
        What::Devices => ad::set_switch(id, ad::Switch::AllDevices, on),
        What::Bluetooth => ad::set_switch(id, ad::Switch::Bluetooth, on),
        What::Graphics => pref::flatpak_set(id, on),
    }
}

fn message(from: &impl IsA<gtk4::Widget>, title: &str, body: &str) {
    let d = adw::AlertDialog::new(Some(title), Some(body));
    d.add_response("ok", "OK");
    d.present(Some(from));
}

pub fn group() -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title("App permissions");
    g.set_description(Some(
        "Pick one thing and see which apps can use it. Apps from Flatpak are fenced in, so you can switch it off for each one. Apps installed other ways (system packages, the AUR, AppImages) are not fenced in; open one in Apps to restrict it. Changes apply the next time the app is opened.",
    ));

    let what_row = adw::ComboRow::new();
    what_row.set_title("Show apps that can use");
    let choices: Vec<&str> = CHOICES.iter().filter(|(w, _, _)| *w != What::Graphics || pref::has_two_gpus()).map(|(_, t, _)| *t).collect();
    what_row.set_model(Some(&gtk4::StringList::new(&choices)));
    g.add(&what_row);

    let rows: Rc<RefCell<Vec<gtk4::Widget>>> = Rc::default();
    let apps: Rc<RefCell<Option<Vec<AppState>>>> = Rc::default();
    let loading = adw::ActionRow::new();
    loading.set_title("Looking at your apps…");
    g.add(&loading);
    rows.borrow_mut().push(loading.clone().upcast());

    let render: Rc<dyn Fn()> = {
        let (g, what_row, rows, apps) = (g.clone(), what_row.clone(), rows.clone(), apps.clone());
        Rc::new(move || {
            for w in rows.borrow_mut().drain(..) {
                g.remove(&w);
            }
            let Some(list) = apps.borrow().clone() else { return };
            let two_gpus = pref::has_two_gpus();
            let visible: Vec<(What, &str, &str)> = CHOICES.iter().filter(|(w, _, _)| *w != What::Graphics || two_gpus).copied().collect();
            let Some(&(what, _, meaning)) = visible.get(what_row.selected() as usize) else { return };
            if list.is_empty() {
                let r = adw::ActionRow::new();
                r.set_title("No Flatpak apps are installed");
                r.set_subtitle("Apps from Flathub show up here once you install them from the Store");
                g.add(&r);
                rows.borrow_mut().push(r.upcast());
                return;
            }
            for a in &list {
                let row = adw::SwitchRow::new();
                row.set_title(&glib::markup_escape_text(&a.name));
                let unknown = matches!(what, What::Camera | What::Microphone) && ((what == What::Camera && a.camera.is_none()) || (what == What::Microphone && a.microphone.is_none()));
                row.set_subtitle(&glib::markup_escape_text(&if unknown { format!("{meaning}. Asks the first time it needs it") } else { meaning.to_string() }));
                row.set_active(is_on(a, what));
                let id = a.id.clone();
                let loading = Rc::new(Cell::new(false));
                let parent = g.clone();
                row.connect_active_notify(move |r| {
                    if loading.get() {
                        return;
                    }
                    let on = r.is_active();
                    let (id, r2, loading, parent) = (id.clone(), r.clone(), loading.clone(), parent.clone());
                    in_background(move || apply(&id, what, on), move |res| {
                        if let Err(e) = res {
                            loading.set(true);
                            r2.set_active(!on);
                            loading.set(false);
                            message(&parent, "Couldn't change it", &e);
                        }
                    });
                });
                g.add(&row);
                rows.borrow_mut().push(row.upcast());
            }
        })
    };
    {
        let render = render.clone();
        what_row.connect_selected_notify(move |_| render());
    }
    {
        let (apps, render) = (apps.clone(), render.clone());
        in_background(load, move |list| {
            *apps.borrow_mut() = Some(list);
            render();
        });
    }
    g
}
