//! Personalization > Dynamic Lighting: control keyboard, mouse, motherboard and RAM lighting through OpenRGB.
//! When OpenRGB is not installed the window offers to install it (one button, asks for the password), instead of
//! telling the person to go and find a package.

use crate::backend::worker::in_background;
use gtk4::prelude::*;
use libadwaita as adw;
use adw::prelude::*;
use std::process::Command;
use std::rc::Rc;

#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    pub index: usize,
    pub name: String,
    pub kind: String,
    /// The text after `Modes:` (current mode in brackets).
    pub modes: String,
}

/// Effects offered when the device lists them: (label shown, mode name sent to OpenRGB).
const EFFECTS: [(&str, &str); 6] = [
    ("Rainbow", "Rainbow"),
    ("Breathing", "Breathing"),
    ("Spectrum cycle", "Spectrum Cycle"),
    ("Color cycle", "Color Cycle"),
    ("Wave", "Wave"),
    ("Flashing", "Flashing"),
];

/// Parses `openrgb --list-devices`: a header line `0: Name` then indented `Type:` and `Modes:` lines.
pub fn parse_devices(out: &str) -> Vec<Device> {
    let mut v: Vec<Device> = Vec::new();
    for line in out.lines() {
        let t = line.trim();
        if !line.starts_with(' ') && !line.starts_with('\t') {
            if let Some((i, name)) = t.split_once(':') {
                if let Ok(index) = i.trim().parse::<usize>() {
                    v.push(Device { index, name: name.trim().to_string(), kind: String::new(), modes: String::new() });
                }
            }
            continue;
        }
        let Some(d) = v.last_mut() else { continue };
        if let Some(x) = t.strip_prefix("Type:") {
            d.kind = x.trim().to_string();
        } else if let Some(x) = t.strip_prefix("Modes:") {
            d.modes = x.trim().to_string();
        }
    }
    v
}

/// Effects this device really lists.
pub fn effects_for(d: &Device) -> Vec<(&'static str, &'static str)> {
    EFFECTS.iter().copied().filter(|(_, mode)| d.modes.to_lowercase().contains(&mode.to_lowercase())).collect()
}

/// The mode that shows one fixed colour: "Direct" if the device has it, else "Static".
pub fn fixed_mode(d: &Device) -> &'static str {
    if d.modes.contains("Direct") { "Direct" } else { "Static" }
}

fn has_openrgb() -> bool {
    Command::new("sh").args(["-c", "command -v openrgb"]).output().map(|o| o.status.success()).unwrap_or(false)
}

fn list_devices() -> Result<Vec<Device>, String> {
    let o = Command::new("openrgb").args(["--noautoconnect", "--list-devices"]).output().map_err(|e| e.to_string())?;
    if !o.status.success() {
        return Err(String::from_utf8_lossy(&o.stderr).trim().to_string());
    }
    Ok(parse_devices(&String::from_utf8_lossy(&o.stdout)))
}

fn openrgb(args: &[String]) {
    let mut c = Command::new("openrgb");
    c.arg("--noautoconnect").args(args);
    let _ = c.status();
}

fn hex(c: &gtk4::gdk::RGBA) -> String {
    let b = |f: f32| (f.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("{:02X}{:02X}{:02X}", b(c.red()), b(c.green()), b(c.blue()))
}

/// Installs OpenRGB and switches on the hardware access it needs. One password prompt.
fn install_openrgb() -> Result<(), String> {
    let script = "pacman -S --needed --noconfirm openrgb && modprobe i2c-dev; \
                  echo i2c-dev > /etc/modules-load.d/zohara-openrgb.conf; \
                  udevadm control --reload-rules; udevadm trigger; command -v openrgb >/dev/null";
    let o = Command::new("pkexec").args(["sh", "-c", script]).output().map_err(|e| e.to_string())?;
    match o.status.code() {
        Some(0) => Ok(()),
        Some(126) | Some(127) => Err("The password prompt was cancelled".into()),
        _ => {
            let err = String::from_utf8_lossy(&o.stderr);
            let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("The install did not finish");
            Err(last.trim().to_string())
        }
    }
}

pub fn open(parent_row: &adw::ActionRow) {
    let content = super::personalization::dialog_content_box();
    super::personalization::open_settings_window_sized(parent_row, "Dynamic Lighting", &content, 560, 640);
    render(content);
}

fn render(content: gtk4::Box) {
    while let Some(c) = content.first_child() {
        content.remove(&c);
    }
    if !has_openrgb() {
        let page = adw::StatusPage::builder()
            .icon_name("weather-clear-symbolic")
            .title("Lighting control needs OpenRGB")
            .description("OpenRGB is a free tool that controls keyboard, mouse, motherboard and memory lighting. Install it here and your devices will show up.")
            .build();
        let btn = gtk4::Button::with_label("Install OpenRGB");
        btn.add_css_class("suggested-action");
        btn.add_css_class("pill");
        btn.set_halign(gtk4::Align::Center);
        let status = gtk4::Label::new(None);
        status.set_wrap(true);
        status.add_css_class("dim-label");
        let col = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
        col.append(&btn);
        col.append(&status);
        page.set_child(Some(&col));
        content.append(&page);
        let content2 = content.clone();
        btn.connect_clicked(move |b| {
            b.set_sensitive(false);
            b.set_label("Installing…");
            status.set_text("Waiting for the password prompt, then downloading. This can take a minute.");
            let (b, status, content3) = (b.clone(), status.clone(), content2.clone());
            in_background(install_openrgb, move |res| match res {
                Ok(()) => render(content3),
                Err(e) => {
                    b.set_sensitive(true);
                    b.set_label("Try again");
                    status.set_text(&format!("Couldn't install it: {e}"));
                }
            });
        });
        return;
    }

    let spinner = gtk4::Spinner::new();
    spinner.start();
    spinner.set_margin_top(24);
    content.append(&spinner);
    in_background(list_devices, move |res| {
        while let Some(c) = content.first_child() {
            content.remove(&c);
        }
        let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        let title = gtk4::Label::new(Some("Lighting devices"));
        title.add_css_class("title-4");
        title.set_hexpand(true);
        title.set_halign(gtk4::Align::Start);
        let refresh = gtk4::Button::from_icon_name("view-refresh-symbolic");
        refresh.set_tooltip_text(Some("Look for devices again"));
        let content_r = content.clone();
        refresh.connect_clicked(move |_| render(content_r.clone()));
        header.append(&title);
        header.append(&refresh);
        content.append(&header);

        let devices = match res {
            Ok(d) => d,
            Err(e) => {
                let l = gtk4::Label::new(Some(&format!("OpenRGB didn't answer: {e}")));
                l.set_wrap(true);
                l.set_halign(gtk4::Align::Start);
                content.append(&l);
                Vec::new()
            }
        };
        if devices.is_empty() {
            let l = gtk4::Label::new(Some(
                "No lighting devices were found. Plug in a lighting device, or press the reload button. If you have one and it still doesn't show, it may not be supported yet.",
            ));
            l.set_wrap(true);
            l.set_halign(gtk4::Align::Start);
            l.add_css_class("dim-label");
            content.append(&l);
            return;
        }

        // All devices at once.
        let all = adw::PreferencesGroup::new();
        all.set_title("All devices");
        let all_row = adw::ActionRow::new();
        all_row.set_title("Turn every light on or off");
        let on = gtk4::Button::with_label("White");
        let off = gtk4::Button::with_label("Off");
        for b in [&on, &off] {
            b.set_valign(gtk4::Align::Center);
        }
        let ds = Rc::new(devices.clone());
        let (ds1, ds2) = (ds.clone(), ds.clone());
        on.connect_clicked(move |_| set_all(&ds1, "FFFFFF"));
        off.connect_clicked(move |_| set_all(&ds2, "000000"));
        all_row.add_suffix(&on);
        all_row.add_suffix(&off);
        all.add(&all_row);
        content.append(&all);

        let group = adw::PreferencesGroup::new();
        group.set_title("Devices");
        for d in devices {
            group.add(&device_row(&d));
        }
        content.append(&group);
    });
}

fn set_all(ds: &[Device], color: &str) {
    let list: Vec<(usize, &'static str)> = ds.iter().map(|d| (d.index, fixed_mode(d))).collect();
    let color = color.to_string();
    std::thread::spawn(move || {
        for (i, mode) in list {
            openrgb(&["-d".into(), i.to_string(), "-m".into(), mode.into(), "-c".into(), color.clone()]);
        }
    });
}

fn device_row(d: &Device) -> adw::ExpanderRow {
    let row = adw::ExpanderRow::new();
    row.set_title(&glib::markup_escape_text(&d.name));
    if !d.kind.is_empty() {
        row.set_subtitle(&d.kind);
    }
    row.add_prefix(&gtk4::Image::from_icon_name("weather-clear-symbolic"));

    let color_row = adw::ActionRow::new();
    color_row.set_title("Colour");
    color_row.set_subtitle("Pick one colour for the whole device");
    let btn = gtk4::ColorDialogButton::new(Some(gtk4::ColorDialog::new()));
    btn.set_valign(gtk4::Align::Center);
    btn.set_rgba(&gtk4::gdk::RGBA::new(1.0, 1.0, 1.0, 1.0));
    let (idx, mode) = (d.index, fixed_mode(d));
    btn.connect_rgba_notify(move |b| {
        let c = hex(&b.rgba());
        std::thread::spawn(move || openrgb(&["-d".into(), idx.to_string(), "-m".into(), mode.into(), "-c".into(), c]));
    });
    color_row.add_suffix(&btn);
    row.add_row(&color_row);

    let fx = effects_for(d);
    if !fx.is_empty() {
        let e = adw::ComboRow::new();
        e.set_title("Effect");
        e.set_subtitle("Moving effects the device can do by itself");
        let mut labels = vec!["Choose an effect"];
        labels.extend(fx.iter().map(|(l, _)| *l));
        e.set_model(Some(&gtk4::StringList::new(&labels)));
        let fx2 = fx.clone();
        e.connect_selected_notify(move |r| {
            let i = r.selected() as usize;
            if i == 0 {
                return;
            }
            let mode = fx2[i - 1].1.to_string();
            std::thread::spawn(move || openrgb(&["-d".into(), idx.to_string(), "-m".into(), mode]));
        });
        row.add_row(&e);
    }

    let off_row = adw::ActionRow::new();
    off_row.set_title("Turn off");
    let off = gtk4::Button::with_label("Off");
    off.set_valign(gtk4::Align::Center);
    off.connect_clicked(move |_| {
        std::thread::spawn(move || openrgb(&["-d".into(), idx.to_string(), "-m".into(), mode.into(), "-c".into(), "000000".into()]));
    });
    off_row.add_suffix(&off);
    row.add_row(&off_row);
    row
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "0: ASUS ROG STRIX\n  Type:           Motherboard\n  Description:    AURA\n  Modes: [Direct] Static Breathing Spectrum Cycle Rainbow\n  Zones: Mainboard\n\n1: Razer Mouse\n  Type:           Mouse\n  Modes: [Static] Wave\n";

    #[test]
    fn devices_are_read_with_type_and_modes() {
        let v = parse_devices(SAMPLE);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].name, "ASUS ROG STRIX");
        assert_eq!(v[0].kind, "Motherboard");
        assert_eq!(v[1].index, 1);
        assert!(v[1].modes.contains("Wave"));
    }

    #[test]
    fn only_listed_effects_are_offered() {
        let v = parse_devices(SAMPLE);
        let a: Vec<_> = effects_for(&v[0]).iter().map(|e| e.0).collect();
        assert_eq!(a, ["Rainbow", "Breathing", "Spectrum cycle"]);
        let b: Vec<_> = effects_for(&v[1]).iter().map(|e| e.0).collect();
        assert_eq!(b, ["Wave"]);
    }

    #[test]
    fn fixed_colour_uses_direct_when_there_is_one() {
        let v = parse_devices(SAMPLE);
        assert_eq!(fixed_mode(&v[0]), "Direct");
        assert_eq!(fixed_mode(&v[1]), "Static");
    }

    #[test]
    fn colours_become_hex() {
        assert_eq!(hex(&gtk4::gdk::RGBA::new(1.0, 0.0, 0.5, 1.0)), "FF0080");
    }
}
