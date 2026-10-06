//! Per-device mouse and touchpad settings through KWin's InputDevice D-Bus
//! API (Plasma Wayland). KWin applies each change to the libinput device
//! immediately and saves it to kcminputrc itself, so nothing else is needed.

use crate::backend::worker::{block_on, in_background};
use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;

const KWIN: &str = "org.kde.KWin";
const DEVICE_IFACE: &str = "org.kde.KWin.InputDevice";

/// How a device is connected, read from where the kernel put it in /sys.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Bus {
    BuiltIn,
    Usb,
    Bluetooth,
    /// Made by software (remote-control tools, virtual keyboards): never a real mouse.
    Virtual,
    #[default]
    Other,
}

impl Bus {
    pub fn label(self) -> &'static str {
        match self {
            Bus::BuiltIn => "Built in",
            Bus::Usb => "USB",
            Bus::Bluetooth => "Bluetooth",
            Bus::Virtual => "Virtual device",
            Bus::Other => "",
        }
    }
}

/// `link` is where `/sys/class/input/eventN` points, for example
/// `../../devices/pci0000:00/0000:00:14.0/usb1/1-2/1-2:1.0/0003:046D:C52B.0003/input/input12/event5`.
pub fn classify_bus(link: &str) -> Bus {
    let l = link.to_lowercase();
    if l.contains("/devices/virtual/") {
        Bus::Virtual
    } else if l.contains("bluetooth") || l.contains("/uhid/") || l.contains("hci") {
        Bus::Bluetooth
    } else if l.contains("/usb") {
        Bus::Usb
    } else if l.contains("i8042") || l.contains("serio") || l.contains("i2c") || l.contains("/platform/") || l.contains("intel-ish") {
        Bus::BuiltIn
    } else {
        Bus::Other
    }
}

#[derive(Clone, Default)]
pub struct Device {
    sys: String,
    bus: Bus,
    name: String,
    touchpad: bool,
    pointer: bool,
    enabled: Option<bool>,
    left_handed: Option<bool>,
    accel: Option<f64>,
    flat_profile: Option<bool>,
    natural_scroll: Option<bool>,
    scroll_factor: Option<f64>,
    tap_to_click: Option<bool>,
    disable_while_typing: Option<bool>,
    middle_emulation: Option<bool>,
}

async fn read_device(conn: &zbus::Connection, sys: String) -> zbus::Result<Device> {
    let p = zbus::Proxy::new(conn, KWIN, format!("/org/kde/KWin/InputDevice/{sys}"), DEVICE_IFACE).await?;
    let b = |name: &'static str| {
        let p = &p;
        async move { p.get_property::<bool>(name).await.ok() }
    };
    let f = |name: &'static str| {
        let p = &p;
        async move { p.get_property::<f64>(name).await.ok() }
    };
    // Only offer a control when the device reports supporting it.
    let when = |supported: Option<bool>, v: Option<bool>| if supported == Some(true) { v } else { None };

    let bus = std::fs::read_link(format!("/sys/class/input/{sys}")).map(|l| classify_bus(&l.to_string_lossy())).unwrap_or_default();
    Ok(Device {
        bus,
        name: p.get_property::<String>("name").await.unwrap_or_else(|_| sys.clone()),
        touchpad: b("touchpad").await.unwrap_or(false),
        pointer: b("pointer").await.unwrap_or(false),
        enabled: when(b("supportsDisableEvents").await, b("enabled").await),
        left_handed: when(b("supportsLeftHanded").await, b("leftHanded").await),
        accel: if b("supportsPointerAcceleration").await == Some(true) { f("pointerAcceleration").await } else { None },
        flat_profile: when(
            b("supportsPointerAccelerationProfileFlat").await,
            b("pointerAccelerationProfileFlat").await,
        ),
        natural_scroll: when(b("supportsNaturalScroll").await, b("naturalScroll").await),
        scroll_factor: f("scrollFactor").await,
        tap_to_click: match p.get_property::<i32>("tapFingerCount").await {
            Ok(n) if n > 0 => b("tapToClick").await,
            _ => None,
        },
        disable_while_typing: when(b("supportsDisableWhileTyping").await, b("disableWhileTyping").await),
        middle_emulation: when(b("supportsMiddleEmulation").await, b("middleEmulation").await),
        sys,
    })
}

fn load_devices() -> Result<Vec<Device>, String> {
    block_on(async {
        let conn = zbus::Connection::session().await.map_err(|e| e.to_string())?;
        let mgr = zbus::Proxy::new(&conn, KWIN, "/org/kde/KWin/InputDevice", "org.kde.KWin.InputDeviceManager")
            .await
            .map_err(|e| e.to_string())?;
        let names: Vec<String> = mgr.get_property("devicesSysNames").await.map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for sys in names {
            if let Ok(d) = read_device(&conn, sys).await {
                out.push(d);
            }
        }
        Ok(out)
    })
}

#[derive(Clone, Copy)]
enum Val {
    B(bool),
    F(f64),
}

fn set(sys: &str, prop: &'static str, v: Val) {
    let path = format!("/org/kde/KWin/InputDevice/{sys}");
    std::thread::spawn(move || {
        let _ = block_on(async {
            let conn = zbus::Connection::session().await?;
            let p = zbus::Proxy::new(&conn, KWIN, path, DEVICE_IFACE).await?;
            match v {
                Val::B(b) => p.set_property(prop, b).await?,
                Val::F(f) => p.set_property(prop, f).await?,
            }
            Ok::<_, zbus::Error>(())
        });
    });
}

fn switch(group: &adw::PreferencesGroup, dev: &Device, value: Option<bool>, prop: &'static str, title: &str, sub: &str) {
    let Some(v) = value else { return };
    let row = adw::SwitchRow::new();
    row.set_title(title);
    if !sub.is_empty() {
        row.set_subtitle(sub);
    }
    row.set_active(v);
    let sys = dev.sys.clone();
    row.connect_active_notify(move |r| set(&sys, prop, Val::B(r.is_active())));
    group.add(&row);
}

fn slider(
    group: &adw::PreferencesGroup,
    dev: &Device,
    value: Option<f64>,
    prop: &'static str,
    title: &str,
    (min, max, step): (f64, f64, f64),
    marks: &[(f64, &str)],
) {
    let Some(v) = value else { return };
    let scale = gtk4::Scale::with_range(gtk4::Orientation::Horizontal, min, max, step);
    scale.set_value(v.clamp(min, max));
    scale.set_size_request(260, -1);
    scale.set_hexpand(true);
    scale.set_valign(gtk4::Align::Center);
    for (at, label) in marks {
        scale.add_mark(*at, gtk4::PositionType::Bottom, Some(label));
    }
    let sys = dev.sys.clone();
    scale.connect_value_changed(move |s| set(&sys, prop, Val::F(s.value())));
    let row = adw::ActionRow::new();
    row.set_title(title);
    row.set_activatable(false);
    row.add_suffix(&scale);
    group.add(&row);
}

impl Device {
    /// Whether KWin offers anything to change for this device. A USB receiver registers several devices
    /// ("Mouse", "Consumer Control", "System Control", ...); only the real mouse has controls.
    fn has_controls(&self) -> bool {
        self.enabled.is_some()
            || self.left_handed.is_some()
            || self.accel.is_some()
            || self.flat_profile.is_some()
            || self.natural_scroll.is_some()
            || self.tap_to_click.is_some()
            || self.disable_while_typing.is_some()
            || self.middle_emulation.is_some()
    }

    /// Worth a card on the Mouse/Touchpad page.
    fn is_useful(&self) -> bool {
        self.bus != Bus::Virtual && self.has_controls()
    }

    fn why_hidden(&self) -> &'static str {
        if self.bus == Bus::Virtual {
            "Virtual device made by software"
        } else {
            "Nothing to change on this one"
        }
    }
}

/// From `libinput debug-events`: the names of the devices that moved, clicked or scrolled.
///
/// The output has "-event5   DEVICE_ADDED   Logitech USB Receiver   seat0 default group1 cap:pk" lines first, and then
/// " event5   POINTER_MOTION ..." lines for each thing that happens.
pub fn parse_libinput_events(text: &str) -> Vec<String> {
    use std::collections::{BTreeSet, HashMap};
    let mut names: HashMap<String, String> = HashMap::new();
    let mut active: BTreeSet<String> = BTreeSet::new();
    for line in text.lines() {
        let trimmed = line.trim_start_matches(|c: char| c == '-' || c == ' ');
        let mut parts = trimmed.split_whitespace();
        let (Some(ev), Some(kind)) = (parts.next(), parts.next()) else { continue };
        if !ev.starts_with("event") {
            continue;
        }
        if kind == "DEVICE_ADDED" {
            // The name is everything between the event type and " seat<N>".
            let after = trimmed.splitn(2, "DEVICE_ADDED").nth(1).unwrap_or("").trim();
            let end = after.find(" seat").unwrap_or(after.len());
            names.insert(ev.to_string(), after[..end].trim().to_string());
        } else if kind.starts_with("POINTER_") {
            active.insert(ev.to_string());
        }
    }
    active.into_iter().filter_map(|e| names.get(&e).cloned()).collect()
}

fn device_group(dev: &Device) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(&glib::markup_escape_text(&dev.name));
    if !dev.bus.label().is_empty() {
        g.set_description(Some(dev.bus.label()));
    }

    if dev.touchpad {
        switch(&g, dev, dev.enabled, "enabled", "Touchpad", "");
        switch(&g, dev, dev.tap_to_click, "tapToClick", "Tap to click", "Tap the touchpad instead of pressing it");
        switch(&g, dev, dev.disable_while_typing, "disableWhileTyping", "Disable while typing", "");
    }
    slider(&g, dev, dev.accel, "pointerAcceleration", "Pointer speed", (-1.0, 1.0, 0.05), &[(-1.0, "Slow"), (0.0, ""), (1.0, "Fast")]);
    // Flat profile = no acceleration; present it as "acceleration on" to match how people think about it.
    if let Some(flat) = dev.flat_profile {
        let row = adw::SwitchRow::new();
        row.set_title("Pointer acceleration");
        row.set_subtitle("Move the pointer further when you move faster");
        row.set_active(!flat);
        let sys = dev.sys.clone();
        row.connect_active_notify(move |r| set(&sys, "pointerAccelerationProfileFlat", Val::B(!r.is_active())));
        g.add(&row);
    }
    switch(&g, dev, dev.natural_scroll, "naturalScroll", "Natural scrolling", "Content moves in the direction of your fingers");
    slider(&g, dev, dev.scroll_factor, "scrollFactor", "Scrolling speed", (0.1, 3.0, 0.1), &[(1.0, "Default")]);
    switch(&g, dev, dev.left_handed, "leftHanded", "Left-handed mode", "Swap the primary and secondary buttons");
    if !dev.touchpad {
        switch(&g, dev, dev.middle_emulation, "middleEmulation", "Middle-click emulation", "Press left and right together to middle-click");
    }
    g
}

/// The "Which one am I using?" card: press the button, move or click the device, and the matching card is marked.
/// KDE does not say which physical device just moved, and reading raw input needs administrator rights, so this asks
/// for the password once and listens with `libinput debug-events` for a few seconds.
fn identify_group(groups: Vec<(String, adw::PreferencesGroup, Option<&'static str>)>) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    let row = adw::ActionRow::new();
    row.set_title("Which one am I using?");
    row.set_subtitle("Press Identify, then move or click the device you want to find. Asks for your password once.");
    row.add_prefix(&gtk4::Image::from_icon_name("input-mouse-symbolic"));
    let btn = gtk4::Button::with_label("Identify");
    btn.set_valign(gtk4::Align::Center);
    btn.add_css_class("suggested-action");
    row.add_suffix(&btn);
    g.add(&row);

    let groups = std::rc::Rc::new(groups);
    btn.connect_clicked(move |b| {
        b.set_sensitive(false);
        row.set_subtitle("Move or click the device you want to find… (8 seconds)");
        let (tx, rx) = std::sync::mpsc::channel::<Result<Vec<String>, String>>();
        std::thread::spawn(move || {
            // Say so up front instead of asking for a password to run something that is not there.
            let tool = ["/usr/bin/libinput", "/usr/sbin/libinput"].iter().any(|p| std::path::Path::new(p).exists());
            let out = if tool {
                Some(std::process::Command::new("pkexec").args(["stdbuf", "-oL", "timeout", "8", "libinput", "debug-events"]).output())
            } else {
                None
            };
            let r = match out {
                None => Err("This needs the libinput-tools package. Update Zohara, then try again.".to_string()),
                // `timeout` ends the tool on purpose, so its exit status says nothing; the text does.
                Some(Ok(o)) if !o.stdout.is_empty() => Ok(parse_libinput_events(&String::from_utf8_lossy(&o.stdout))),
                // pkexec: 126 = not authorized or the prompt was dismissed; 127 = it could not run the command.
                Some(Ok(o)) if o.status.code() == Some(126) => Err("The password prompt was cancelled.".to_string()),
                Some(Ok(_)) => Err("The device check could not start. Try again, or open Troubleshoot.".to_string()),
                Some(Err(e)) => Err(e.to_string()),
            };
            let _ = tx.send(r);
        });
        let (b, row, groups) = (b.clone(), row.clone(), groups.clone());
        glib::timeout_add_local(std::time::Duration::from_millis(250), move || match rx.try_recv() {
            Ok(result) => {
                b.set_sensitive(true);
                let used: Vec<String> = result.as_ref().map(|v| v.clone()).unwrap_or_default();
                for (name, group, base) in groups.iter() {
                    if used.iter().any(|u| u == name) {
                        group.set_description(Some("● In use: you just used this one"));
                        group.add_css_class("input-active");
                    } else {
                        group.set_description(*base);
                        group.remove_css_class("input-active");
                    }
                }
                row.set_subtitle(&match result {
                    Err(e) => e,
                    Ok(v) if v.is_empty() => "Nothing moved. Try again and move or click the device.".to_string(),
                    Ok(v) => format!("You used: {}", v.join(", ")),
                });
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(_) => glib::ControlFlow::Break,
        });
    });
    g
}

/// Devices that were left out, so nothing is lost: each with the reason.
fn hidden_group(hidden: &[Device]) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    let ex = adw::ExpanderRow::new();
    ex.set_use_markup(false);
    ex.set_title(&format!("Other input devices ({})", hidden.len()));
    ex.set_subtitle("Hidden because there is nothing to change on them");
    for d in hidden {
        let r = adw::ActionRow::new();
        r.set_use_markup(false);
        r.set_title(&d.name);
        r.set_subtitle(d.why_hidden());
        r.set_activatable(false);
        ex.add_row(&r);
    }
    g.add(&ex);
    g
}

/// Fills `container` with one settings group per real device, with a way to find out which one is in use.
pub fn populate(container: &gtk4::Box, touchpads: bool) {
    let status = adw::StatusPage::builder().title("Looking for devices…").build();
    status.set_css_classes(&["compact"]);
    container.append(&status);

    let container = container.clone();
    in_background(load_devices, move |res| {
        container.remove(&status);
        let all: Vec<Device> = match res {
            Ok(d) => d.into_iter().filter(|d| if touchpads { d.touchpad } else { d.pointer && !d.touchpad }).collect(),
            Err(_) => {
                let s = adw::StatusPage::builder()
                    .icon_name("dialog-information-symbolic")
                    .title("Device settings unavailable")
                    .description("Per-device settings come from KWin and need the Plasma (Wayland) session.")
                    .build();
                s.set_css_classes(&["compact"]);
                container.append(&s);
                return;
            }
        };
        let (shown, hidden): (Vec<Device>, Vec<Device>) = all.into_iter().partition(|d| d.is_useful());
        if shown.is_empty() {
            let s = adw::StatusPage::builder()
                .icon_name(if touchpads { "input-touchpad-symbolic" } else { "input-mouse-symbolic" })
                .title(if touchpads { "No touchpad found" } else { "No mouse found" })
                .build();
            s.set_css_classes(&["compact"]);
            container.append(&s);
        } else {
            let mut groups = Vec::new();
            let mut cards = Vec::new();
            for d in &shown {
                let g = device_group(d);
                let base = if d.bus.label().is_empty() { None } else { Some(d.bus.label()) };
                groups.push((d.name.clone(), g.clone(), base));
                cards.push(g);
            }
            // Only worth asking "which one?" when there is more than one to tell apart.
            if shown.len() >= 2 {
                container.append(&identify_group(groups));
            }
            for g in cards {
                container.append(&g);
            }
        }
        if !hidden.is_empty() {
            container.append(&hidden_group(&hidden));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn where_a_device_sits_says_how_it_is_connected() {
        assert_eq!(classify_bus("../../devices/pci0000:00/0000:00:14.0/usb1/1-2/1-2:1.0/0003:046D:C52B.0003/input/input12/event5"), Bus::Usb);
        assert_eq!(classify_bus("../../devices/virtual/input/input7/event7"), Bus::Virtual);
        assert_eq!(classify_bus("../../devices/platform/i8042/serio1/input/input5/event4"), Bus::BuiltIn);
        assert_eq!(classify_bus("../../devices/pci0000:00/0000:00:15.1/i2c_designware.1/i2c-9/i2c-ELAN0412:00/0018:04F3:3140.0001/input/input8/event6"), Bus::BuiltIn);
        assert_eq!(classify_bus("../../devices/virtual/misc/uhid/0005:046D:B023.0006/input/input30/event12"), Bus::Virtual);
        assert_eq!(classify_bus("../../devices/pci0000:00/0000:00:14.0/usb1/1-10/1-10:1.0/bluetooth/hci0/hci0:3/0005:046D:B023.0006/input/input30/event12"), Bus::Bluetooth);
    }

    #[test]
    fn a_device_with_nothing_to_change_is_not_shown() {
        let consumer_control = Device { pointer: true, bus: Bus::Usb, ..Default::default() };
        assert!(!consumer_control.is_useful());
        let mouse = Device { pointer: true, bus: Bus::Usb, accel: Some(0.0), ..Default::default() };
        assert!(mouse.is_useful());
        let virtual_mouse = Device { pointer: true, bus: Bus::Virtual, accel: Some(0.0), ..Default::default() };
        assert!(!virtual_mouse.is_useful());
        assert_eq!(virtual_mouse.why_hidden(), "Virtual device made by software");
        assert_eq!(consumer_control.why_hidden(), "Nothing to change on this one");
    }

    #[test]
    fn libinput_output_names_the_device_that_moved() {
        let text = "\
-event3   DEVICE_ADDED     Power Button                      seat0 default group1 cap:k
-event5   DEVICE_ADDED     Logitech USB Receiver             seat0 default group2 cap:pk left scroll-nat
-event7   DEVICE_ADDED     SynPS/2 Synaptics TouchPad        seat0 default group3 cap:pt  size 100x60mm tap
 event5   POINTER_MOTION    +1.19s	  0.00/  1.00 ( +0.00/ +1.00)
 event5   POINTER_BUTTON    +2.30s	BTN_LEFT (272) pressed, seat count: 1
 event3   KEYBOARD_KEY      +3.00s	KEY_POWER (116) pressed
";
        assert_eq!(parse_libinput_events(text), vec!["Logitech USB Receiver".to_string()]);
        assert!(parse_libinput_events("").is_empty());
        // Two devices used: both are reported.
        let both = format!("{text} event7   POINTER_MOTION    +4.0s	  1.00/  1.00 ( +1.00/ +1.00)\n");
        assert_eq!(parse_libinput_events(&both).len(), 2);
    }
}
