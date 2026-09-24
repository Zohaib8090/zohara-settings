//! Per-device mouse and touchpad settings through KWin's InputDevice D-Bus
//! API (Plasma Wayland). KWin applies each change to the libinput device
//! immediately and saves it to kcminputrc itself, so nothing else is needed.

use crate::backend::worker::{block_on, in_background};
use adw::prelude::*;
use gtk4::prelude::*;
use libadwaita as adw;

const KWIN: &str = "org.kde.KWin";
const DEVICE_IFACE: &str = "org.kde.KWin.InputDevice";

#[derive(Clone, Default)]
pub struct Device {
    sys: String,
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

    Ok(Device {
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

fn device_group(dev: &Device) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::new();
    g.set_title(&glib::markup_escape_text(&dev.name));

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

/// Fills `container` with one settings group per matching device.
pub fn populate(container: &gtk4::Box, touchpads: bool) {
    let status = adw::StatusPage::builder().title("Looking for devices…").build();
    status.set_css_classes(&["compact"]);
    container.append(&status);

    let container = container.clone();
    in_background(load_devices, move |res| {
        container.remove(&status);
        let devices: Vec<Device> = match res {
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
        if devices.is_empty() {
            let s = adw::StatusPage::builder()
                .icon_name(if touchpads { "input-touchpad-symbolic" } else { "input-mouse-symbolic" })
                .title(if touchpads { "No touchpad found" } else { "No mouse found" })
                .build();
            s.set_css_classes(&["compact"]);
            container.append(&s);
            return;
        }
        for d in &devices {
            container.append(&device_group(d));
        }
    });
}
