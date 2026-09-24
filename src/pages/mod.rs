pub mod home;
pub mod system;
pub mod bluetooth;
pub mod network;
pub mod network_extra;
pub mod speedtest;
pub mod personalization;
pub mod apps;
pub mod offline_maps;
pub mod web_apps;
pub mod accounts;
pub mod time_language;
pub mod gaming;
pub mod accessibility;
pub mod privacy;
pub mod updates;
pub mod display;
pub mod display_layout;
pub mod sound;
pub mod notifications;
pub mod power;
pub mod storage;
pub mod input_devices;
pub mod mouse;
pub mod touchpad;
pub mod keyboard;
pub mod shortcuts;
pub mod default_apps;
pub mod advanced;
pub mod zohara_link;

/// Switch the Settings window to the page with this sidebar label.
pub fn goto(widget: &impl gtk4::prelude::IsA<gtk4::Widget>, label: &str) {
    use gtk4::prelude::*;
    use gtk4::glib::prelude::ToVariant;
    let _ = widget.as_ref().activate_action("win.goto", Some(&label.to_variant()));
}
